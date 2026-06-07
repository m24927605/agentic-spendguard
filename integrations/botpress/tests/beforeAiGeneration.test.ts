// beforeAiGeneration.test.ts — unit suite covering B01–B07 per tests.md §2.2.

import { RuntimeError } from "@botpress/sdk";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  type SpendGuardHandleStash,
  runBeforeAiGeneration,
} from "../src/hooks/beforeAiGeneration.js";
import { SpendGuardReservation } from "../src/reservation.js";
import { makeConfig, makeHookInput } from "./_fixtures.js";
import { type MockSidecarHandle, setupMockSidecar } from "./_mockSidecar.js";

describe("beforeAiGeneration hook (B01–B07)", () => {
  let mock: MockSidecarHandle;
  let warnSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(async () => {
    mock = await setupMockSidecar();
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });

  afterEach(async () => {
    await mock.close();
    warnSpy.mockRestore();
  });

  test("B01 test_allow_returns_data_with_handle_stash", async () => {
    const configuration = makeConfig({ sidecarUrl: mock.url });
    const out = await runBeforeAiGeneration({
      input: makeHookInput(),
      configuration,
    });
    const stashed = (out.data as SpendGuardHandleStash)._spendguardHandle;
    expect(stashed).toBeDefined();
    expect(stashed?.reservationId.length).toBeGreaterThan(0);
  });

  test("B02 test_deny_throws_runtime_error_with_budget_denied_code", async () => {
    mock.setOptions({ verdict: "DENY" });
    const configuration = makeConfig({ sidecarUrl: mock.url });
    let caught: unknown;
    try {
      await runBeforeAiGeneration({
        input: makeHookInput(),
        configuration,
      });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(RuntimeError);
    expect((caught as Error).message).toContain("denied");
  });

  test("B03 test_deny_no_upstream", async () => {
    // Critical INV-1: DENY produces zero trace POSTs (proxy for upstream
    // HTTP in the unit tier; in the integration tier, this becomes an
    // assertion against the mock OpenAI counting stub).
    mock.setOptions({ verdict: "DENY" });
    const configuration = makeConfig({ sidecarUrl: mock.url });
    await expect(
      runBeforeAiGeneration({ input: makeHookInput(), configuration }),
    ).rejects.toBeInstanceOf(RuntimeError);
    expect(mock.hits.trace).toBe(0);
  });

  test("B04 test_degrade_throws_runtime_error_budget_degraded", async () => {
    mock.setOptions({ verdict: "DEGRADE" });
    const configuration = makeConfig({ sidecarUrl: mock.url });
    let caught: unknown;
    try {
      await runBeforeAiGeneration({
        input: makeHookInput(),
        configuration,
      });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(RuntimeError);
    expect((caught as Error).message).toContain("degraded");
  });

  test("B05 test_reentrant_safety", async () => {
    // Two concurrent calls for the same conversation produce distinct
    // handles. INV-10.
    const configuration = makeConfig({ sidecarUrl: mock.url });
    const [a, b] = await Promise.all([
      runBeforeAiGeneration({ input: makeHookInput(), configuration }),
      runBeforeAiGeneration({ input: makeHookInput(), configuration }),
    ]);
    const ha = (a.data as SpendGuardHandleStash)._spendguardHandle;
    const hb = (b.data as SpendGuardHandleStash)._spendguardHandle;
    expect(ha).toBeDefined();
    expect(hb).toBeDefined();
    expect(ha?.reservationId).not.toBe(hb?.reservationId);
    expect(ha?.stepId).not.toBe(hb?.stepId);
  });

  test("B06 test_config_error_throws_budget_config", async () => {
    const reservation = new SpendGuardReservation(makeConfig({ sidecarUrl: mock.url }));
    // Force a SpendGuardConfigError by hand-constructing reservation, then
    // overriding the run to throw a config-shaped error. Easier path:
    // synthesise a hook input whose configuration is structurally invalid
    // (missing tenantId via Object.create) — the runBeforeAiGeneration
    // factory path will throw SpendGuardConfigError → RuntimeError.
    void reservation;
    const configuration = makeConfig({ sidecarUrl: mock.url, tenantId: "" });
    let caught: unknown;
    try {
      await runBeforeAiGeneration({
        input: makeHookInput(),
        configuration,
      });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(RuntimeError);
    expect((caught as Error).message).toContain("config");
  });

  test("B07 test_strict_ordering_reserve_before_data_return", async () => {
    // The /v1/decision POST timestamp must precede the hook return point.
    const configuration = makeConfig({ sidecarUrl: mock.url });
    const start = performance.now();
    const out = await runBeforeAiGeneration({
      input: makeHookInput(),
      configuration,
    });
    const end = performance.now();
    const decisionEv = mock.events.find((e) => e.kind === "decision");
    expect(decisionEv).toBeDefined();
    // Decision timestamp falls inside the (start, end) window.
    expect(decisionEv!.timestamp).toBeGreaterThanOrEqual(start);
    expect(decisionEv!.timestamp).toBeLessThanOrEqual(end);
    // Handle was minted from the decision response.
    const handle = (out.data as SpendGuardHandleStash)._spendguardHandle;
    expect(handle?.reservationId.length).toBeGreaterThan(0);
  });
});
