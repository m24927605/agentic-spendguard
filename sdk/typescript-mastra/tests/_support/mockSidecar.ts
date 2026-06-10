// COV_D38_02 — In-process mock of `SpendGuardClient` for the processor
// test matrix.
//
// **Why an interface-level mock vs the D05 UDS mock?**: the D05 substrate
// mocks at the UDS gRPC boundary because the SDK itself owns the wire path.
// The Mastra adapter only sees the `SpendGuardClient` *interface* —
// `processInputStep` consumes `reserve()` directly (and COV_D38_03's commit
// hooks consume `commitEstimated()`). Adapting the D06
// `tests/_support/mockSidecar.ts` interface-level pattern keeps the TP suite
// focused on the processor ↔ Mastra surface (the wire path is exercised by
// D05's own UDS suite). implementation.md §1 sketched a re-export of the
// substrate UDS mock; the adapted interface-level mock is the D06-precedent
// shape that actually matches what the adapter touches.
//
// D38 fail-closed note (design §7 LOCKED rule 1): EVERY rejecting plan —
// including SIDECAR_UNAVAILABLE and HANDSHAKE_ERROR — must propagate out of
// `processInputStep` and abort the step. There is NO swallow path in this
// adapter; tests assert propagation, never continuation.

import {
  ApprovalRequired,
  type BudgetClaim,
  type CommitEstimatedRequest,
  DecisionDenied,
  type DecisionOutcome,
  DecisionStopped,
  HandshakeError,
  type ReserveRequest,
  SidecarUnavailable,
  type SpendGuardClient,
} from "@spendguard/sdk";

// ── Decision shapes ───────────────────────────────────────────────────────

/**
 * Test-time decision plan: tells the mock how the next (or per-call)
 * `reserve` should resolve. ALLOW → returns a synthetic `DecisionOutcome`;
 * every other variant rejects with the matching typed error from
 * `@spendguard/sdk`. All rejecting variants are FAIL-CLOSED for D38: the
 * processor must propagate them (design §7 rules 1-3).
 */
export type DecisionPlan =
  | { kind: "ALLOW"; decisionId?: string; reservationId?: string }
  | { kind: "DENY"; reasonCodes?: readonly string[] }
  | { kind: "STOP"; reasonCodes?: readonly string[] }
  | { kind: "APPROVAL_REQUIRED"; approvalRequestId?: string }
  | { kind: "SIDECAR_UNAVAILABLE"; message?: string }
  | { kind: "HANDSHAKE_ERROR"; message?: string }
  | { kind: "SENTINEL_ERROR"; error: Error };

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
 * wire-shape fields the processor emitted (`trigger`, `runId`, `llmCallId`,
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

/** Snapshot of one `commitEstimated` invocation (COV_D38_03 consumer). */
export interface RecordedCommit {
  request: CommitEstimatedRequest;
  rejected?: { name: string; message: string };
  callIndex: number;
}

// ── Mock options ──────────────────────────────────────────────────────────

/**
 * Per-call queue of decisions. The mock pops one entry per `reserve` call;
 * when the queue empties it falls back to the default decision (ALLOW
 * unless overridden).
 */
export interface MockSidecarOptions {
  tenantId?: string;
  sessionId?: string;
  /** Override the queue of `reserve` decisions. */
  decisionQueue?: DecisionPlan[];
  /** Default decision when the queue is empty. Defaults to ALLOW. */
  defaultDecision?: DecisionPlan;
  /** When set, every commit rejects with this error. */
  simulatedCommitError?: Error;
}

// ── MockSpendGuardClient ──────────────────────────────────────────────────

/**
 * In-process mock implementing the `SpendGuardClient` interface surface the
 * Mastra adapter touches. Construction is synchronous (no UDS bind, no gRPC
 * channel), so tests start instantly.
 *
 * Usage:
 *
 *     const mock = new MockSpendGuardClient({
 *       decisionQueue: [{ kind: "ALLOW" }, { kind: "DENY" }],
 *     });
 *     const guard = new SpendGuardProcessor({
 *       client: mock.client,
 *       tenantId: "tenant-test",
 *     });
 *     // ... drive a real Agent (or the hook directly) through the guard ...
 *     expect(mock.reserveCalls).toHaveLength(2);
 */
export class MockSpendGuardClient {
  readonly tenantId: string;
  readonly sessionId: string;

  private readonly decisionQueue: DecisionPlan[];
  private readonly defaultDecision: DecisionPlan;
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
    this.simulatedCommitError = options.simulatedCommitError;
  }

  /**
   * Cast helper — the adapter takes a `SpendGuardClient`, so the test rig
   * hands it the mock under the locked interface contract. The mock only
   * implements the surface the reserve/commit path touches.
   */
  get client(): SpendGuardClient {
    return this as unknown as SpendGuardClient;
  }

  /** Mock implementation of `SpendGuardClient.reserve(...)`. */
  async reserve(req: ReserveRequest): Promise<DecisionOutcome> {
    this.reserveCounter += 1;
    const callIndex = this.reserveCounter;
    const plan = this.decisionQueue.shift() ?? this.defaultDecision;
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
      case "HANDSHAKE_ERROR": {
        const err = new HandshakeError(plan.message ?? "mock handshake missing");
        record.rejected = { name: err.name, message: err.message };
        this.reserveCalls.push(record);
        throw err;
      }
      case "SENTINEL_ERROR": {
        record.rejected = { name: plan.error.name, message: plan.error.message };
        this.reserveCalls.push(record);
        throw plan.error;
      }
    }
  }

  /** Mock implementation of `SpendGuardClient.commitEstimated(...)`. */
  async commitEstimated(req: CommitEstimatedRequest): Promise<void> {
    this.commitCounter += 1;
    const callIndex = this.commitCounter;
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

  /** Convenience: the last reserve request the processor emitted. */
  get lastReserveRequest(): ReserveRequest | undefined {
    return this.reserveCalls[this.reserveCalls.length - 1]?.request;
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
 * processor's default heuristic.
 */
export function makeBudgetClaim(scopeId: string, amountAtomic: bigint | string): BudgetClaim {
  return {
    scopeId,
    amountAtomic: typeof amountAtomic === "bigint" ? amountAtomic.toString() : amountAtomic,
    unit: { unit: "USD_MICROS", denomination: 1 },
  };
}
