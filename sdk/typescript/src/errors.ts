// SpendGuard SDK — typed error hierarchy.
//
// Mirrors the Python SDK `errors.py` one-to-one (design.md §4.5, review-standards
// §5). The hierarchy is the contract that downstream adapters (D04 / D06 / D08 /
// D29) route on: a single `instanceof SpendGuardError` catches everything; more
// specific `instanceof DecisionDenied` etc. routes the policy outcomes.
//
// Locked decisions enforced here:
//   - Every class has a stable `name` field (preserved across JSON.stringify).
//   - `SidecarUnavailable.statusCode === 503 as const`; `DecisionDenied.statusCode === 403 as const`.
//   - `cause` is forwarded when the constructor receives it (ES2022 Error.cause).
//   - `ApprovalRequired.resume()` delegates to `client.resumeAfterApproval`.
//
// SLICE 3 wires the full hierarchy because (a) the slice doc lists Config /
// Connection / Decision discriminated subtypes as a deliverable, and (b)
// downstream adapter specs already reference the full surface — restricting to
// a subset here would force a v0.minor bump in SLICE 4. See review-standards.md
// §5 for the parity gate.

/**
 * Root of the SpendGuard error hierarchy. Every error thrown by the SDK
 * inherits from this class so adapters can route with one `instanceof` check.
 */
export class SpendGuardError extends Error {
  override name = "SpendGuardError";
  constructor(message: string, opts?: { cause?: unknown }) {
    super(message);
    if (opts?.cause !== undefined) {
      (this as { cause?: unknown }).cause = opts.cause;
    }
    // Preserve `name` across JSON.stringify (review-standards §5.6). Default
    // Error makes `name` non-enumerable; assigning here flips it to enumerable
    // so JSON output carries it.
    Object.defineProperty(this, "name", {
      value: this.name,
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
}

/**
 * Constructor-time configuration error. Thrown synchronously from
 * `new SpendGuardClient(...)` or `SpendGuardClient.fromEnv()` when:
 *   - `socketPath` is missing and `SPENDGUARD_SOCKET_PATH` /
 *     `SPENDGUARD_SIDECAR_UDS` are unset.
 *   - `tenantId` is missing and `SPENDGUARD_TENANT_ID` is unset.
 *   - `otelTracer` and `onSpan` are both provided (mutually exclusive).
 *   - `SPENDGUARD_DECISION_TIMEOUT_MS` is not a finite non-negative integer.
 */
export class SpendGuardConfigError extends SpendGuardError {
  override name = "SpendGuardConfigError";
}

/**
 * Connection-layer failure: the UDS / gRPC channel could not be opened,
 * the sidecar is unreachable, the deadline expired before reply, or the
 * server cancelled mid-flight.
 *
 * Adapters typically map `SidecarUnavailable` → 503 upstream (the `statusCode`
 * is the conventional HTTP analog; the SDK itself does not speak HTTP).
 *
 * Raised by `mapGrpcStatusToError` (SLICE 5) for:
 *   - gRPC `UNAVAILABLE` — channel torn down / sidecar gone.
 *   - gRPC `DEADLINE_EXCEEDED` — request timed out before reply.
 *   - gRPC `CANCELLED` — server (or client) cancelled mid-flight.
 *
 * The original `RpcError` is preserved in `cause` so adapters can dig into
 * the underlying gRPC trailer metadata if they need to.
 *
 * SLICE 8 wires retry classification for the same cluster into this class
 * via `_classify_rpc_error` parity.
 */
export class SidecarUnavailable extends SpendGuardError {
  override name = "SidecarUnavailable";
  readonly statusCode = 503 as const;
}

/**
 * Connection-not-established error. Thrown when the caller invokes an RPC
 * before `connect()` (or attempts to read `sessionId` before `handshake()`).
 */
export class SpendGuardConnectionError extends SpendGuardError {
  override name = "SpendGuardConnectionError";
}

/**
 * Handshake-protocol failure: the sidecar replied with a protocol-version
 * mismatch, a capability level the adapter cannot satisfy, or an unsigned
 * announcement when one was required.
 *
 * SLICE 4 wires the handshake payload mapping; the class itself is locked here.
 */
export class HandshakeError extends SpendGuardError {
  override name = "HandshakeError";
}

/**
 * Init payload for `DecisionDenied` (and its subclasses). All decision-typed
 * errors carry the audit chain coordinates so adapters can correlate with the
 * sidecar's emitted CloudEvents.
 */
export interface DecisionDeniedInit {
  decisionId: string;
  reasonCodes?: readonly string[];
  auditDecisionEventId?: string;
  matchedRuleIds?: readonly string[];
}

/**
 * Decision-time denial. Thrown when the sidecar returns a non-`CONTINUE` /
 * non-`DEGRADE` outcome. Subclasses discriminate by terminal kind:
 * `DecisionStopped` (STOP / STOP_RUN_PROJECTION), `DecisionSkipped` (SKIP),
 * `ApprovalRequired` (REQUIRE_APPROVAL).
 *
 * SLICE 4 wires the mapping from `DecisionResponse.decision` enum to the
 * concrete subclass; this base class is locked here.
 */
export class DecisionDenied extends SpendGuardError {
  override name = "DecisionDenied";
  readonly statusCode = 403 as const;
  readonly decisionId: string;
  readonly reasonCodes: readonly string[];
  readonly auditDecisionEventId?: string;
  readonly matchedRuleIds: readonly string[];
  constructor(message: string, init: DecisionDeniedInit, opts?: { cause?: unknown }) {
    super(message, opts);
    this.decisionId = init.decisionId;
    this.reasonCodes = init.reasonCodes ?? [];
    if (init.auditDecisionEventId !== undefined) {
      this.auditDecisionEventId = init.auditDecisionEventId;
    }
    this.matchedRuleIds = init.matchedRuleIds ?? [];
  }
}

/**
 * Sidecar returned `STOP` or `STOP_RUN_PROJECTION`. Run loop must unwind
 * without further LLM / tool calls. `reasonCodes` carry the contract-DSL
 * matched-rule outcomes (`projection.run.over_threshold`, etc.).
 */
export class DecisionStopped extends DecisionDenied {
  override name = "DecisionStopped";
}

/**
 * Sidecar returned `SKIP` — current step / call must be skipped but the run
 * may continue. Treated as a terminal decision for the in-flight boundary
 * only.
 */
export class DecisionSkipped extends DecisionDenied {
  override name = "DecisionSkipped";
}

/**
 * Init payload for `ApprovalRequired`. Carries the approval bookkeeping the
 * adapter needs to drive the human-in-the-loop resume round-trip.
 */
export interface ApprovalRequiredInit extends DecisionDeniedInit {
  approvalRequestId: string;
  approverRole?: string;
  tenantId?: string;
}

/**
 * Minimal client shape that `ApprovalRequired.resume()` needs. Avoids a
 * circular import between `errors.ts` and `client.ts`; the real
 * `SpendGuardClient` satisfies this contract.
 */
export interface ApprovalResumeClient {
  resumeAfterApproval(req: {
    approvalId: string;
    tenantId: string;
    decisionId: string;
  }): Promise<unknown>;
}

/**
 * Sidecar returned `REQUIRE_APPROVAL`. The adapter must surface the approval
 * to a human operator (Slack / control plane / etc.) and call
 * `await err.resume(client)` after the operator acts.
 *
 * `tenantId` is carried explicitly so the resume round-trip can scope the
 * GetApprovalForResume lookup against tenant (per Python `client.py` round-2
 * #9 part 2 PR 9d).
 */
export class ApprovalRequired extends DecisionDenied {
  override name = "ApprovalRequired";
  readonly approvalRequestId: string;
  readonly approverRole?: string;
  readonly tenantId?: string;
  constructor(message: string, init: ApprovalRequiredInit, opts?: { cause?: unknown }) {
    super(message, init, opts);
    this.approvalRequestId = init.approvalRequestId;
    if (init.approverRole !== undefined) {
      this.approverRole = init.approverRole;
    }
    if (init.tenantId !== undefined) {
      this.tenantId = init.tenantId;
    }
  }

  /**
   * Resume the decision after a human operator has acted. Delegates to
   * `client.resumeAfterApproval(...)`.
   *
   * @throws ApprovalDeniedError when the operator rejected.
   * @throws ApprovalLapsedError when the approval expired / was cancelled.
   * @throws ApprovalBundleHotReloadedError when bundle rotated mid-approval.
   */
  async resume(client: ApprovalResumeClient): Promise<unknown> {
    return client.resumeAfterApproval({
      approvalId: this.approvalRequestId,
      tenantId: this.tenantId ?? "",
      decisionId: this.decisionId,
    });
  }
}

/**
 * Operator explicitly denied the approval. Carries `approverSubject` and
 * `approverReason` from the audit row so the adapter can surface them upstream.
 *
 * Reason codes are forced to include `approval_denied` as the first entry per
 * Python parity.
 */
export class ApprovalDeniedError extends DecisionDenied {
  override name = "ApprovalDeniedError";
  readonly approverSubject?: string;
  readonly approverReason?: string;
  constructor(
    message: string,
    init: DecisionDeniedInit & { approverSubject?: string; approverReason?: string },
    opts?: { cause?: unknown },
  ) {
    super(
      message,
      { ...init, reasonCodes: ["approval_denied", ...(init.reasonCodes ?? [])] },
      opts,
    );
    if (init.approverSubject !== undefined) {
      this.approverSubject = init.approverSubject;
    }
    if (init.approverReason !== undefined) {
      this.approverReason = init.approverReason;
    }
  }
}

/**
 * Approval state was non-terminal at the time the adapter polled. State is
 * one of `pending` / `expired` / `cancelled` / `unknown`; the reason codes
 * include `approval_lapsed_<state>` per Python parity.
 */
export class ApprovalLapsedError extends DecisionDenied {
  override name = "ApprovalLapsedError";
  readonly state: "pending" | "expired" | "cancelled" | "unknown";
  constructor(
    message: string,
    init: DecisionDeniedInit & { state: "pending" | "expired" | "cancelled" | "unknown" },
    opts?: { cause?: unknown },
  ) {
    super(
      message,
      { ...init, reasonCodes: [`approval_lapsed_${init.state}`, ...(init.reasonCodes ?? [])] },
      opts,
    );
    this.state = init.state;
  }
}

/**
 * Approval was issued under one contract-bundle hash but the sidecar's
 * currently-installed bundle differs. The adapter MUST refuse to resume —
 * re-evaluation may produce a different decision that the operator did not
 * see when they approved.
 *
 * Raised by `mapGrpcStatusToError` (SLICE 5) when a release / reserve trip
 * surfaces gRPC `FAILED_PRECONDITION` with the reason-code metadata field
 * set to `BUNDLE_HOT_RELOADED`. When the bundle hashes are not carried in
 * trailer metadata the constructor receives empty strings; adapters should
 * treat that as "hashes unavailable" and re-fetch from the handshake cache.
 */
export class ApprovalBundleHotReloadedError extends SpendGuardError {
  override name = "ApprovalBundleHotReloadedError";
  readonly originalBundleHash: string;
  readonly currentBundleHash: string;
  constructor(
    message: string,
    init: { originalBundleHash: string; currentBundleHash: string },
    opts?: { cause?: unknown },
  ) {
    super(message, opts);
    this.originalBundleHash = init.originalBundleHash;
    this.currentBundleHash = init.currentBundleHash;
  }
}

/**
 * Adapter's `publish_effect` step failed to apply the mutation patch. The
 * adapter should call `client.safeConfirmApplyFailed(...)` to anchor the
 * rollback in the audit chain.
 *
 * Raised by `mapGrpcStatusToError` (SLICE 5) when a release / reserve trip
 * surfaces gRPC `FAILED_PRECONDITION` with the reason-code metadata field set
 * to `IDEMPOTENCY_CONFLICT` (replay of a release with a different request
 * body), `BUDGET_EXCEEDED` (reservation can no longer be released against the
 * current ledger state), or any unknown FAILED_PRECONDITION reason — the
 * latter is the conservative default so adapters never see a bare
 * `SpendGuardError` for FAILED_PRECONDITION trips.
 */
export class MutationApplyFailed extends SpendGuardError {
  override name = "MutationApplyFailed";
}

/**
 * Decision-time wrapper for arbitrary post-decision errors that don't fit the
 * `DecisionDenied` hierarchy. Provided as a discriminated subtype so adapters
 * that want a single `catch (e: SpendGuardDecisionError)` block work without
 * pattern-matching on the structural `DecisionDenied` chain.
 *
 * SLICE 4 may extend this with concrete subclasses; in SLICE 3 it exists only
 * as the discriminated-subtype contract the slice doc requires.
 */
export class SpendGuardDecisionError extends SpendGuardError {
  override name = "SpendGuardDecisionError";
}

/**
 * Thrown by `PricingLookup.usdMicrosForCall` when a token bucket has a
 * non-zero count but NO configured price for either the specific token kind
 * or the default kind.
 *
 * Fail-closed rationale: the previous behavior silently coerced the missing
 * price to `0`, booking a `$0` charge for the call and under-counting the
 * budget — exactly the under-charge failure mode the guardrail exists to
 * prevent. Unknown / new models (the most likely to be mispriced) were
 * precisely the ones that escaped accounting. Refusing loudly forces the
 * adapter to supply a price (or explicitly handle the gap) rather than leak
 * spend. Carries the offending `provider` / `model` / `tokenKind` so the
 * caller can pinpoint the missing pricing-table row.
 */
export class PricingMissingError extends SpendGuardError {
  override name = "PricingMissingError";
  readonly provider: string;
  readonly model: string;
  readonly tokenKind: string;
  constructor(
    args: { provider: string; model: string; tokenKind: string },
    opts?: { cause?: unknown },
  ) {
    super(
      `no price configured for provider=${JSON.stringify(args.provider)} model=${JSON.stringify(args.model)} tokenKind=${JSON.stringify(args.tokenKind)} (neither the specific kind nor the default kind has a price); refusing to charge $0 — supply a price for this model or handle PricingMissingError`,
      opts,
    );
    this.provider = args.provider;
    this.model = args.model;
    this.tokenKind = args.tokenKind;
  }
}
