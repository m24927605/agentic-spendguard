// src/processor.ts — SpendGuardProcessor (LOCKED shape design §5; reserve
// hook body per design §6 / implementation.md §3.4).
//
// COV_D38_02 ships the pre-dispatch reserve path only:
//   - `processInputStep`  → RESERVE (fail-closed, design §7 rules 1-3)
//   - `processLLMRequest` → no-op in v1 (design §11.3)
// Commit/failure hooks (`processLLMResponse` / `processOutputStep`) land in
// COV_D38_03.
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
// `processOutputStep` is recorded by COV_D38_03 (V4/V7). Quickstart copies:
//   new Agent({ name, instructions, model, inputProcessors: [guard] })

import type {
  ProcessInputStepArgs,
  ProcessLLMRequestArgs,
  Processor,
} from "@mastra/core/processors";
import type { BudgetClaim, DecisionOutcome, ReserveRequest, UnitRef } from "@spendguard/sdk";
import { flattenStepText } from "./flatten.js";
import { STEP_ID_LLM_CALL, deriveStepIdentity } from "./identity.js";
import { InflightMap } from "./inflight.js";
import type { SpendGuardProcessorOptions } from "./options.js";

const DEFAULT_ROUTE = "mastra-llm";
const DEFAULT_UNIT: UnitRef = { unit: "USD_MICROS", denomination: 1 };
const CHARS_PER_TOKEN_HEURISTIC = 4;
const DEFAULT_MICROS_PER_TOKEN = 1_000n;

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
    });
    // No changes returned — the processor never mutates the step (TP-21).
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
}
