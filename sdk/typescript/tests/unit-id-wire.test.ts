// HARDEN_D05_UR SLICE 1 — UnitRef.unitId wire-shape tests.
//
// design.md §2.2 LOCKS the wire-threading contract: `mapUnitRef` returns
// `unit.unitId ?? ""`. This test file is the wire-shape gate for the substrate
// change — it stands up a real `MockSidecar`, drives `reserve()` and
// `commitEstimated()` through the UDS UDP path, and asserts the captured
// `DecisionRequest` / `TraceEvent` proto carry the caller's `unitId` verbatim
// (or `""` for the backward-compat path).
//
// Inventory (tests.md §1.2):
//   U-WS-01 unitId provided → wire shows UUID verbatim
//   U-WS-02 unitId omitted → wire shows ""
//   U-WS-03 3-claim reserve, each with distinct unitId → all land verbatim
//   U-WS-04 projectedUnit also threads unitId
//   U-WS-05 commitEstimated wire path also threads unitId
//   U-WS-06 explicit "" preserved as "" (no coercion to undefined)
//
// Cross-adapter smoke (tests.md §3):
//   XA-01 construct client → handshake() → reserve() with unitId → mock
//          sidecar sees the UUID end-to-end (protects against future regression
//          of the substrate from any caller that mixes barrel + subpath imports).
//
// Companion locked-surface tests (U-LS-01 + U-LS-02) live in
// `tests/locked-surface.test.ts` and cover the TYPE-LEVEL surface; this file
// covers the RUNTIME-LEVEL surface (the wire path the ledger reads).

import { afterEach, describe, expect, it } from "vitest";

import type { DecisionRequest as ProtoDecisionRequest } from "../src/_proto/spendguard/sidecar_adapter/v1/adapter.js";
import type { TraceEvent as ProtoTraceEvent } from "../src/_proto/spendguard/sidecar_adapter/v1/adapter.js";
import { TraceEventAck_Status } from "../src/_proto/spendguard/sidecar_adapter/v1/adapter.js";
import { SpendGuardClient } from "../src/index.js";
import type { CommitEstimatedRequest, ReserveRequest } from "../src/index.js";
import { MockSidecar } from "./_support/mockSidecar.js";

// Restore SPENDGUARD_* env between tests so a stray var doesn't leak.
const ENV_KEYS = [
  "SPENDGUARD_SOCKET_PATH",
  "SPENDGUARD_SIDECAR_UDS",
  "SPENDGUARD_TENANT_ID",
  "SPENDGUARD_DISABLE",
  "SPENDGUARD_UNIT_ID",
] as const;
const savedEnv: Record<string, string | undefined> = {};
for (const k of ENV_KEYS) savedEnv[k] = process.env[k];
afterEach(() => {
  for (const k of ENV_KEYS) {
    if (savedEnv[k] === undefined) delete process.env[k];
    else process.env[k] = savedEnv[k];
  }
});

/** Canonical reserve request shape; per-test overrides supplied as needed. */
function reserveReq(overrides: Partial<ReserveRequest> = {}): ReserveRequest {
  return {
    trigger: "LLM_CALL_PRE",
    runId: "run-1",
    stepId: "step-1",
    llmCallId: "llm-1",
    decisionId: "d-1",
    route: "openai|gpt-4o-mini",
    projectedClaims: [
      {
        scopeId: "tenant/test/global",
        amountAtomic: "1000",
        unit: { unit: "USD_MICROS", denomination: -6 },
      },
    ],
    idempotencyKey: "sg-0123456789abcdef",
    ...overrides,
  };
}

/** Canonical commitEstimated request shape; per-test overrides supplied as needed. */
function commitReq(overrides: Partial<CommitEstimatedRequest> = {}): CommitEstimatedRequest {
  return {
    runId: "run-1",
    stepId: "step-1",
    llmCallId: "llm-1",
    decisionId: "d-1",
    reservationId: "mock-reservation-1",
    estimatedAmountAtomic: "500",
    unit: { unit: "USD_MICROS", denomination: -6 },
    pricing: {
      pricingVersion: "v2026.05.09-1",
      pricingHash: new Uint8Array([0x01, 0x02]),
    },
    providerEventId: "pe-1",
    outcome: "SUCCESS",
    ...overrides,
  };
}

// ── U-WS-01..06 — wire-shape gate ────────────────────────────────────────

describe("HARDEN_D05_UR §2.2 — UnitRef.unitId wire threading", () => {
  it("U-WS-01: unitId UUID threads to BudgetClaim.unit.unitId verbatim on reserve()", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        // Default CONTINUE response so the assertion below can run.
        return {
          decisionId: "mock-d-1",
          auditDecisionEventId: "mock-audit-1",
          decision: 1, // CONTINUE
          reasonCodes: ["mock_allow"],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "mock-tx-1",
          reservationIds: ["mock-reservation-1"],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      await client.reserve(
        reserveReq({
          projectedClaims: [
            {
              scopeId: "tenant/test/global",
              amountAtomic: "1000",
              unit: {
                unit: "USD_MICROS",
                denomination: -6,
                unitId: "550e8400-e29b-41d4-a716-446655440000",
              },
            },
          ],
        }),
      );
      expect(captured).not.toBeNull();
      const dr = captured as unknown as ProtoDecisionRequest;
      expect(dr.inputs?.projectedClaims?.length).toBe(1);
      const claim0 = dr.inputs?.projectedClaims?.[0];
      expect(claim0?.unit?.unitId).toBe("550e8400-e29b-41d4-a716-446655440000");
      // The free-form slug stays on `unitName` — the substrate change does NOT
      // alter that mapping (review-standards §10 anti-pattern guard).
      expect(claim0?.unit?.unitName).toBe("USD_MICROS");
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it('U-WS-02: omitting unitId sends "" on the wire (backward compat)', async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return {
          decisionId: "mock-d-2",
          auditDecisionEventId: "mock-audit-2",
          decision: 1,
          reasonCodes: [],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "mock-tx-2",
          reservationIds: ["mock-reservation-2"],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      await client.reserve(reserveReq()); // no unitId in projectedClaims
      const dr = captured as unknown as ProtoDecisionRequest;
      // The substrate `unit.unitId ?? ""` MUST surface as empty string, NOT
      // undefined — the ledger validator's "empty" check is the LOCKED
      // semantics (design.md §1.4) and proto3 string fields default to "" so
      // either path serializes identically on the wire.
      expect(dr.inputs?.projectedClaims?.[0]?.unit?.unitId).toBe("");
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("U-WS-03: 3 claims with distinct unitIds all land verbatim on the wire", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return {
          decisionId: "mock-d-3",
          auditDecisionEventId: "mock-audit-3",
          decision: 1,
          reasonCodes: [],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "mock-tx-3",
          reservationIds: ["r-a", "r-b", "r-c"],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const uA = "11111111-1111-4111-8111-111111111111";
      const uB = "22222222-2222-4222-8222-222222222222";
      const uC = "33333333-3333-4333-8333-333333333333";
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      await client.reserve(
        reserveReq({
          projectedClaims: [
            {
              scopeId: "tenant/test/global",
              amountAtomic: "100",
              unit: { unit: "USD_MICROS", denomination: -6, unitId: uA },
            },
            {
              scopeId: "tenant/test/run",
              amountAtomic: "200",
              unit: { unit: "OUTPUT_TOKENS", denomination: 0, unitId: uB },
            },
            {
              scopeId: "tenant/test/step",
              amountAtomic: "300",
              unit: { unit: "ACU", denomination: 0, unitId: uC },
            },
          ],
        }),
      );
      const dr = captured as unknown as ProtoDecisionRequest;
      const claims = dr.inputs?.projectedClaims ?? [];
      expect(claims.length).toBe(3);
      expect(claims[0]?.unit?.unitId).toBe(uA);
      expect(claims[1]?.unit?.unitId).toBe(uB);
      expect(claims[2]?.unit?.unitId).toBe(uC);
      // Free-form slugs preserved per-claim (no cross-claim leakage).
      expect(claims[0]?.unit?.unitName).toBe("USD_MICROS");
      expect(claims[1]?.unit?.unitName).toBe("OUTPUT_TOKENS");
      expect(claims[2]?.unit?.unitName).toBe("ACU");
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("U-WS-04: projectedUnit on reserve() also threads unitId", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return {
          decisionId: "mock-d-4",
          auditDecisionEventId: "mock-audit-4",
          decision: 1,
          reasonCodes: [],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "mock-tx-4",
          reservationIds: ["mock-reservation-4"],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const projUuid = "abcdef00-0000-4000-8000-000000000999";
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      await client.reserve(
        reserveReq({
          projectedUnit: {
            unit: "USD_MICROS",
            denomination: -6,
            unitId: projUuid,
          },
        }),
      );
      const dr = captured as unknown as ProtoDecisionRequest;
      // §1.2 LOCKED: client.ts:1246 threads projectedUnit through mapUnitRef
      // too, so the fix lands here as well.
      expect(dr.inputs?.projectedUnit?.unitId).toBe(projUuid);
      expect(dr.inputs?.projectedUnit?.unitName).toBe("USD_MICROS");
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("U-WS-05: commitEstimated wire path also threads unitId", async () => {
    let captured: ProtoTraceEvent | null = null;
    const mock = await MockSidecar.start({
      onEmitTraceEvents: (event) => {
        captured = event;
        return { eventId: "ack-5", status: TraceEventAck_Status.ACCEPTED };
      },
    });
    try {
      const commitUuid = "fedcba00-0000-4000-8000-000000000777";
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      await client.commitEstimated(
        commitReq({
          unit: { unit: "USD_MICROS", denomination: -6, unitId: commitUuid },
        }),
      );
      const ev = captured as unknown as ProtoTraceEvent;
      // §1.2 LOCKED: client.ts:1356 (single-event LLM_CALL_POST path) also
      // calls mapUnitRef on req.unit; the fix lands here too.
      if (ev.payload.oneofKind === "llmCallPost") {
        expect(ev.payload.llmCallPost.unit?.unitId).toBe(commitUuid);
        expect(ev.payload.llmCallPost.unit?.unitName).toBe("USD_MICROS");
      } else {
        throw new Error(`payload oneofKind mismatch: ${ev.payload.oneofKind}`);
      }
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it('U-WS-06: explicit empty-string unitId preserved as "" (identity-preserving)', async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return {
          decisionId: "mock-d-6",
          auditDecisionEventId: "mock-audit-6",
          decision: 1,
          reasonCodes: [],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "mock-tx-6",
          reservationIds: ["mock-reservation-6"],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      await client.reserve(
        reserveReq({
          projectedClaims: [
            {
              scopeId: "tenant/test/global",
              amountAtomic: "1000",
              // Caller explicitly passes empty string — the substrate must NOT
              // coerce this to undefined or otherwise reshape it. Identity is
              // the contract; the sidecar still rejects "" (semantics unchanged
              // per design.md §1.2 / review-standards §1.2).
              unit: { unit: "USD_MICROS", denomination: -6, unitId: "" },
            },
          ],
        }),
      );
      const dr = captured as unknown as ProtoDecisionRequest;
      expect(dr.inputs?.projectedClaims?.[0]?.unit?.unitId).toBe("");
      await client.close();
    } finally {
      await mock.close();
    }
  });
});

// ── XA-01 — cross-adapter smoke ──────────────────────────────────────────
//
// design.md §3 / tests.md §3 require a smoke test that constructs a real
// `SpendGuardClient`, runs handshake() + reserve() against the mock sidecar,
// and asserts the wire shape carries the UUID end-to-end. This is the
// regression guard that protects the substrate change from future churn in
// the surrounding wrapping (cache / retry / otel — SLICE 8 added these around
// `reserve()`; XA-01 proves the unitId still survives the wrapping).

describe("HARDEN_D05_UR §3 — cross-adapter unitId smoke", () => {
  it("XA-01: client.handshake() + client.reserve() pass unitId end-to-end through wire", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return {
          decisionId: "mock-d-xa",
          auditDecisionEventId: "mock-audit-xa",
          decision: 1,
          reasonCodes: ["mock_allow"],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "mock-tx-xa",
          reservationIds: ["mock-reservation-xa"],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const ledgerUuid = "00000000-0000-4000-8000-000000000001";
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "tenant-xa",
      });
      // Mirror the adapter lifecycle: connect() → handshake() → reserve().
      // SLICE 8 wraps reserve() with cache / retry / otel; the unitId MUST
      // survive that wrapping verbatim.
      await client.connect();
      const outcome = await client.handshake();
      expect(outcome.sessionId).toBeTruthy();
      const decision = await client.reserve(
        reserveReq({
          projectedClaims: [
            {
              scopeId: "tenant/xa/global",
              amountAtomic: "2500",
              unit: { unit: "USD_MICROS", denomination: -6, unitId: ledgerUuid },
            },
          ],
        }),
      );
      expect(decision.decision).toBe("CONTINUE");
      // The wire assertion is the load-bearing part — mock sidecar saw the
      // UUID, proving the SDK substrate threads it through unchanged.
      const dr = captured as unknown as ProtoDecisionRequest;
      expect(dr.inputs?.projectedClaims?.[0]?.unit?.unitId).toBe(ledgerUuid);
      await client.close();
    } finally {
      await mock.close();
    }
  });
});
