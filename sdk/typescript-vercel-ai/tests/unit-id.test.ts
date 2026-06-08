// HARDEN_D05_UR_S02 — `unitId` option plumbing tests for the Vercel AI SDK
// `createSpendGuardMiddleware` factory. Mirrors the LOCKED per-adapter
// TA-01 / TA-02 / TA-03 pattern from docs/specs/harden_d05_unit_ref/tests.md §2.1.
//
// SLICE 2 contract (additive only):
//   - TA-01: options interface accepts `unitId` at construction time.
//   - TA-02: reserve wire shape's `projectedClaims[0].unit.unitId` carries
//     the value verbatim through `transformParams`.
//   - TA-03: backward compat — middleware constructs without `unitId` and
//     the wire `BudgetClaim.unit.unitId` is undefined (substrate coerces to "").

import type {
  CommitEstimatedRequest,
  DecisionOutcome,
  ReserveRequest,
  SpendGuardClient,
} from "@spendguard/sdk";
import type { LanguageModelV1CallOptions } from "ai";
import { describe, expect, it, vi } from "vitest";
import { createSpendGuardMiddleware } from "../src/middleware.js";
import type { SpendGuardMiddlewareOptions } from "../src/options.js";

const TENANT_ID = "tenant-unitid-test";
const UNIT_ID_FIXTURE = "550e8400-e29b-41d4-a716-446655440000";

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

function makeParams(): LanguageModelV1CallOptions {
  return {
    inputFormat: "messages",
    mode: { type: "regular" },
    prompt: [{ role: "user", content: [{ type: "text", text: "hi" }] }],
  } as unknown as LanguageModelV1CallOptions;
}

function getReserve(opts: SpendGuardMiddlewareOptions): ReturnType<typeof vi.fn> {
  return (opts.client as unknown as { reserve: ReturnType<typeof vi.fn> }).reserve;
}

describe("createSpendGuardMiddleware — HARDEN_D05_UR `unitId` option (TA-01..03)", () => {
  it("TA-01 — options interface accepts `unitId` at construction time", () => {
    const opts: SpendGuardMiddlewareOptions = {
      client: makeMockClient(),
      tenantId: TENANT_ID,
      unitId: UNIT_ID_FIXTURE,
    };
    const middleware = createSpendGuardMiddleware(opts);
    expect(middleware).toBeDefined();
    expect(middleware.middlewareVersion).toBe("v1");
  });

  it("TA-02 — `unitId` threads to wire `BudgetClaim.unit.unitId` verbatim", async () => {
    const opts: SpendGuardMiddlewareOptions = {
      client: makeMockClient(),
      tenantId: TENANT_ID,
      unitId: UNIT_ID_FIXTURE,
    };
    const middleware = createSpendGuardMiddleware(opts);
    if (!middleware.transformParams) throw new Error("transformParams must be defined");
    await middleware.transformParams({ params: makeParams(), type: "generate" });

    const reserve = getReserve(opts);
    expect(reserve).toHaveBeenCalledTimes(1);
    const req = reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims).toHaveLength(1);
    expect(req.projectedClaims[0]?.unit.unitId).toBe(UNIT_ID_FIXTURE);
    expect(req.projectedClaims[0]?.unit.unit).toBe("USD_MICROS");
  });

  it("TA-03 — backward compat: no `unitId` → wire `unit.unitId` is undefined", async () => {
    const opts: SpendGuardMiddlewareOptions = {
      client: makeMockClient(),
      tenantId: TENANT_ID,
    };
    const middleware = createSpendGuardMiddleware(opts);
    if (!middleware.transformParams) throw new Error("transformParams must be defined");
    await middleware.transformParams({ params: makeParams(), type: "generate" });

    const reserve = getReserve(opts);
    const req = reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims[0]?.unit.unitId).toBeUndefined();
    expect(req.projectedClaims[0]?.unit.unit).toBe("USD_MICROS");
  });

  it("TA-02b — distinct middleware instances with different `unitId`s thread independently", async () => {
    const optsA: SpendGuardMiddlewareOptions = {
      client: makeMockClient(),
      tenantId: TENANT_ID,
      unitId: UNIT_ID_FIXTURE,
    };
    const optsB: SpendGuardMiddlewareOptions = {
      client: makeMockClient(),
      tenantId: TENANT_ID,
      unitId: "ffffffff-eeee-dddd-cccc-bbbbbbbbbbbb",
    };
    const mwA = createSpendGuardMiddleware(optsA);
    const mwB = createSpendGuardMiddleware(optsB);

    if (!mwA.transformParams) throw new Error("transformParams must be defined");
    if (!mwB.transformParams) throw new Error("transformParams must be defined");
    await mwA.transformParams({ params: makeParams(), type: "generate" });
    await mwB.transformParams({ params: makeParams(), type: "generate" });

    const reqA = getReserve(optsA).mock.calls[0]?.[0] as ReserveRequest;
    const reqB = getReserve(optsB).mock.calls[0]?.[0] as ReserveRequest;
    expect(reqA.projectedClaims[0]?.unit.unitId).toBe(UNIT_ID_FIXTURE);
    expect(reqB.projectedClaims[0]?.unit.unitId).toBe("ffffffff-eeee-dddd-cccc-bbbbbbbbbbbb");
  });
});
