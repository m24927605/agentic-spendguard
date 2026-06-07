// `wrapGenerate` / `wrapStream` real implementations — D06 SLICE 4 + SLICE 5.
//
// SLICE 2/3 shipped only the factory + `transformParams` reserve wiring; the
// generate-commit + stream-commit paths were `NotImpl` stubs. SLICE 4 + SLICE 5
// (bundled per the marathon dispatch) replace those stubs with the real
// commit-on-success / release-on-failure paths:
//
//   - `wrapGenerate({ doGenerate, params })` — non-streaming path. Looks up the
//     stash entry written by `transformParams` (keyed by the `params` reference
//     via WeakMap). When present, runs the inner `doGenerate()` inside a
//     try/catch: SUCCESS → `client.commitEstimated({outcomeKind:"SUCCESS"})`
//     with the provider-reported `(promptTokens, completionTokens)` tuple;
//     FAILURE (catch) → `client.commitEstimated({outcomeKind:"FAILURE"})`
//     with the error message threaded onto `actualErrorMessage`, then
//     re-throws. When no stash entry exists (degraded path: `reserve()` failed
//     and SLICE 3 swallowed the error) the call passes through unmodified —
//     same discipline as the SLICE 3 `SidecarUnavailable` branch.
//
//   - `wrapStream({ doStream, params })` — streaming path. Looks up the stash;
//     when absent, passes through. When present, calls inner `doStream()` to
//     get `{ stream, ...rest }`, then wraps `stream` in a `TransformStream`
//     that (a) forwards every part downstream unmodified, (b) accumulates the
//     `usage` payload from the `finish` part as it flows through, and (c) on
//     stream end (`flush()`) emits a SUCCESS commit asynchronously with the
//     final usage tuple. Stream-side errors mirror downstream AND emit a
//     FAILURE commit. A single `terminal` flag guards against a finish/error
//     race so exactly one of SUCCESS / FAILURE fires.
//
// Commit-side failures (e.g. sidecar UNAVAILABLE post-finish) do NOT corrupt
// the stream — review-standards.md §3.4 / design.md §6 race-guard semantics.
// Sidecar TTL reconciles via the audit chain.
//
// Token-usage extraction handles both AI SDK v4 canonical camelCase
// (`{promptTokens, completionTokens}`) AND snake_case OpenAI-passthrough
// (`{prompt_tokens, completion_tokens}`) shapes — matches the LangChain
// adapter's `extractTokenUsage` discipline. See `tests/middleware.test.ts`
// for the cross-shape parity cases.
//
// Design references:
//   - docs/specs/coverage/D06_vercel_ai_sdk/design.md §5 (architecture),
//     §6 (streaming semantics), §8 locked decisions #3 / #8.
//   - docs/specs/coverage/D06_vercel_ai_sdk/implementation.md §3 (core types),
//     §4 (streaming instrumentation), §8 (commit + rollback paths).
//   - docs/specs/coverage/D06_vercel_ai_sdk/review-standards.md §2 (v1
//     conformance), §3 (streaming correctness), §7 (error handling).

import type {
  CommitEstimatedRequest,
  PricingFreeze,
  SpendGuardClient,
  UnitRef,
} from "@spendguard/sdk";
import type { LanguageModelV1, LanguageModelV1StreamPart } from "ai";

// ── StashEntry contract ────────────────────────────────────────────────────
//
// Mirrors the `StashEntry` shape `middleware.ts` writes. Re-declared here as a
// local interface (rather than imported) so the two files have an explicit
// contract — middleware.ts owns the WeakMap, wrapper.ts owns the consumer
// side. Both shapes MUST stay in sync; the local interface keeps the linkage
// explicit at the type level.

export interface StashEntry {
  decisionId: string;
  reservationId: string;
  runId: string;
  idempotencyKey: string;
}

// ── Shared constants (mirror middleware.ts) ────────────────────────────────
//
// Picked verbatim from middleware.ts so the commit-side request fields match
// what `transformParams` advertised at reserve time. Drift here breaks the
// substrate's idempotency-key reconciliation; review-standards §4.

const STEP_ID_LLM_CALL = "llm_call";
const DEFAULT_UNIT: UnitRef = { unit: "USD_MICROS", denomination: 1 };
const EMPTY_PRICING: PricingFreeze = {
  pricingVersion: "",
  pricingHash: new Uint8Array(0),
};

// ── Public entry points ────────────────────────────────────────────────────

/**
 * Stash lookup function injected by `middleware.ts`. Kept as a thin function
 * pointer so wrapper.ts does NOT take a direct dependency on middleware.ts's
 * module-level WeakMap (would create an import cycle).
 */
export type StashLookup = (params: unknown) => StashEntry | undefined;

/**
 * SLICE 4 real implementation of `wrapGenerate`.
 *
 * Looks up the stash entry written by `transformParams`. When absent, calls
 * the inner `doGenerate()` straight through (degraded path: SLICE 3 swallowed
 * a SidecarUnavailable / transport error and no stash was set). When present,
 * wraps the inner call in try/catch:
 *
 *   - SUCCESS: extracts `(promptTokens, completionTokens)` from `result.usage`
 *     (accepting both camelCase + snake_case shapes for cross-provider
 *     parity), then emits a `client.commitEstimated({outcomeKind:"SUCCESS"})`
 *     with the actuals on the wire-typed fields. Returns the inner result
 *     unchanged.
 *   - FAILURE (catch): emits a `client.commitEstimated({outcomeKind:"FAILURE"})`
 *     with the error message threaded onto `actualErrorMessage`, then
 *     re-throws the original error so the AI SDK caller sees the typed
 *     provider error.
 */
export function makeWrapGenerate(
  client: SpendGuardClient,
  lookupStash: StashLookup,
): NonNullable<Parameters<typeof identityMiddleware>[0]> {
  return async ({
    doGenerate,
    params,
  }: {
    doGenerate: () => ReturnType<LanguageModelV1["doGenerate"]>;
    doStream: () => ReturnType<LanguageModelV1["doStream"]>;
    params: unknown;
    model: LanguageModelV1;
  }): Promise<Awaited<ReturnType<LanguageModelV1["doGenerate"]>>> => {
    const entry = lookupStash(params);

    // Degraded path — reserve() failed in transformParams and was swallowed
    // per the "operational degradation, not enforcement" policy. The LLM
    // call MUST still fire; no commit is emitted because no reservation
    // exists to settle.
    if (entry === undefined) {
      return doGenerate();
    }

    try {
      const result = await doGenerate();
      const usage = extractUsageFromGenerate(result);
      await safeCommit(client, entry, {
        outcomeKind: "SUCCESS",
        outcome: "SUCCESS",
        actualInputTokensWire: String(usage.promptTokens),
        actualOutputTokensWire: String(usage.completionTokens),
      });
      return result;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      await safeCommit(client, entry, {
        outcomeKind: "FAILURE",
        outcome: "PROVIDER_ERROR",
        actualErrorMessage: message,
      });
      throw err;
    }
  };
}

/**
 * SLICE 5 real implementation of `wrapStream`.
 *
 * Looks up the stash entry written by `transformParams`. When absent, calls
 * the inner `doStream()` straight through. When present, calls `doStream()`
 * to obtain `{ stream, ...rest }` and replaces `stream` with a wrapped
 * `ReadableStream` that:
 *
 *   1. Forwards every `LanguageModelV1StreamPart` downstream unmodified
 *      (consumers see the original stream, byte-for-byte).
 *   2. Watches each part for the `finish` event and snapshots its `usage`
 *      payload (`{promptTokens, completionTokens}`). Multiple `finish` parts
 *      (rare; should not happen per AI SDK contract) — last one wins.
 *   3. On the stream's terminal `flush()` (consumer drained successfully),
 *      asynchronously emits a SUCCESS commit with the captured usage tuple.
 *   4. On a stream-side `error` part OR an upstream throw, emits a FAILURE
 *      commit with the error message and propagates the error downstream.
 *   5. Single `terminal` boolean ensures exactly-once commit emission across
 *      the finish / error race window (design.md §6 + review-standards §3.5).
 *
 * Commit-side failure NEVER corrupts the stream — `safeCommit` swallows the
 * commit RPC's own errors and falls back to a `console.warn`. The substrate
 * sidecar's TTL reconciler closes out any orphaned reservation. Review-
 * standards §3.4 covers the rationale.
 */
export function makeWrapStream(
  client: SpendGuardClient,
  lookupStash: StashLookup,
): NonNullable<Parameters<typeof identityMiddleware>[1]> {
  return async ({
    doStream,
    params,
  }: {
    doGenerate: () => ReturnType<LanguageModelV1["doGenerate"]>;
    doStream: () => ReturnType<LanguageModelV1["doStream"]>;
    params: unknown;
    model: LanguageModelV1;
  }): Promise<Awaited<ReturnType<LanguageModelV1["doStream"]>>> => {
    const entry = lookupStash(params);
    const inner = await doStream();

    // Degraded path — passthrough without instrumentation.
    if (entry === undefined) {
      return inner;
    }

    const instrumented = instrumentStream(inner.stream, async (kind, ctx) => {
      if (kind === "finish") {
        await safeCommit(client, entry, {
          outcomeKind: "SUCCESS",
          outcome: "SUCCESS",
          actualInputTokensWire: String(ctx.promptTokens),
          actualOutputTokensWire: String(ctx.completionTokens),
        });
      } else {
        await safeCommit(client, entry, {
          outcomeKind: "FAILURE",
          outcome: "PROVIDER_ERROR",
          actualErrorMessage: ctx.errorMessage,
        });
      }
    });

    return { ...inner, stream: instrumented };
  };
}

// ── Stream instrumentation ─────────────────────────────────────────────────

type StreamCallback =
  | { kind: "finish"; promptTokens: number; completionTokens: number }
  | { kind: "error"; errorMessage: string };

type StreamCallbackHandler = (
  kind: StreamCallback["kind"],
  ctx: { promptTokens: number; completionTokens: number; errorMessage: string },
) => Promise<void>;

/**
 * Wrap `inner` with a `TransformStream` that forwards parts unmodified,
 * accumulates `finish`-part usage, and emits the SUCCESS / FAILURE commit
 * exactly once via `onTerminal`.
 *
 * The `terminal` flag race-guard ensures exactly one of `finish` / `error`
 * fires even when both happen near-simultaneously. Design.md §6.
 */
function instrumentStream(
  inner: ReadableStream<LanguageModelV1StreamPart>,
  onTerminal: StreamCallbackHandler,
): ReadableStream<LanguageModelV1StreamPart> {
  let terminal = false;
  let lastPromptTokens = 0;
  let lastCompletionTokens = 0;

  const transform = new TransformStream<LanguageModelV1StreamPart, LanguageModelV1StreamPart>({
    transform(part, controller) {
      // Always forward the part downstream FIRST so the consumer's stream
      // shape is byte-for-byte identical to the inner stream.
      controller.enqueue(part);

      // Accumulate usage from `finish` parts. `error` parts trigger the
      // FAILURE path via the upstream-error catch in `start()`.
      if (part.type === "finish") {
        const usage = extractUsageFromStreamPart(part);
        if (usage !== undefined) {
          lastPromptTokens = usage.promptTokens;
          lastCompletionTokens = usage.completionTokens;
        }
      } else if (part.type === "error") {
        // Stream-side error parts ALSO terminate the stash settlement.
        if (!terminal) {
          terminal = true;
          const message = part.error instanceof Error ? part.error.message : String(part.error);
          // Fire-and-forget — commit failure must not block stream forwarding.
          void onTerminal("error", {
            promptTokens: 0,
            completionTokens: 0,
            errorMessage: message,
          }).catch((commitErr) => {
            console.warn(
              `[spendguard:vercel-ai] stream FAILURE commit threw: ${
                commitErr instanceof Error ? commitErr.message : String(commitErr)
              }`,
            );
          });
        }
      }
    },
    async flush() {
      // Normal stream-end path. Race guard against a pre-fired `error` part.
      if (terminal) return;
      terminal = true;
      try {
        await onTerminal("finish", {
          promptTokens: lastPromptTokens,
          completionTokens: lastCompletionTokens,
          errorMessage: "",
        });
      } catch (commitErr) {
        // commit-side failure must NOT corrupt the stream — review-standards
        // §3.4. Sidecar TTL reconciles. Log + drop.
        console.warn(
          `[spendguard:vercel-ai] stream SUCCESS commit threw: ${
            commitErr instanceof Error ? commitErr.message : String(commitErr)
          }`,
        );
      }
    },
  });

  const piped = inner.pipeThrough(transform);

  // Outer ReadableStream so we can mirror inner errors to the consumer AND
  // emit a FAILURE commit when the inner pipeline throws (vs an `error`
  // part flowing through normally).
  return new ReadableStream<LanguageModelV1StreamPart>({
    async start(controller) {
      const reader = piped.getReader();
      try {
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          controller.enqueue(value);
        }
        controller.close();
      } catch (err) {
        if (!terminal) {
          terminal = true;
          const message = err instanceof Error ? err.message : String(err);
          try {
            await onTerminal("error", {
              promptTokens: 0,
              completionTokens: 0,
              errorMessage: message,
            });
          } catch (commitErr) {
            console.warn(
              `[spendguard:vercel-ai] stream FAILURE commit threw: ${
                commitErr instanceof Error ? commitErr.message : String(commitErr)
              }`,
            );
          }
        }
        controller.error(err);
      } finally {
        reader.releaseLock();
      }
    },
    async cancel(reason) {
      // Consumer-initiated cancel → release the reservation as FAILURE.
      if (!terminal) {
        terminal = true;
        const message =
          reason instanceof Error
            ? reason.message
            : reason !== undefined
              ? String(reason)
              : "stream cancelled";
        try {
          await onTerminal("error", {
            promptTokens: 0,
            completionTokens: 0,
            errorMessage: message,
          });
        } catch (commitErr) {
          console.warn(
            `[spendguard:vercel-ai] stream cancel FAILURE commit threw: ${
              commitErr instanceof Error ? commitErr.message : String(commitErr)
            }`,
          );
        }
      }
    },
  });
}

// ── Commit dispatch ────────────────────────────────────────────────────────

/**
 * `client.commitEstimated(...)` wrapper that builds the request shape mirror
 * of the LangChain adapter's success/failure path and swallows commit-RPC
 * failures so commit-side errors NEVER bubble back to the consumer.
 *
 * The LLM call result has already been delivered (success path) or the
 * original provider error has already been re-thrown (failure path) — a
 * commit-side throw at this point would corrupt that surface for the
 * consumer with an unrelated error. Sidecar TTL reconciles any orphaned
 * reservation via the audit chain.
 */
async function safeCommit(
  client: SpendGuardClient,
  entry: StashEntry,
  outcome:
    | {
        outcomeKind: "SUCCESS";
        outcome: "SUCCESS";
        actualInputTokensWire: string;
        actualOutputTokensWire: string;
      }
    | {
        outcomeKind: "FAILURE";
        outcome: "PROVIDER_ERROR";
        actualErrorMessage: string;
      },
): Promise<void> {
  const req: CommitEstimatedRequest = {
    runId: entry.runId,
    stepId: STEP_ID_LLM_CALL,
    llmCallId: entry.runId,
    decisionId: entry.decisionId,
    reservationId: entry.reservationId,
    estimatedAmountAtomic: "0",
    unit: DEFAULT_UNIT,
    pricing: EMPTY_PRICING,
    providerEventId: "",
    outcome: outcome.outcome,
    outcomeKind: outcome.outcomeKind,
    ...(outcome.outcomeKind === "SUCCESS"
      ? {
          actualInputTokensWire: outcome.actualInputTokensWire,
          actualOutputTokensWire: outcome.actualOutputTokensWire,
        }
      : { actualErrorMessage: outcome.actualErrorMessage }),
  };

  try {
    await client.commitEstimated(req);
  } catch (commitErr) {
    console.warn(
      `[spendguard:vercel-ai] commitEstimated(${outcome.outcomeKind}) threw for runId=${entry.runId}: ${
        commitErr instanceof Error ? commitErr.message : String(commitErr)
      }`,
    );
  }
}

// ── Usage extraction ───────────────────────────────────────────────────────

interface ExtractedUsage {
  promptTokens: number;
  completionTokens: number;
}

/**
 * Extract `(promptTokens, completionTokens)` from a `doGenerate()` result.
 * Accepts the canonical AI SDK v4 camelCase shape AND the OpenAI-passthrough
 * snake_case shape so the wrapper handles raw provider payloads identically
 * across `@ai-sdk/openai` / `@ai-sdk/anthropic` / future providers.
 */
function extractUsageFromGenerate(result: unknown): ExtractedUsage {
  if (result === null || typeof result !== "object") {
    return { promptTokens: 0, completionTokens: 0 };
  }
  const bag = result as { usage?: unknown };
  return extractUsageFromBag(bag.usage);
}

/**
 * Extract usage from a `LanguageModelV1StreamPart` of kind `finish`. The
 * official v1 shape carries `usage: {promptTokens, completionTokens}` —
 * extracted via the same shared accessor so snake_case provider relays are
 * also accepted defensively.
 */
function extractUsageFromStreamPart(
  part: Extract<LanguageModelV1StreamPart, { type: "finish" }>,
): ExtractedUsage | undefined {
  const usage = part.usage;
  if (usage === undefined || usage === null) return undefined;
  return extractUsageFromBag(usage);
}

function extractUsageFromBag(bag: unknown): ExtractedUsage {
  if (bag === null || typeof bag !== "object") {
    return { promptTokens: 0, completionTokens: 0 };
  }
  const obj = bag as Record<string, unknown>;
  const prompt = readNumeric(obj, ["promptTokens", "prompt_tokens"]);
  const completion = readNumeric(obj, ["completionTokens", "completion_tokens"]);
  return {
    promptTokens: prompt ?? 0,
    completionTokens: completion ?? 0,
  };
}

function readNumeric(bag: Record<string, unknown>, keys: readonly string[]): number | undefined {
  for (const key of keys) {
    const value = bag[key];
    if (typeof value === "number" && Number.isFinite(value)) {
      return value;
    }
  }
  return undefined;
}

// ── Type carrier — keeps wrapper.ts importable without an import cycle ─────
//
// `makeWrapGenerate` / `makeWrapStream` return values are typed against the
// `LanguageModelV1Middleware` hook shape. `identityMiddleware` is a no-op
// helper whose ONLY purpose is to give the return-type annotation a concrete
// origin (`NonNullable<Parameters<...>[0|1]>`) without re-importing `ai`'s
// `LanguageModelV1Middleware` type here. Keeps the wrapper file self-
// contained and lets middleware.ts remain the single point of contact with
// the `ai` peer-dep's middleware-type surface.

function identityMiddleware(
  _wrapGenerate?: (args: {
    doGenerate: () => ReturnType<LanguageModelV1["doGenerate"]>;
    doStream: () => ReturnType<LanguageModelV1["doStream"]>;
    params: unknown;
    model: LanguageModelV1;
  }) => Promise<Awaited<ReturnType<LanguageModelV1["doGenerate"]>>>,
  _wrapStream?: (args: {
    doGenerate: () => ReturnType<LanguageModelV1["doGenerate"]>;
    doStream: () => ReturnType<LanguageModelV1["doStream"]>;
    params: unknown;
    model: LanguageModelV1;
  }) => Promise<Awaited<ReturnType<LanguageModelV1["doStream"]>>>,
): void {
  // intentional no-op — see file-level note on the type carrier.
}

// ── Deprecated SLICE 2/3 stubs ─────────────────────────────────────────────
//
// Kept for backwards compatibility with anything that imported the stub
// directly (none in-tree; the stubs were `@internal`). New code MUST use
// `makeWrapGenerate` / `makeWrapStream`. The stubs themselves now throw the
// same `SpendGuardMiddlewareNotImplemented` error as before so a stale
// downstream consumer who imported the symbol still gets a clear signal.

/**
 * Error thrown by the SLICE 2/3 stubs.
 *
 * @deprecated SLICE 4 + SLICE 5 ship the real `makeWrapGenerate` /
 * `makeWrapStream` implementations; this error class is preserved only so
 * stale downstream imports of `wrapGenerateStub` / `wrapStreamStub` still
 * surface a typed error. New code MUST use the `makeWrap*` factories.
 *
 * @internal
 */
export class SpendGuardMiddlewareNotImplemented extends Error {
  constructor(hook: "wrapGenerate" | "wrapStream") {
    super(
      `@spendguard/vercel-ai: ${hook} stub invoked; SLICE 4/5 replaced these with makeWrap* factories. If you see this error in production, your build is pinned to an old import.`,
    );
    this.name = "SpendGuardMiddlewareNotImplemented";
  }
}

/**
 * @deprecated SLICE 4 — use `makeWrapGenerate(client, lookupStash)` instead.
 * @internal
 */
export async function wrapGenerateStub(): Promise<never> {
  throw new SpendGuardMiddlewareNotImplemented("wrapGenerate");
}

/**
 * @deprecated SLICE 5 — use `makeWrapStream(client, lookupStash)` instead.
 * @internal
 */
export async function wrapStreamStub(): Promise<never> {
  throw new SpendGuardMiddlewareNotImplemented("wrapStream");
}
