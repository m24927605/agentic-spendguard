// SLICE 4 — In-process mock of `SpendGuardClient` for end-to-end integration
// tests against a stubbed `@langchain/openai` ChatOpenAI.
//
// **Scope vs D05's mock**: the D05 mock at `sdk/typescript/tests/_support/`
// stands up a real UDS gRPC server because the SDK itself owns the wire path;
// the LangChain adapter only ever sees the `SpendGuardClient` *interface*. So
// SLICE 4 mocks at the interface boundary — a tiny "gRPC-equivalent" double
// that satisfies the two RPCs the handler touches (`reserve` /
// `commitEstimated`) plus the `tenantId` getter the adapter reads for
// idempotency-key derivation.
//
// The mock records every call so tests can assert against the exact wire
// shape the handler emits. Per-test overrides drive the four axes the SLICE 4
// scope sheet enumerates:
//
//   1. `nextDecision`  — happy-path ALLOW or terminal DENY / STOP / APPROVAL.
//   2. `simulatedLatencyMs` — sleep before resolving so concurrency tests can
//      prove independent inflight slots.
//   3. `simulatedReserveError` — reject `reserve()` with a typed error so the
//      handler's throw-on-deny vs swallow-on-unavailable path is exercised.
//   4. `simulatedCommitError`  — reject `commitEstimated()` so the handler's
//      commit-error swallow path is exercised.
//
// The mock is plain TypeScript — no UDS, no gRPC, no protobuf — so the tests
// stay deterministic and fast.

import {
  ApprovalRequired,
  type BudgetClaim,
  type CommitEstimatedRequest,
  DecisionDenied,
  type DecisionOutcome,
  DecisionStopped,
  type ReserveRequest,
  SidecarUnavailable,
  type SpendGuardClient,
} from "@spendguard/sdk";

// ── Decision shapes ───────────────────────────────────────────────────────

/**
 * Test-time decision plan: tells the mock how the next (or per-call) `reserve`
 * should resolve. ALLOW → returns a synthetic `DecisionOutcome`; DENY / STOP /
 * APPROVAL_REQUIRED → rejects with the matching typed error from
 * `@spendguard/sdk`.
 *
 * `SIDECAR_UNAVAILABLE` is also surfaced as a `DecisionPlan` variant — it is
 * the canonical operational-degradation case that the handler must SWALLOW
 * (the LLM call proceeds without a budget gate). Keeping it in the same enum
 * as ALLOW/DENY means tests pick the desired path with a single field.
 */
export type DecisionPlan =
  | { kind: "ALLOW"; decisionId?: string; reservationId?: string }
  | { kind: "DENY"; reasonCodes?: readonly string[] }
  | { kind: "STOP"; reasonCodes?: readonly string[] }
  | { kind: "APPROVAL_REQUIRED"; approvalRequestId?: string }
  | { kind: "SIDECAR_UNAVAILABLE"; message?: string };

/** Default ALLOW outcome the mock returns when no override is configured. */
const DEFAULT_OUTCOME: DecisionOutcome = {
  decisionId: "mock-decision-default",
  auditDecisionEventId: "mock-audit-default",
  decision: "CONTINUE",
  mutationPatchJson: "{}",
  effectHash: new Uint8Array(0),
  ledgerTransactionId: "mock-ledger-tx-default",
  reservationIds: ["mock-reservation-default"],
  ttlExpiresAtSeconds: 0,
  reasonCodes: [],
  matchedRuleIds: [],
};

// ── Recorded call shapes ──────────────────────────────────────────────────

/**
 * Snapshot of one `reserve` invocation. Tests assert against the exact
 * wire-shape fields the handler emitted (`trigger`, `runId`, `llmCallId`,
 * `idempotencyKey`, `projectedClaims[0].amountAtomic`, etc.).
 */
export interface RecordedReserve {
  request: ReserveRequest;
  /** Set when reserve rejected; carries the typed error class name. */
  rejected?: { name: string; message: string };
  /** Resolved outcome (omitted when the call rejected). */
  resolved?: DecisionOutcome;
  /** Order of the call across the mock's lifetime (1-indexed). */
  callIndex: number;
}

/**
 * Snapshot of one `commitEstimated` invocation. Mirrors `RecordedReserve` for
 * the POST/ERROR branch — `outcomeKind` carries SUCCESS / FAILURE so tests
 * can assert handleLLMEnd vs handleLLMError without re-deriving from the
 * `outcome` field.
 */
export interface RecordedCommit {
  request: CommitEstimatedRequest;
  rejected?: { name: string; message: string };
  callIndex: number;
}

// ── Mock options ──────────────────────────────────────────────────────────

/**
 * Per-call queue of decisions. The mock pops one entry per `reserve` call;
 * when the queue empties it falls back to the default ALLOW. This shape is
 * tuned for the concurrent-invokes test case (3 parallel chains each with
 * a distinct decision).
 */
export interface MockSidecarOptions {
  tenantId?: string;
  sessionId?: string;
  /** Override the queue of `reserve` decisions. */
  decisionQueue?: DecisionPlan[];
  /** Default decision when the queue is empty. Defaults to ALLOW. */
  defaultDecision?: DecisionPlan;
  /** Sleep before resolving `reserve` (ms). Default 0. */
  simulatedReserveLatencyMs?: number;
  /** Sleep before resolving `commitEstimated` (ms). Default 0. */
  simulatedCommitLatencyMs?: number;
  /** When set, every commit rejects with this error. */
  simulatedCommitError?: Error;
}

// ── MockSpendGuardClient ──────────────────────────────────────────────────

/**
 * In-process mock implementing the `SpendGuardClient` interface surface the
 * LangChain adapter touches. Construction is synchronous (no UDS bind, no
 * gRPC channel), so tests start instantly.
 *
 * Usage:
 *
 *     const mock = new MockSpendGuardClient({
 *       decisionQueue: [{ kind: "ALLOW" }, { kind: "DENY" }],
 *     });
 *     const handler = new SpendGuardCallbackHandler({ client: mock.client });
 *     // ... drive a ChatOpenAI through the handler ...
 *     expect(mock.reserveCalls).toHaveLength(2);
 *
 * Inspection surface:
 *   - `reserveCalls` / `commitCalls` — captured call records.
 *   - `tenantId` / `sessionId` — getters the adapter reads.
 *   - `client` — cast to `SpendGuardClient` so the handler can consume it.
 */
export class MockSpendGuardClient {
  readonly tenantId: string;
  readonly sessionId: string;

  /** Queue of decisions consumed per `reserve` call. */
  private readonly decisionQueue: DecisionPlan[];
  private readonly defaultDecision: DecisionPlan;
  private readonly simulatedReserveLatencyMs: number;
  private readonly simulatedCommitLatencyMs: number;
  private readonly simulatedCommitError: Error | undefined;

  /** All `reserve` invocations in arrival order. */
  readonly reserveCalls: RecordedReserve[] = [];
  /** All `commitEstimated` invocations in arrival order. */
  readonly commitCalls: RecordedCommit[] = [];

  private reserveCounter = 0;
  private commitCounter = 0;

  constructor(options: MockSidecarOptions = {}) {
    this.tenantId = options.tenantId ?? "tenant-mock-default";
    this.sessionId = options.sessionId ?? "session-mock-default";
    this.decisionQueue = options.decisionQueue ? [...options.decisionQueue] : [];
    this.defaultDecision = options.defaultDecision ?? { kind: "ALLOW" };
    this.simulatedReserveLatencyMs = options.simulatedReserveLatencyMs ?? 0;
    this.simulatedCommitLatencyMs = options.simulatedCommitLatencyMs ?? 0;
    this.simulatedCommitError = options.simulatedCommitError;
  }

  /**
   * Cast helper — the adapter takes a `SpendGuardClient`, so the test rig
   * hands it the mock under the locked interface contract. The mock only
   * implements the surface the SLICE 3 reserve/commit path touches.
   */
  get client(): SpendGuardClient {
    return this as unknown as SpendGuardClient;
  }

  /** Mock implementation of `SpendGuardClient.reserve(...)`. */
  async reserve(req: ReserveRequest): Promise<DecisionOutcome> {
    this.reserveCounter += 1;
    const callIndex = this.reserveCounter;
    const plan = this.decisionQueue.shift() ?? this.defaultDecision;
    if (this.simulatedReserveLatencyMs > 0) {
      await sleep(this.simulatedReserveLatencyMs);
    }
    const record: RecordedReserve = { request: req, callIndex };

    switch (plan.kind) {
      case "ALLOW": {
        const outcome: DecisionOutcome = {
          ...DEFAULT_OUTCOME,
          decisionId: plan.decisionId ?? `mock-decision-${callIndex}`,
          reservationIds: [plan.reservationId ?? `mock-reservation-${callIndex}`],
        };
        record.resolved = outcome;
        this.reserveCalls.push(record);
        return outcome;
      }
      case "DENY": {
        const err = new DecisionDenied("mock budget denied", {
          decisionId: `mock-decision-deny-${callIndex}`,
          reasonCodes: plan.reasonCodes ?? ["BUDGET_EXCEEDED"],
        });
        record.rejected = { name: err.name, message: err.message };
        this.reserveCalls.push(record);
        throw err;
      }
      case "STOP": {
        const err = new DecisionStopped("mock STOP terminal", {
          decisionId: `mock-decision-stop-${callIndex}`,
          reasonCodes: plan.reasonCodes ?? ["projection.run.over_threshold"],
        });
        record.rejected = { name: err.name, message: err.message };
        this.reserveCalls.push(record);
        throw err;
      }
      case "APPROVAL_REQUIRED": {
        const err = new ApprovalRequired("mock approval required", {
          decisionId: `mock-decision-approval-${callIndex}`,
          approvalRequestId: plan.approvalRequestId ?? `mock-approval-${callIndex}`,
          reasonCodes: ["approval_required"],
        });
        record.rejected = { name: err.name, message: err.message };
        this.reserveCalls.push(record);
        throw err;
      }
      case "SIDECAR_UNAVAILABLE": {
        const err = new SidecarUnavailable(plan.message ?? "mock sidecar UDS gone");
        record.rejected = { name: err.name, message: err.message };
        this.reserveCalls.push(record);
        throw err;
      }
    }
  }

  /** Mock implementation of `SpendGuardClient.commitEstimated(...)`. */
  async commitEstimated(req: CommitEstimatedRequest): Promise<void> {
    this.commitCounter += 1;
    const callIndex = this.commitCounter;
    if (this.simulatedCommitLatencyMs > 0) {
      await sleep(this.simulatedCommitLatencyMs);
    }
    const record: RecordedCommit = { request: req, callIndex };
    if (this.simulatedCommitError !== undefined) {
      record.rejected = {
        name: this.simulatedCommitError.name,
        message: this.simulatedCommitError.message,
      };
      this.commitCalls.push(record);
      throw this.simulatedCommitError;
    }
    this.commitCalls.push(record);
  }

  /** Convenience: the last reserve request the handler emitted. */
  get lastReserveRequest(): ReserveRequest | undefined {
    return this.reserveCalls[this.reserveCalls.length - 1]?.request;
  }

  /** Convenience: the last commit request the handler emitted. */
  get lastCommitRequest(): CommitEstimatedRequest | undefined {
    return this.commitCalls[this.commitCalls.length - 1]?.request;
  }

  /** Convenience: the projected claim amount on the most-recent reserve. */
  get lastClaimAmountAtomic(): bigint | undefined {
    const claim = this.lastReserveRequest?.projectedClaims[0];
    return claim ? BigInt(claim.amountAtomic) : undefined;
  }

  /** Reset all recorded calls (useful between sequential test phases). */
  reset(): void {
    this.reserveCalls.length = 0;
    this.commitCalls.length = 0;
    this.reserveCounter = 0;
    this.commitCounter = 0;
  }
}

/**
 * Helper to build a synthetic `BudgetClaim` projection — useful for tests
 * that want to assert against a known claim shape without re-deriving the
 * handler's default heuristic.
 */
export function makeBudgetClaim(scopeId: string, amountAtomic: bigint | string): BudgetClaim {
  return {
    scopeId,
    amountAtomic: typeof amountAtomic === "bigint" ? amountAtomic.toString() : amountAtomic,
    unit: { unit: "USD_MICROS", denomination: 1 },
  };
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
