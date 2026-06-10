// src/processor.ts — SpendGuardProcessor (LOCKED shape design §5; reserve
// hook body per design §6 / implementation.md §3.4).
//
// COV_D38_02 shipped the pre-dispatch reserve path:
//   - `processInputStep`  → RESERVE (fail-closed, design §7 rules 1-3)
//   - `processLLMRequest` → no-op in v1 (design §11.3)
// COV_D38_03 ships the post-dispatch settlement paths (design §6.1 rows
// 3-5):
//   - `processLLMResponse` → SUCCESS COMMIT with usage actuals when exposed
//     (§6.6 LOCKED estimated-amount fallback otherwise)
//   - `processOutputStep`  → backstop COMMIT (at most one commit per
//     reservation — the inflight pop IS the guard)
//   - `processAPIError`    → FAILURE COMMIT (V7 pin below)
//
// ── [VERIFY-AT-IMPL: V1] PINNED (COV_D38_02, @mastra/core 1.41.0) ─────────
// `implements Processor` against the installed package IS the hook-signature
// gate (design §5). Installed shapes recorded:
//   - `Processor<TId, TTripwireMetadata>` REQUIRES `readonly id: TId`
//     (`name` is optional). The §5 class shell shows only `name`; the §5
//     "typechecks against the installed peer" rule therefore mandates `id`
//     as well — both carry the same "spendguard-processor" literal.
//   - `processInputStep?(args: ProcessInputStepArgs): Promise<...>` — args
//     extend ProcessorMessageContext: `messages: MastraDBMessage[]`,
//     `messageList`, `stepNumber` (0-indexed), `steps`, `systemMessages`,
//     `state`, `model`, `abort(reason?, options?): never`, `retryCount`,
//     optional `messageId` / `requestContext` / `writer` / `abortSignal`.
//     Async contract: may return a Promise; returning `undefined` means
//     "no changes" (the processor never mutates the step — TP-21).
//   - `processLLMRequest?(args: ProcessLLMRequestArgs):
//     Promise<ProcessLLMRequestResult> | ProcessLLMRequestResult` —
//     `undefined`/`void` result forwards the prompt unchanged.
//   - `processLLMResponse` / `processOutputStep` arg shapes are consumed by
//     COV_D38_03 (V4/V7).
//
// ── [VERIFY-AT-IMPL: V2] PINNED (COV_D38_02, @mastra/core 1.41.0) ─────────
// Pin selected: **throw directly** (first pre-declared alternative).
// Empirical evidence (tests/failClosed.test.ts TP-10, real Agent + recording
// stub model):
//   - a throw from `processInputStep` halts the step BEFORE the provider
//     call — zero `doGenerate`/`doStream` invocations — and
//     `agent.generate()` REJECTS;
//   - the hook-provided `abort()` (TripWire) is NOT required to halt — and
//     is unusable for D38: a TripWire makes `agent.generate()` RESOLVE with
//     a tripwire result instead of rejecting, which would break the
//     "Agent rejects" observable contract (TP-10).
// Honest limitation recorded: Mastra 1.41.0 runs input processors inside an
// internal workflow whose engine serializes step errors, so the REJECTION
// the consumer sees wraps our typed error's MESSAGE but not the class
// instance (its `cause` chain ends in a serialized POJO). The typed error
// itself is thrown by this adapter at the hook boundary (where `instanceof`
// holds — TP-13/14/15/16 pin it) and its message text is preserved on the
// consumer-facing rejection (TP-10 pins that). This is a property of the
// Mastra runtime, not an adapter degradation — there is still NO fail-open
// branch and the provider call NEVER fires.
//
// ── [VERIFY-AT-IMPL: V5] PINNED (COV_D38_02, @mastra/core 1.41.0) ─────────
// Agent constructor mount key: `inputProcessors` (an `outputProcessors` list
// also exists; there is no unified list). `inputProcessors` drives
// `processInputStep` AND the request-bracket hooks (`processLLMRequest` /
// `processLLMResponse`) — the agent loop builds its per-request
// ProcessorRunner from the `inputProcessors` list. Output-side mounting for
// `processOutputStep` is recorded by COV_D38_03 (V4 pin in usage.ts: the
// backstop `processOutputStep` only fires for processors ALSO mounted via
// `outputProcessors`). Quickstart copies:
//   new Agent({ name, instructions, model, inputProcessors: [guard] })
//
// ── [VERIFY-AT-IMPL: V4] PINNED (COV_D38_03, @mastra/core 1.41.0) ─────────
// Usage fields + streamed-step hook ordering: see the full pin block in
// usage.ts. Summary: flat camelCase `inputTokens`/`outputTokens`
// (LanguageModelUsage = LanguageModelV2Usage & {...}; the loop's
// normalizeUsage() flattens v6/V3 nested usage); exposed DIRECTLY as
// `args.usage` at `processOutputStep` and via the stripped `finish` chunk's
// `payload.output.usage` at `processLLMResponse`. Ordering on streamed
// steps: `processLLMResponse` fires FIRST (input-processor runner — per the
// installed .d.ts it is "called after the LLM step completes (or a cached
// response is replayed)"; the `fromCache: boolean` arg flags replays),
// `processOutputStep` fires LAST (output-processor runner) — so
// `processOutputStep` is the §6.1 backstop commit and the FIFO inflight pop
// is the at-most-one-commit guard between them.
//
// Correlation-key recovery at the commit hooks (V3 corollary): the commit
// hooks expose NO step messages, so the reserve-time §6.5 key (the
// adapter-derived runId) cannot be re-derived from content there. The
// hooks DO share the per-request, per-processor `state` bag
// (ProcessorRunner.getProcessorState keyed by processor id; one
// `processorStates` Map is threaded through every runner the loop builds
// for a request — input-step, request-bracket, output-step, AND api-error
// runners). The reserve hook therefore stashes the step's runId key in
// `args.state` and the commit hooks read it back; `opts.runIdProvider` is
// the secondary recovery source. This carries the LOCKED §6.5 key — it
// does not change the keying scheme or the §6.5 entry shape (the entry's
// additive `unit` field is the dated 2026-06-10 design amendment).
//
// ── [VERIFY-AT-IMPL: V7] PINNED (COV_D38_03, @mastra/core 1.41.0) ─────────
// Pin selected: **FAILURE commit at the signal** (first pre-declared
// alternative). The Processor surface exposes TWO error signals; both are
// handled and the FIFO inflight pop dedups them to one settlement:
//   1. PRIMARY (empirically proven, TP-27b): model-execution errors arrive
//      as an `error` CHUNK on `processLLMResponse`'s args.chunks —
//      `{ type: "error", payload: { error } }` (payload.error keeps
//      `.message`). A throwing model produces chunks
//      ["step-start", "error"] at the response hook, which emits the
//      FAILURE commit before Mastra rethrows the provider error.
//   2. SECONDARY: `processAPIError(args: { error: unknown, ... })` —
//      installed-surface hook for non-retryable API rejections;
//      `ProcessorRunner.runProcessAPIError` iterates input + output + error
//      processors, so the V5-pinned `inputProcessors` mount receives it.
//      Empirically NOT invoked for plain model-execution throws (signal 1
//      covers those); implemented as the belt for the API-rejection path.
// Limits honestly recorded:
//   - Mid-stream consumer ABORT (`options.abortSignal.aborted`) invokes
//     neither signal → the sidecar TTL sweep is the LOCKED settlement
//     backstop for aborts (design §6.1 last row / §8).
//   - NO cancel-before-dispatch hook exists on the installed surface →
//     NO `client.release()` path (design §11.9; absence does not block v1).
// The adapter never requests a retry (returns undefined): the original
// provider error must propagate (design §7 commit rows).

import type { MessageList } from "@mastra/core/agent";
import type {
  ProcessAPIErrorArgs,
  ProcessInputStepArgs,
  ProcessLLMRequestArgs,
  ProcessLLMResponseArgs,
  ProcessOutputStepArgs,
  Processor,
} from "@mastra/core/processors";
import type {
  BudgetClaim,
  CommitEstimatedRequest,
  DecisionOutcome,
  PricingFreeze,
  ReserveRequest,
  UnitRef,
} from "@spendguard/sdk";
import { flattenStepText } from "./flatten.js";
import { STEP_ID_LLM_CALL, deriveStepIdentity } from "./identity.js";
import { type InflightEntry, InflightMap } from "./inflight.js";
import type { SpendGuardProcessorOptions } from "./options.js";
import { type ExtractedUsage, extractUsage } from "./usage.js";

const DEFAULT_ROUTE = "mastra-llm";
const DEFAULT_UNIT: UnitRef = { unit: "USD_MICROS", denomination: 1 };
// Default commit freeze tuple when `opts.pricing` is absent (HARDEN_D05_WI:
// the commit's freeze tuple must match the reservation's — empty matches
// only no-bundle reservations; see design §6.7 amendment #3, 2026-06-11).
const EMPTY_PRICING: PricingFreeze = { pricingVersion: "", pricingHash: new Uint8Array(0) };
const CHARS_PER_TOKEN_HEURISTIC = 4;
const DEFAULT_MICROS_PER_TOKEN = 1_000n;
/** `args.state` slot carrying the §6.5 inflight key across hooks (V4 pin). */
const INFLIGHT_STATE_KEY = "spendguard.inflightKey";

/** Commit-outcome variants the settlement path emits (D04 safeCommit parity). */
type SettleOutcome =
  | { outcomeKind: "SUCCESS"; usage: ExtractedUsage | undefined }
  | { outcomeKind: "FAILURE"; actualErrorMessage: string };

/**
 * V7 PRIMARY error signal (pin header): scan the response hook's stripped
 * chunks for `{ type: "error", payload: { error } }` and surface the error
 * message. `payload.error` is an Error instance (or a serialized POJO with
 * `.message`) on the installed runtime.
 */
function findErrorChunkMessage(chunks: unknown): string | undefined {
  if (!Array.isArray(chunks)) {
    return undefined;
  }
  for (const chunk of chunks) {
    if (chunk === null || typeof chunk !== "object") {
      continue;
    }
    if ((chunk as Record<string, unknown>).type !== "error") {
      continue;
    }
    const payload = (chunk as Record<string, unknown>).payload;
    if (payload !== null && typeof payload === "object") {
      const error = (payload as Record<string, unknown>).error;
      if (error instanceof Error) {
        return error.message;
      }
      if (error !== null && typeof error === "object") {
        const message = (error as Record<string, unknown>).message;
        if (typeof message === "string") {
          return message;
        }
      }
      if (error !== undefined) {
        return String(error);
      }
    }
    return "provider error (no message exposed)";
  }
  return undefined;
}

export class SpendGuardProcessor implements Processor {
  /** Required by the installed `Processor` interface (V1 pin above). */
  readonly id = "spendguard-processor";
  /** Stable processor name (Mastra requires one per processor instance). */
  readonly name = "spendguard-processor";

  private readonly opts: SpendGuardProcessorOptions;
  private readonly inflight = new InflightMap();

  constructor(options: SpendGuardProcessorOptions) {
    if (options === null || typeof options !== "object") {
      throw new TypeError("SpendGuardProcessor: options must be an object");
    }
    if (!options.client) {
      throw new TypeError("SpendGuardProcessor: options.client is required");
    }
    if (typeof options.tenantId !== "string" || options.tenantId.length === 0) {
      throw new TypeError("SpendGuardProcessor: options.tenantId is required (non-empty string)");
    }
    this.opts = options;
  }

  /**
   * RESERVE — before-LLM-step boundary (design §6.1 row 1). Fires at every
   * step including tool-call continuations.
   */
  async processInputStep(args: ProcessInputStepArgs): Promise<undefined> {
    const stepText = flattenStepText(args.messages);
    const externalRunId = this.opts.runIdProvider?.();
    const identity = deriveStepIdentity({
      tenantId: this.opts.tenantId,
      stepText,
      // V3 PINNED: no Mastra run id at the hook (see inflight.ts) — the
      // only external source is the consumer-supplied runIdProvider.
      ...(externalRunId !== undefined ? { externalRunId } : {}),
    });
    const claims: BudgetClaim[] = this.opts.claimEstimator
      ? [
          ...this.opts.claimEstimator({
            stepText,
            runId: identity.runId,
            llmCallId: identity.llmCallId,
          }),
        ]
      : [this.projectClaim(stepText)];

    const req: ReserveRequest = {
      trigger: "LLM_CALL_PRE",
      runId: identity.runId,
      stepId: STEP_ID_LLM_CALL,
      llmCallId: identity.llmCallId,
      decisionId: identity.decisionId,
      route: this.opts.route ?? DEFAULT_ROUTE,
      projectedClaims: claims,
      idempotencyKey: identity.idempotencyKey,
    };

    // FAIL-CLOSED (design §7, LOCKED): NO try/catch around reserve(). Every
    // failure — DecisionDenied, DecisionStopped, ApprovalRequired,
    // SidecarUnavailable, HandshakeError, SpendGuardError — propagates and
    // halts the step before the provider call (V2 PINNED: throw directly;
    // the observable contract — zero provider calls on failure — is
    // test-pinned by TP-10/TP-13/TP-15).
    //
    // RESIDUAL(D38-V2): Mastra 1.41.0 serializes step errors inside its
    // internal workflow, so the consumer-facing catch contract is split:
    // at the AGENT boundary (`agent.generate()` rejection) consumers must
    // MESSAGE-MATCH — the typed error's message is preserved but the class
    // instance is not (`cause` chain ends in a serialized POJO); at the
    // HOOK boundary (this throw) `instanceof` holds (TP-13..16). Design §7.3
    // consumer-reachability prong tracked as
    // (gh issue #181: D38 V2 cause-chain);
    // COV_D38_06 documents it user-facing.
    const outcome: DecisionOutcome = await this.opts.client.reserve(req);

    this.inflight.push(identity.runId, {
      decisionId: outcome.decisionId,
      reservationId: outcome.reservationIds[0] ?? "",
      runId: identity.runId,
      llmCallId: identity.llmCallId,
      idempotencyKey: identity.idempotencyKey,
      projectedAmountAtomic: claims[0]?.amountAtomic ?? "0",
      // Reserve-time unit stash (design §6.5 dated amendment 2026-06-10):
      // a custom claimEstimator may reserve under a different unit/unitId,
      // and the commit must tuple-match the reservation (HARDEN_D05_WI).
      unit: claims[0]?.unit ?? this.buildUnit(),
      // Reserve-time pricing-freeze stash (design §6.7 amendment #3,
      // 2026-06-11): the production sidecar stamps the reservation with the
      // loaded bundle's freeze, and the commit must repeat it
      // (HARDEN_D05_WI; D04 `pending.pricing = opts.pricing` precedent).
      ...(this.opts.pricing !== undefined ? { pricing: this.opts.pricing } : {}),
    });
    // Stash the §6.5 inflight key for the commit hooks (V4 pin header):
    // the commit hooks expose no step messages, so they recover the key
    // from the request-scoped per-processor state bag. The loop is
    // sequential within a request (step N settles before step N+1 opens),
    // so overwriting per step is safe.
    if (args.state !== null && typeof args.state === "object") {
      (args.state as Record<string, unknown>)[INFLIGHT_STATE_KEY] = identity.runId;
    }
    // No changes returned — the processor never mutates the step (TP-21).
    return undefined;
  }

  /**
   * SUCCESS COMMIT — after each provider response (design §6.1 row 3).
   * Usage actuals when the finish chunk exposes them (V4 pin in usage.ts);
   * §6.6 LOCKED estimated-amount fallback otherwise.
   *
   * Commit-path errors are SWALLOWED (logged at error level) — design §7.4
   * LOCKED pre/post asymmetry: a post-call commit failure cannot un-spend;
   * the sidecar TTL sweep settles the reservation. This swallow must never
   * creep into the pre-dispatch reserve path (review-standards §2.6).
   */
  async processLLMResponse(args: ProcessLLMResponseArgs): Promise<undefined> {
    const entry = this.popInflight(args.state);
    if (entry === undefined) {
      console.warn(
        "[spendguard:mastra] processLLMResponse: no inflight entry (idempotent re-delivery?)",
      );
      return undefined;
    }
    // V7 PRIMARY signal: a model-execution error rides the chunk stream as
    // `{ type: "error", payload: { error } }` and STILL reaches this hook
    // (design §6.1 row 5 — "whichever hook Mastra exposes"). FAILURE
    // settlement here; Mastra rethrows the original provider error after.
    const errorMessage = findErrorChunkMessage(args.chunks);
    if (errorMessage !== undefined) {
      await this.settleCommit(entry, { outcomeKind: "FAILURE", actualErrorMessage: errorMessage });
      return undefined;
    }
    await this.settleCommit(entry, { outcomeKind: "SUCCESS", usage: extractUsage(args) });
    return undefined;
  }

  /**
   * Backstop COMMIT — after the step's output is assembled (design §6.1 row
   * 4). Fires only for `outputProcessors`-mounted instances and runs AFTER
   * `processLLMResponse` on streamed steps (V4 pin), so in the common case
   * the reservation is already settled and the inflight pop comes back
   * empty — that is the at-most-one-commit guard, not an error (silent
   * no-op; TP-31). It settles for real only when an open reservation
   * reaches this hook unsettled (e.g. a dual-mounted instance whose
   * response-hook settlement did not run); an output-mounted-ONLY instance
   * never reserves, so its backstop pop always no-ops.
   */
  async processOutputStep(args: ProcessOutputStepArgs): Promise<MessageList> {
    const entry = this.popInflight(args.state);
    if (entry !== undefined) {
      await this.settleCommit(entry, { outcomeKind: "SUCCESS", usage: extractUsage(args) });
    }
    // Returning the SAME MessageList instance = "no external list" under the
    // installed contract; the processor never mutates the step (TP-21).
    return args.messageList;
  }

  /**
   * FAILURE COMMIT — V7 SECONDARY signal (pin header): non-retryable API
   * rejections surfaced through the installed `processAPIError` hook
   * (design §6.1 row 5). The FIFO pop dedups against the response hook's
   * error-chunk settlement. Never requests a retry and never throws past
   * the commit swallow: the ORIGINAL provider error must propagate to the
   * consumer (design §7 commit rows).
   */
  async processAPIError(args: ProcessAPIErrorArgs): Promise<undefined> {
    const entry = this.popInflight(args.state);
    if (entry !== undefined) {
      const message = args.error instanceof Error ? args.error.message : String(args.error);
      await this.settleCommit(entry, { outcomeKind: "FAILURE", actualErrorMessage: message });
    }
    // undefined → the adapter does not handle the error; Mastra surfaces it.
    return undefined;
  }

  /**
   * No-op in v1 (design §11.3 LOCKED): the reserve already brackets the
   * step at `processInputStep`. Kept as the pinned fallback reserve point
   * if a model path ever skips `processInputStep` (V1 register note); any
   * reserve logic here is drift.
   */
  processLLMRequest(_args: ProcessLLMRequestArgs): undefined {
    return undefined;
  }

  /** §6.4 LOCKED default claim projection (D04/D06 parity). */
  private projectClaim(stepText: string): BudgetClaim {
    const estimatedTokens = BigInt(
      Math.max(1, Math.ceil(stepText.length / CHARS_PER_TOKEN_HEURISTIC)),
    );
    const cap = this.opts.defaultBudgetMicrosCap;
    const amountMicros =
      cap !== undefined && cap > 0n ? cap : estimatedTokens * DEFAULT_MICROS_PER_TOKEN;
    return {
      scopeId: this.opts.budgetId ?? this.opts.tenantId,
      amountAtomic: amountMicros.toString(),
      unit: this.buildUnit(),
    };
  }

  private buildUnit(): UnitRef {
    // HARDEN_D05_UR — day-1 unitId threading (design §11.5). Omitted unitId
    // keeps the pre-HARDEN wire shape (substrate coerces to "").
    return this.opts.unitId ? { ...DEFAULT_UNIT, unitId: this.opts.unitId } : DEFAULT_UNIT;
  }

  /**
   * Recover the §6.5 inflight key at a commit hook (V4 pin header) and pop
   * the oldest open entry for it. Key sources, in order: the state-stashed
   * per-step runId, then the consumer's runIdProvider. No key / no entry →
   * undefined (caller decides warn vs silent backstop no-op).
   */
  private popInflight(state: unknown): InflightEntry | undefined {
    let key: string | undefined;
    if (state !== null && typeof state === "object") {
      const stashed = (state as Record<string, unknown>)[INFLIGHT_STATE_KEY];
      if (typeof stashed === "string" && stashed.length > 0) {
        key = stashed;
      }
    }
    key = key ?? this.opts.runIdProvider?.();
    return key === undefined ? undefined : this.inflight.pop(key);
  }

  /**
   * Emit the settlement `commitEstimated` for a popped reservation —
   * tuple-matched to the reserve (HARDEN_D05_WI): same identity tuple, same
   * unit, same pricing freeze (`opts.pricing` stash; empty tuple when the
   * option is absent — design §6.7 amendment #3).
   *
   *   - SUCCESS + usage: actuals on the wire fields; estimate = token sum
   *     (shipped-D04-handler wire shape, HARDEN_D05_WI — the ledger rejects
   *     `estimated_amount_atomic = 0` bookings).
   *   - SUCCESS without usage (§6.6 LOCKED fallback): estimate =
   *     reserve-time `projectedAmountAtomic`; actuals OMITTED — the audit
   *     chain records that no provider actuals were observed.
   *   - FAILURE: estimate = reserve-time projection (usage is absent on the
   *     error path — same §6.6 rule), `actualErrorMessage` threaded.
   *
   * Commit RPC errors are swallowed at error level (§7.4 LOCKED asymmetry;
   * sidecar TTL sweep + audit chain settle the reservation). KNOWN drift,
   * absorbed: the sidecar may reject the outcome COMPANION event with
   * "missing estimated_amount_atomic" — the booking still lands; the
   * warn-not-throw path covers it (do not chase).
   */
  private async settleCommit(entry: InflightEntry, outcome: SettleOutcome): Promise<void> {
    let estimatedAmountAtomic = entry.projectedAmountAtomic;
    if (outcome.outcomeKind === "SUCCESS" && outcome.usage !== undefined) {
      try {
        estimatedAmountAtomic = (
          BigInt(outcome.usage.inputTokens) + BigInt(outcome.usage.outputTokens)
        ).toString();
      } catch {
        estimatedAmountAtomic = entry.projectedAmountAtomic;
      }
    }
    const req: CommitEstimatedRequest = {
      runId: entry.runId,
      stepId: STEP_ID_LLM_CALL,
      llmCallId: entry.llmCallId,
      decisionId: entry.decisionId,
      reservationId: entry.reservationId,
      estimatedAmountAtomic,
      // Reserve-time unit reuse (design §6.5 dated amendment 2026-06-10):
      // the inflight entry carries the reservation's `claim[0].unit`, so a
      // custom claimEstimator's unit/unitId tuple-matches on the commit
      // (HARDEN_D05_WI; D04 `pending.unit = projectedClaim.unit` precedent).
      unit: entry.unit,
      // Reserve-time pricing-freeze reuse (design §6.7 amendment #3,
      // 2026-06-11): repeat the stashed `opts.pricing` tuple — the sidecar
      // stamps reservations with the loaded bundle's freeze and rejects
      // empty-tuple commits with `pricing freeze mismatch` (proved live by
      // the COV_D38_05 demo). Absent option → empty tuple (back-compat:
      // matches no-bundle reservations only).
      pricing: entry.pricing ?? EMPTY_PRICING,
      providerEventId:
        outcome.outcomeKind === "SUCCESS" ? (outcome.usage?.providerEventId ?? "") : "",
      ...(outcome.outcomeKind === "SUCCESS"
        ? {
            outcome: "SUCCESS" as const,
            outcomeKind: "SUCCESS" as const,
            ...(outcome.usage !== undefined
              ? {
                  actualInputTokensWire: String(outcome.usage.inputTokens),
                  actualOutputTokensWire: String(outcome.usage.outputTokens),
                }
              : {}),
          }
        : {
            outcome: "PROVIDER_ERROR" as const,
            outcomeKind: "FAILURE" as const,
            actualErrorMessage: outcome.actualErrorMessage,
          }),
    };
    try {
      await this.opts.client.commitEstimated(req);
    } catch (err) {
      // §7.4 LOCKED: post-dispatch commit failure must never destroy the
      // consumer's already-paid-for result. TTL sweep settles.
      console.error(
        `[spendguard:mastra] commitEstimated(${outcome.outcomeKind}) failed for runId=${entry.runId}; TTL sweep will settle: ${
          err instanceof Error ? err.message : String(err)
        }`,
      );
    }
  }
}
