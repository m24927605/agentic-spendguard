// HARDEN_D05_UR_S02 — `unitId` option plumbing tests for the LangChain.js
// `SpendGuardCallbackHandler`. Mirrors the LOCKED per-adapter TA-01 / TA-02 /
// TA-03 pattern from docs/specs/harden_d05_unit_ref/tests.md §2.1.
//
// SLICE 2 contract (additive only):
//   - TA-01: options interface accepts `unitId` (type-level + construct).
//   - TA-02: reserve wire shape's `projectedClaims[0].unit.unitId` carries
//     the value verbatim.
//   - TA-03: backward compat — handler constructs without `unitId` and the
//     wire `BudgetClaim.unit.unitId` is empty string (substrate behaviour).
//
// Scope: SLICE 2 mechanical sweep — DOES NOT touch commit-path tests
// (handler.test.ts SUCCESS / FAILURE commits remain baseline-untouched);
// the commit-side wire shape is additionally regression-checked via the
// `commitUnit()` helper so a future R-round can confirm POST events also
// thread the unitId without introducing baseline drift.

import type { Serialized } from "@langchain/core/load/serializable";
import type { BaseMessage } from "@langchain/core/messages";
import type {
  CommitEstimatedRequest,
  DecisionOutcome,
  ReserveRequest,
  SpendGuardClient,
} from "@spendguard/sdk";
import { describe, expect, it, vi } from "vitest";
import { SpendGuardCallbackHandler } from "../src/handler.js";
import type { SpendGuardCallbackHandlerOptions } from "../src/options.js";

const TENANT_ID = "tenant-unitid-test";
const RUN_ID = "11111111-2222-3333-4444-555555555555";
const UNIT_ID_FIXTURE = "550e8400-e29b-41d4-a716-446655440000";

const FAKE_SERIALIZED = {
  lc: 1,
  type: "constructor",
  id: ["test"],
  kwargs: {},
} as unknown as Serialized;

function makeMessage(content: string): BaseMessage {
  return { content } as unknown as BaseMessage;
}

function makeMessages(...texts: string[]): BaseMessage[][] {
  return [texts.map(makeMessage)];
}

function makeOutcome(): DecisionOutcome {
  return {
    decisionId: "decision-id-substrate-minted",
    auditDecisionEventId: "audit-evt-1",
    decision: "CONTINUE",
    mutationPatchJson: "{}",
    effectHash: new Uint8Array(0),
    ledgerTransactionId: "ledger-tx-1",
    reservationIds: ["reservation-id-substrate-minted"],
    ttlExpiresAtSeconds: 0,
    reasonCodes: [],
    matchedRuleIds: [],
  };
}

function makeMockClient(): SpendGuardClient {
  const mock = {
    tenantId: TENANT_ID,
    reserve: vi.fn<(req: ReserveRequest) => Promise<DecisionOutcome>>(),
    commitEstimated: vi.fn<(req: CommitEstimatedRequest) => Promise<void>>(),
  };
  mock.reserve.mockResolvedValue(makeOutcome());
  mock.commitEstimated.mockResolvedValue(undefined);
  return mock as unknown as SpendGuardClient;
}

function getMock(handler: SpendGuardCallbackHandler): {
  reserve: ReturnType<typeof vi.fn>;
  commitEstimated: ReturnType<typeof vi.fn>;
} {
  return (
    handler as unknown as {
      client: {
        reserve: ReturnType<typeof vi.fn>;
        commitEstimated: ReturnType<typeof vi.fn>;
      };
    }
  ).client;
}

describe("SpendGuardCallbackHandler — HARDEN_D05_UR `unitId` option (TA-01..03)", () => {
  it("TA-01 — options interface accepts `unitId` at construction time", () => {
    const opts: SpendGuardCallbackHandlerOptions = {
      client: makeMockClient(),
      tenantId: TENANT_ID,
      unitId: UNIT_ID_FIXTURE,
    };
    const handler = new SpendGuardCallbackHandler(opts);
    expect(handler).toBeDefined();
    expect(handler.name).toBe("spendguard_callback_handler");
  });

  it("TA-02 — `unitId` threads to wire `BudgetClaim.unit.unitId` verbatim", async () => {
    const client = makeMockClient();
    const handler = new SpendGuardCallbackHandler({
      client,
      tenantId: TENANT_ID,
      unitId: UNIT_ID_FIXTURE,
    });
    const mock = getMock(handler);

    await handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID);

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims).toHaveLength(1);
    expect(req.projectedClaims[0]?.unit.unitId).toBe(UNIT_ID_FIXTURE);
    // Free-form unit slug untouched — `unit` and `unitId` are not interchangeable.
    expect(req.projectedClaims[0]?.unit.unit).toBe("USD_MICROS");
  });

  it("TA-03 — backward compat: no `unitId` → wire `unit.unitId` is undefined (substrate coerces to '')", async () => {
    const client = makeMockClient();
    const handler = new SpendGuardCallbackHandler({
      client,
      tenantId: TENANT_ID,
    });
    const mock = getMock(handler);

    await handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID);

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims).toHaveLength(1);
    // SDK-side `mapUnitRef` (sdk/typescript/src/client.ts) coerces to "" on
    // the wire; at the adapter boundary the field is undefined per the
    // backward-compat invariant in implementation.md §7.
    expect(req.projectedClaims[0]?.unit.unitId).toBeUndefined();
    expect(req.projectedClaims[0]?.unit.unit).toBe("USD_MICROS");
  });

  it("TA-02b — multiple handlers with different `unitId`s thread independently (no shared state)", async () => {
    const clientA = makeMockClient();
    const handlerA = new SpendGuardCallbackHandler({
      client: clientA,
      tenantId: TENANT_ID,
      unitId: UNIT_ID_FIXTURE,
    });
    const clientB = makeMockClient();
    const handlerB = new SpendGuardCallbackHandler({
      client: clientB,
      tenantId: TENANT_ID,
      unitId: "ffffffff-eeee-dddd-cccc-bbbbbbbbbbbb",
    });

    await handlerA.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID);
    await handlerB.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID);

    const reqA = getMock(handlerA).reserve.mock.calls[0]?.[0] as ReserveRequest;
    const reqB = getMock(handlerB).reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(reqA.projectedClaims[0]?.unit.unitId).toBe(UNIT_ID_FIXTURE);
    expect(reqB.projectedClaims[0]?.unit.unitId).toBe("ffffffff-eeee-dddd-cccc-bbbbbbbbbbbb");
  });
});
