// HARDEN_D05_WI — BudgetClaim.windowInstanceId wire-shape tests.
//
// Mirror of HARDEN_D05_UR's `unit-id-wire.test.ts`: the substrate previously
// hardcoded `windowInstanceId: ""` when mapping `projectedClaims` onto the
// `DecisionRequest` proto, so no TS adapter could ever satisfy the ledger's
// `claim[N].window_instance_id empty` validator. The cure is the same
// additive broadening: `BudgetClaim.windowInstanceId?: string`, threaded as
// `claim.windowInstanceId ?? ""`.
//
// Inventory:
//   WI-WS-01 windowInstanceId provided → wire shows UUID verbatim
//   WI-WS-02 windowInstanceId omitted → wire shows "" (backward compat)
//   WI-WS-03 3-claim reserve with distinct windowInstanceIds → all verbatim

import { describe, expect, it } from "vitest";

import type { DecisionRequest as ProtoDecisionRequest } from "../src/_proto/spendguard/sidecar_adapter/v1/adapter.js";
import { SpendGuardClient } from "../src/index.js";
import type { ReserveRequest } from "../src/index.js";
import { MockSidecar } from "./_support/mockSidecar.js";

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

function allowResponse(suffix: string) {
  return {
    decisionId: `mock-d-${suffix}`,
    auditDecisionEventId: `mock-audit-${suffix}`,
    decision: 1, // CONTINUE
    reasonCodes: ["mock_allow"],
    matchedRuleIds: [],
    mutationPatchJson: "",
    effectHash: new Uint8Array(),
    ledgerTransactionId: `mock-tx-${suffix}`,
    reservationIds: [`mock-reservation-${suffix}`],
    ttlExpiresAt: { seconds: "0", nanos: 0 },
    approvalRequestId: "",
    approverRole: "",
    terminal: false,
    runCodeTriggered: "",
  };
}

describe("HARDEN_D05_WI — BudgetClaim.windowInstanceId wire threading", () => {
  it("WI-WS-01: windowInstanceId UUID threads to claim.windowInstanceId verbatim on reserve()", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return allowResponse("wi1");
      },
    });
    try {
      const wiUuid = "55555555-5555-4555-8555-555555555555";
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      await client.reserve(
        reserveReq({
          projectedClaims: [
            {
              scopeId: "tenant/test/global",
              amountAtomic: "1000",
              unit: { unit: "USD_MICROS", denomination: -6 },
              windowInstanceId: wiUuid,
            },
          ],
        }),
      );
      const dr = captured as unknown as ProtoDecisionRequest;
      expect(dr.inputs?.projectedClaims?.length).toBe(1);
      expect(dr.inputs?.projectedClaims?.[0]?.windowInstanceId).toBe(wiUuid);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it('WI-WS-02: omitting windowInstanceId sends "" on the wire (backward compat)', async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return allowResponse("wi2");
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      await client.reserve(reserveReq()); // no windowInstanceId in projectedClaims
      const dr = captured as unknown as ProtoDecisionRequest;
      // `claim.windowInstanceId ?? ""` MUST surface as empty string, NOT
      // undefined — proto3 string fields default to "" so either path
      // serializes identically; the ledger's "empty" rejection semantics are
      // unchanged for legacy callers.
      expect(dr.inputs?.projectedClaims?.[0]?.windowInstanceId).toBe("");
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("WI-WS-03: 3 claims with distinct windowInstanceIds all land verbatim", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return allowResponse("wi3");
      },
    });
    try {
      const wA = "11111111-1111-4111-8111-111111111111";
      const wB = "22222222-2222-4222-8222-222222222222";
      const wC = "33333333-3333-4333-8333-333333333333";
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      await client.reserve(
        reserveReq({
          projectedClaims: [
            {
              scopeId: "tenant/test/global",
              amountAtomic: "100",
              unit: { unit: "USD_MICROS", denomination: -6 },
              windowInstanceId: wA,
            },
            {
              scopeId: "tenant/test/run",
              amountAtomic: "200",
              unit: { unit: "OUTPUT_TOKENS", denomination: 0 },
              windowInstanceId: wB,
            },
            {
              scopeId: "tenant/test/step",
              amountAtomic: "300",
              unit: { unit: "ACU", denomination: 0 },
              windowInstanceId: wC,
            },
          ],
        }),
      );
      const dr = captured as unknown as ProtoDecisionRequest;
      const claims = dr.inputs?.projectedClaims ?? [];
      expect(claims.length).toBe(3);
      expect(claims[0]?.windowInstanceId).toBe(wA);
      expect(claims[1]?.windowInstanceId).toBe(wB);
      expect(claims[2]?.windowInstanceId).toBe(wC);
      await client.close();
    } finally {
      await mock.close();
    }
  });
});
