// `createSpendGuardMiddleware` — the public Vercel AI SDK middleware factory.
//
// SLICE 2 shipped the skeleton + WeakMap stash + factory validation. SLICE 3
// wires `transformParams` against the substrate's `reserve()` RPC:
//
//   - Each `transformParams({ params })` call derives a stable
//     `(runId, idempotencyKey)` pair from the `params` reference (via
//     `./ids.ts`), projects a coarse pre-call `BudgetClaim` from the
//     flattened prompt text, and dispatches `client.reserve(...)`. On
//     success the resulting `(decisionId, reservationId)` pair is stashed
//     in a module-level `WeakMap<LanguageModelV1CallOptions, StashEntry>`
//     keyed by the params reference itself — review-standards.md §8.1.
//   - On `DecisionDenied` (or subclass — `DecisionStopped`,
//     `ApprovalRequired`) the error rethrows so the SDK caller halts before
//     `doGenerate()` fires. On `SidecarUnavailable` (or any other substrate
//     error) the call passes through without a stash — the LLM call
//     proceeds without a budget gate, matching D04's
//     "operational degradation, not enforcement" stance.
//   - The returned object satisfies AI SDK v4's `LanguageModelV1Middleware`
//     shape exactly (`transformParams` + `wrapGenerate` + `wrapStream` +
//     `middlewareVersion: "v1"`). SLICE 4/5 will replace the
//     `wrapGenerate` / `wrapStream` stubs with the real commit / release
//     paths; SLICE 2/3 keeps the stubs (`./wrapper.ts`) so a caller who
//     accidentally drives a SLICE-2/3 build with `generateText(...)` gets
//     a pointed error rather than silent success.
//
// Design references:
//   - docs/specs/coverage/D06_vercel_ai_sdk/design.md §4 (public surface),
//     §5 (architecture), §8 (locked design decisions #2 / #4)
//   - docs/specs/coverage/D06_vercel_ai_sdk/implementation.md §3 (core
//     types) — D06's spec was written against AI SDK v5; the installed
//     `ai@^4.0.0` peer wires v1 of the middleware type, so this slice
//     targets `LanguageModelV1Middleware`. The v5 migration is a follow-up.
//   - docs/specs/coverage/D06_vercel_ai_sdk/review-standards.md §1 / §4 /
//     §7 / §8 (locked surface / idempotency / error propagation / WeakMap
//     stash discipline).
//
// Mirrors D04 SLICE 2/3 discipline: minimal locked options surface
// (`client` + `tenantId` + `budgetId?`) first; additive optional fields
// (`unit`, `pricing`, `claimEstimator`, `providerEventIdExtractor`, …)
// land in SLICE 4+ when the commit / release paths actually consume them.

import {
  type BudgetClaim,
  DecisionDenied,
  type DecisionOutcome,
  type ReserveRequest,
  type UnitRef,
  deriveUuidFromSignature,
} from "@spendguard/sdk";
import type {
  LanguageModelV1CallOptions,
  LanguageModelV1Middleware,
  LanguageModelV1Prompt,
} from "ai";
import { deriveIdempotencyKey } from "./ids.js";
import type { SpendGuardMiddlewareOptions } from "./options.js";
import { makeWrapGenerate, makeWrapStream } from "./wrapper.js";

// ── Internal stash shape ──────────────────────────────────────────────────

/**
 * Per-call correlation record. Written by `transformParams`, consumed by
 * `wrapGenerate` / `wrapStream` (SLICE 4 / SLICE 5). Keyed by the
 * `LanguageModelV1CallOptions` reference itself — review-standards §8
 * "WeakMap stash discipline" P1.
 *
 * Stored on a `WeakMap<LanguageModelV1CallOptions, StashEntry>` so the GC
 * collects the entry the moment the AI SDK drops the params reference;
 * no manual cleanup, no leak (review-standards §8.4).
 */
interface StashEntry {
  decisionId: string;
  reservationId: string;
  /**
   * Per-call run id (UUID) — caller-stable across SDK retries because it
   * is derived from the params reference content via
   * `deriveUuidFromSignature`. Forwarded to SLICE 4/5's commit / release
   * paths.
   */
  runId: string;
  /**
   * Idempotency key the substrate saw on `reserve()`. Cached so SLICE 4/5
   * can pass it onto the commit / release RPCs without re-deriving.
   */
  idempotencyKey: string;
}

// ── Defaults the SLICE 2/3 options surface deliberately omits ────────────
//
// Mirror D04 SLICE 3: pick sensible defaults for fields the LOCKED options
// surface does not yet expose so the SLICE 3 wiring is end-to-end testable
// without expanding the public type.

/** Default route label surfaced on `ReserveRequest.route`. */
const DEFAULT_ROUTE = "vercel-ai-llm";

/** Default budget unit — micro-dollars, the substrate's canonical money unit. */
const DEFAULT_UNIT: UnitRef = { unit: "USD_MICROS", denomination: 1 };

/**
 * Constant `stepId` for the SLICE 3 LLM-call boundary. Matched against the
 * value baked into `./ids.ts:deriveIdempotencyKey`, so the idempotency key
 * the adapter ships matches what the substrate would re-derive from the
 * canonical fields.
 */
const STEP_ID_LLM_CALL = "llm_call";

/**
 * Rough character → token ratio for projecting a pre-call budget claim
 * from the flattened prompt text. Mirrors D04 SLICE 3's heuristic exactly;
 * the substrate cares that the claim shape is well-formed, the
 * authoritative spend lands on the POST commit (SLICE 4/5).
 */
const CHARS_PER_TOKEN_HEURISTIC = 4;

/** Default micros projected per estimated token at PRE time. */
const DEFAULT_MICROS_PER_TOKEN = 1_000n;

/** Scope tag used when deriving the per-call runId from params content. */
const RUN_ID_SCOPE = "vercel_ai_run_id";

// ── Module-level WeakMap stash ────────────────────────────────────────────
//
// Keyed by `LanguageModelV1CallOptions` reference (params object). The AI
// SDK v4 retry loop re-enters the middleware with the SAME params reference
// across retries, so a WeakMap survives the retry window without us
// holding the reference ourselves (review-standards §8 + design.md
// §8 locked decision #4).
//
// Note: deliberately module-level (not per-factory-call) so the stash is
// shared across multiple middleware instances built in the same process —
// no double-entry risk because the WeakMap is keyed by reference identity,
// and `transformParams` is the sole writer.
const STASH = new WeakMap<LanguageModelV1CallOptions, StashEntry>();

/**
 * Construct a Vercel AI SDK middleware that enforces SpendGuard budget
 * guardrails on every wrapped model call.
 *
 * Compose via `wrapLanguageModel({ model, middleware })`. Every
 * `generateText` / `streamText` invocation flows through:
 *   1. `transformParams` → `client.reserve(LLM_CALL_PRE)` (this slice).
 *   2. `wrapGenerate` → `client.commitEstimated(SUCCESS)` / `release` on
 *      failure (SLICE 4).
 *   3. `wrapStream` → TransformStream-based commit-after-finish (SLICE 5).
 *
 * SLICE 2/3 ships steps (1) only. `wrapGenerate` / `wrapStream` throw a
 * clear "SLICE N not implemented" signal so a consumer who calls into a
 * SLICE-2/3 build of the package gets a pointed error instead of silent
 * skip.
 *
 * @param opts Locked options surface. The minimum required fields are
 *             `client` (a configured `SpendGuardClient`) and `tenantId`
 *             (the tenant the call bills against). `budgetId` is optional
 *             and overrides the default tenant-scoped budget routing.
 *
 * @example
 * ```ts
 * import { createSpendGuardMiddleware } from "@spendguard/vercel-ai";
 * import { wrapLanguageModel, generateText } from "ai";
 * import { openai } from "@ai-sdk/openai";
 *
 * const client = new SpendGuardClient({ ... });
 * await client.connect();
 * await client.handshake();
 *
 * const middleware = createSpendGuardMiddleware({
 *   client,
 *   tenantId: "tenant-prod",
 * });
 * const model = wrapLanguageModel({
 *   model: openai("gpt-4o-mini"),
 *   middleware,
 * });
 * const { text } = await generateText({ model, prompt: "Hello" });
 * ```
 *
 * @throws DecisionDenied (and `DecisionStopped` / `ApprovalRequired`
 *   subclasses) from `transformParams` when the substrate denies the
 *   reserve. The AI SDK caller sees the typed error directly.
 */
export function createSpendGuardMiddleware(
  opts: SpendGuardMiddlewareOptions,
): LanguageModelV1Middleware {
  validateOpts(opts);

  return {
    middlewareVersion: "v1",

    transformParams: async ({ params }) => {
      const runId = deriveRunId(params, opts.tenantId);
      const idempotencyKey = deriveIdempotencyKey({
        tenantId: opts.tenantId,
        runId,
      });
      const req: ReserveRequest = {
        trigger: "LLM_CALL_PRE",
        runId,
        stepId: STEP_ID_LLM_CALL,
        llmCallId: runId,
        decisionId: runId,
        route: DEFAULT_ROUTE,
        projectedClaims: [projectClaim(params, opts)],
        idempotencyKey,
      };

      let outcome: DecisionOutcome;
      try {
        outcome = await opts.client.reserve(req);
      } catch (err) {
        // `DecisionDenied` (and `DecisionStopped` / `ApprovalRequired`
        // subclasses) MUST propagate so the SDK caller halts before
        // `doGenerate()` fires — review-standards §7.1 / §7.2.
        if (err instanceof DecisionDenied) {
          throw err;
        }
        // Anything else — `SidecarUnavailable`, transport hiccups, ack
        // rejections — is operational. Log + return params unchanged; do
        // NOT block the LLM call. No stash entry is set, so the matching
        // SLICE-4/5 commit / release paths will no-op (warn) — same
        // discipline as D04 SLICE 3.
        const reason = err instanceof Error ? err.message : String(err);
        console.warn(
          `[spendguard:vercel-ai] reserve() failed for runId=${runId}; ` +
            `LLM call proceeds without budget gate (${reason})`,
        );
        return params;
      }

      STASH.set(params, {
        decisionId: outcome.decisionId,
        reservationId: outcome.reservationIds[0] ?? "",
        runId,
        idempotencyKey,
      });
      return params;
    },

    // SLICE 4 + SLICE 5 wire the real commit / release paths via the stash
    // lookup pointer (avoids an import cycle with `./wrapper.js`). The
    // factories build hook callbacks typed against AI SDK v4's
    // `LanguageModelV1Middleware` shape.
    wrapGenerate: makeWrapGenerate(opts.client, (params) =>
      STASH.get(params as LanguageModelV1CallOptions),
    ),
    wrapStream: makeWrapStream(opts.client, (params) =>
      STASH.get(params as LanguageModelV1CallOptions),
    ),
  };
}

// ── Internal helpers ──────────────────────────────────────────────────────

function validateOpts(opts: SpendGuardMiddlewareOptions): void {
  if (opts === null || typeof opts !== "object") {
    throw new TypeError("createSpendGuardMiddleware: opts must be an object");
  }
  if (!opts.client) {
    throw new TypeError("createSpendGuardMiddleware: opts.client is required");
  }
  if (typeof opts.tenantId !== "string" || opts.tenantId.length === 0) {
    throw new TypeError("createSpendGuardMiddleware: opts.tenantId is required (non-empty string)");
  }
}

/**
 * Derive a stable per-call runId UUID from the params reference.
 *
 * The AI SDK does NOT mint a public-surface run id of its own — middlewares
 * must derive one. Hashing the flattened prompt text + tenant gives us:
 *
 *   1. Determinism across retries: the SDK retry loop calls
 *      `transformParams` with the SAME params reference + same content,
 *      so the runId stays stable and the substrate idempotency cache
 *      collapses the duplicate reserves.
 *   2. Determinism across processes: same prompt + same tenant → same
 *      runId regardless of node restart.
 *   3. Cross-call isolation: two distinct prompts → distinct runIds.
 *
 * Trade-off: two distinct calls with byte-identical prompts share a runId.
 * That is the same behaviour the Python `pydantic_ai.py::_derive_call_identity`
 * helper exhibits (design.md §5) and is what the substrate idempotency
 * cache is designed for — if a caller wants to force fresh ids they can
 * salt their prompts.
 */
function deriveRunId(params: LanguageModelV1CallOptions, tenantId: string): string {
  const promptText = flattenPromptText(params.prompt);
  // Signature deliberately threads through tenantId + a fixed envelope
  // tag so two tenants on the same node never share a runId, and the
  // signature can be extended in later slices without rotating cached ids.
  const signature = `v1|${tenantId}|${promptText}`;
  return deriveUuidFromSignature(signature, { scope: RUN_ID_SCOPE });
}

/**
 * Flatten the AI SDK v4 prompt array to a single deterministic text blob.
 * System messages stringify their content; user / assistant messages
 * walk their parts and concatenate text parts only — image / tool-call /
 * reasoning parts are dropped from the heuristic for SLICE 3.
 *
 * Stable serialisation matters: the runId derivation downstream depends
 * on byte-identical output for byte-identical input.
 */
function flattenPromptText(prompt: LanguageModelV1Prompt): string {
  const out: string[] = [];
  for (const msg of prompt) {
    if (msg.role === "system") {
      out.push(msg.content);
      continue;
    }
    if (msg.role === "tool") {
      // Tool messages carry `Array<ToolResultPart>` — skipped at SLICE 3
      // (the heuristic is "prompt text"; tool results land in SLICE 4 if
      // pricing wants them).
      continue;
    }
    // user | assistant: walk parts, append text-typed parts only.
    for (const part of msg.content) {
      if (part.type === "text") {
        out.push(part.text);
      }
    }
  }
  return out.join("\n");
}

/**
 * Project a coarse pre-call `BudgetClaim` from the flattened prompt text.
 * Mirrors D04 SLICE 3 / SLICE 5 heuristic exactly; the substrate cares
 * only that the claim shape is well-formed at PRE time. SLICE 4/5 supply
 * the authoritative provider-reported numbers on the success commit.
 */
function projectClaim(
  params: LanguageModelV1CallOptions,
  opts: SpendGuardMiddlewareOptions,
): BudgetClaim {
  const totalChars = flattenPromptText(params.prompt).length;
  const estimatedTokens = BigInt(Math.max(1, Math.ceil(totalChars / CHARS_PER_TOKEN_HEURISTIC)));
  const amountMicros = estimatedTokens * DEFAULT_MICROS_PER_TOKEN;
  // HARDEN_D05_UR — thread caller-supplied unitId onto the wire UnitRef.
  // Omitted unitId keeps the pre-HARDEN_D05_UR wire shape (substrate
  // `mapUnitRef` coerces to "").
  const unit: UnitRef = opts.unitId ? { ...DEFAULT_UNIT, unitId: opts.unitId } : DEFAULT_UNIT;
  return {
    scopeId: opts.budgetId ?? opts.tenantId,
    amountAtomic: amountMicros.toString(),
    unit,
  };
}

/**
 * Test-only escape hatch: expose the module-level WeakMap so the SLICE 3
 * test suite can assert stash-write behaviour without poking module
 * internals via TypeScript casts.
 *
 * Marked `@internal` — NOT re-exported from `./index.ts`. Consumers should
 * never reach for this; the stash is an implementation detail.
 *
 * @internal
 */
export function _internalStashFor(params: LanguageModelV1CallOptions): StashEntry | undefined {
  return STASH.get(params);
}
