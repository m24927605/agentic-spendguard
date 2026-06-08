// HARDEN_D05_UR_S02 — `unitId` option plumbing tests for the Inngest
// AgentKit `wrapWithSpendGuard` factory. Mirrors the LOCKED per-adapter
// TA-01 / TA-02 / TA-03 pattern from docs/specs/harden_d05_unit_ref/tests.md §2.1.
//
// SLICE 2 contract (additive only):
//   - TA-01: options interface accepts `unitId` at construction time.
//   - TA-02: reserve wire shape's `projectedClaims[0].unit.unitId` carries
//     the value verbatim through the bracket's DEFAULT claim projection.
//   - TA-03: backward compat — adapter constructs without `unitId` and the
//     wire `BudgetClaim.unit.unitId` is undefined (substrate coerces to "").
//
// Note: `claimEstimator` always wins per the locked options contract (Python
// parity "explicit non-null wins"). These tests therefore drive the
// DEFAULT-claim projection path by omitting `claimEstimator` — that is the
// only path `unitId` is threaded automatically.

import type { ReserveRequest } from "@spendguard/sdk";
import { describe, expect, it } from "vitest";
import type { WrapWithSpendGuardOptions } from "../src/options.js";
import { wrapWithSpendGuard } from "../src/wrapWithSpendGuard.js";
import { makeMockStepAi, makeRuntimeCtx } from "./_support/mockAgentKit.js";
import { makeMockClient } from "./_support/mockClient.js";

const TENANT_ID = "tenant-unitid-test";
const STEP_ID = "step-unitid-test";
const UNIT_ID_FIXTURE = "550e8400-e29b-41d4-a716-446655440000";

describe("wrapWithSpendGuard — HARDEN_D05_UR `unitId` option (TA-01..03)", () => {
  it("TA-01 — options interface accepts `unitId` at construction time", () => {
    const mock = makeMockClient(TENANT_ID);
    const { stepAi } = makeMockStepAi();
    const opts: WrapWithSpendGuardOptions = {
      tenantId: TENANT_ID,
      unitId: UNIT_ID_FIXTURE,
    };
    const sg = wrapWithSpendGuard(stepAi, mock.client, opts);
    expect(sg).toBeDefined();
    expect(typeof sg.infer).toBe("function");
  });

  it("TA-02 — `unitId` threads to wire `BudgetClaim.unit.unitId` verbatim on default claim", async () => {
    const mock = makeMockClient(TENANT_ID);
    const { stepAi } = makeMockStepAi();
    const sg = wrapWithSpendGuard(stepAi, mock.client, {
      tenantId: TENANT_ID,
      unitId: UNIT_ID_FIXTURE,
    });

    await sg.infer(
      "test-step",
      { model: "gpt-4o-mini", body: {} },
      makeRuntimeCtx({
        step: { id: STEP_ID, attempt: 0 },
      }),
    );

    expect(mock.reserve).toHaveBeenCalledTimes(1);
    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims).toHaveLength(1);
    expect(req.projectedClaims[0]?.unit.unitId).toBe(UNIT_ID_FIXTURE);
    expect(req.projectedClaims[0]?.unit.unit).toBe("USD_MICROS");
  });

  it("TA-03 — backward compat: no `unitId` → wire `unit.unitId` is undefined", async () => {
    const mock = makeMockClient(TENANT_ID);
    const { stepAi } = makeMockStepAi();
    const sg = wrapWithSpendGuard(stepAi, mock.client, { tenantId: TENANT_ID });

    await sg.infer(
      "test-step",
      { model: "gpt-4o-mini", body: {} },
      makeRuntimeCtx({
        step: { id: STEP_ID, attempt: 0 },
      }),
    );

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims[0]?.unit.unitId).toBeUndefined();
    expect(req.projectedClaims[0]?.unit.unit).toBe("USD_MICROS");
  });

  it("TA-02b — distinct adapter instances with different `unitId`s thread independently", async () => {
    const mockA = makeMockClient(TENANT_ID);
    const mockB = makeMockClient(TENANT_ID);
    const sgA = wrapWithSpendGuard(makeMockStepAi().stepAi, mockA.client, {
      tenantId: TENANT_ID,
      unitId: UNIT_ID_FIXTURE,
    });
    const sgB = wrapWithSpendGuard(makeMockStepAi().stepAi, mockB.client, {
      tenantId: TENANT_ID,
      unitId: "ffffffff-eeee-dddd-cccc-bbbbbbbbbbbb",
    });

    await sgA.infer(
      "test-step",
      { model: "gpt-4o-mini", body: {} },
      makeRuntimeCtx({
        step: { id: STEP_ID, attempt: 0 },
      }),
    );
    await sgB.infer(
      "test-step",
      { model: "gpt-4o-mini", body: {} },
      makeRuntimeCtx({
        step: { id: STEP_ID, attempt: 0 },
      }),
    );

    const reqA = mockA.reserve.mock.calls[0]?.[0] as ReserveRequest;
    const reqB = mockB.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(reqA.projectedClaims[0]?.unit.unitId).toBe(UNIT_ID_FIXTURE);
    expect(reqB.projectedClaims[0]?.unit.unitId).toBe("ffffffff-eeee-dddd-cccc-bbbbbbbbbbbb");
  });
});
