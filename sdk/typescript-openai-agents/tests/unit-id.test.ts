// HARDEN_D05_UR_S02 — `unitId` option plumbing tests for the OpenAI Agents
// TS adapter `withSpendGuard` factory. Mirrors the LOCKED per-adapter
// TA-01 / TA-02 / TA-03 pattern from docs/specs/harden_d05_unit_ref/tests.md §2.1.
//
// SLICE 2 contract (additive only):
//   - TA-01: options interface accepts `unitId` at construction time.
//   - TA-02: reserve wire shape's `projectedClaims[0].unit.unitId` carries
//     the value verbatim through the PRE/POST bracket.
//   - TA-03: backward compat — adapter constructs without `unitId` and the
//     wire `BudgetClaim.unit.unitId` is undefined (substrate coerces to "").

import type { ReserveRequest } from "@spendguard/sdk";
import { describe, expect, it } from "vitest";
import type { SpendGuardAgentsOptions } from "../src/options.js";
import { runContext } from "../src/runContext.js";
import { withSpendGuard } from "../src/withSpendGuard.js";
import { makeMockClient } from "./_support/mockClient.js";
import { makeMockInnerModel, makeRequest } from "./_support/mockInnerModel.js";

const TENANT_ID = "tenant-unitid-test";
const RUN_ID = "11111111-2222-3333-4444-555555555555";
const UNIT_ID_FIXTURE = "550e8400-e29b-41d4-a716-446655440000";

describe("withSpendGuard — HARDEN_D05_UR `unitId` option (TA-01..03)", () => {
  it("TA-01 — options interface accepts `unitId` at construction time", () => {
    const { client } = makeMockClient();
    const inner = makeMockInnerModel({ model: "gpt-4o-mini" });
    const opts: SpendGuardAgentsOptions = {
      client,
      tenantId: TENANT_ID,
      unitId: UNIT_ID_FIXTURE,
    };
    const wrapped = withSpendGuard(inner, opts);
    expect(wrapped).toBeDefined();
    expect(typeof wrapped.getResponse).toBe("function");
  });

  it("TA-02 — `unitId` threads to wire `BudgetClaim.unit.unitId` verbatim", async () => {
    const mock = makeMockClient();
    const inner = makeMockInnerModel({ model: "gpt-4o-mini" });
    const wrapped = withSpendGuard(inner, {
      client: mock.client,
      tenantId: TENANT_ID,
      unitId: UNIT_ID_FIXTURE,
    });

    await runContext({ runId: RUN_ID }, () => wrapped.getResponse(makeRequest()));

    expect(mock.reserve).toHaveBeenCalledTimes(1);
    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims).toHaveLength(1);
    expect(req.projectedClaims[0]?.unit.unitId).toBe(UNIT_ID_FIXTURE);
    expect(req.projectedClaims[0]?.unit.unit).toBe("USD_MICROS");
  });

  it("TA-03 — backward compat: no `unitId` → wire `unit.unitId` is undefined", async () => {
    const mock = makeMockClient();
    const inner = makeMockInnerModel({ model: "gpt-4o-mini" });
    const wrapped = withSpendGuard(inner, {
      client: mock.client,
      tenantId: TENANT_ID,
    });

    await runContext({ runId: RUN_ID }, () => wrapped.getResponse(makeRequest()));

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims[0]?.unit.unitId).toBeUndefined();
    expect(req.projectedClaims[0]?.unit.unit).toBe("USD_MICROS");
  });

  it("TA-02b — distinct adapter instances with different `unitId`s thread independently", async () => {
    const mockA = makeMockClient();
    const mockB = makeMockClient();
    const innerA = makeMockInnerModel({ model: "gpt-4o-mini" });
    const innerB = makeMockInnerModel({ model: "gpt-4o-mini" });
    const wA = withSpendGuard(innerA, {
      client: mockA.client,
      tenantId: TENANT_ID,
      unitId: UNIT_ID_FIXTURE,
    });
    const wB = withSpendGuard(innerB, {
      client: mockB.client,
      tenantId: TENANT_ID,
      unitId: "ffffffff-eeee-dddd-cccc-bbbbbbbbbbbb",
    });

    await runContext({ runId: RUN_ID }, () => wA.getResponse(makeRequest()));
    await runContext({ runId: RUN_ID }, () => wB.getResponse(makeRequest()));

    const reqA = mockA.reserve.mock.calls[0]?.[0] as ReserveRequest;
    const reqB = mockB.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(reqA.projectedClaims[0]?.unit.unitId).toBe(UNIT_ID_FIXTURE);
    expect(reqB.projectedClaims[0]?.unit.unitId).toBe("ffffffff-eeee-dddd-cccc-bbbbbbbbbbbb");
  });
});
