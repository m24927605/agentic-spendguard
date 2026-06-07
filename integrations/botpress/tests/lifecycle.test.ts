// lifecycle.test.ts — unit suite covering L01–L05 per tests.md §2.5.

import { RuntimeError } from "@botpress/sdk";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { ConfigurationSchema } from "../src/config.js";
import { validateConfiguration } from "../src/lifecycle/validateConfiguration.js";
import { FIXTURE_TENANT_ID, makeConfig } from "./_fixtures.js";
import { type MockSidecarHandle, setupMockSidecar } from "./_mockSidecar.js";

describe("validateConfiguration (L01–L05)", () => {
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

  test("L01 test_validateConfiguration_issues_reserve_release_roundtrip", async () => {
    const configuration = makeConfig({ sidecarUrl: mock.url });
    await validateConfiguration({ configuration });
    expect(mock.hits.decision).toBe(1);
    expect(mock.hits.trace).toBe(1);
    // Trace body is the release (outcome=REJECTED) since the probe always
    // releases the reservation immediately.
    const traceEv = mock.events.find((e) => e.kind === "trace");
    expect(traceEv).toBeDefined();
    const body = traceEv?.body as unknown as Record<string, unknown>;
    expect(body.outcome).toBe("REJECTED");
  });

  test("L02 test_validate_bad_sidecar_propagates", async () => {
    // Close the mock to make sidecar unreachable.
    await mock.close();
    const configuration = makeConfig({ sidecarUrl: "http://127.0.0.1:1" });
    let caught: unknown;
    try {
      await validateConfiguration({ configuration });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(RuntimeError);
    expect((caught as Error).message.toLowerCase()).toContain("sidecar unreachable");
    // Re-open a new mock so afterEach's close on the original (already closed)
    // doesn't fail.
    mock = await setupMockSidecar();
  });

  test("L03 test_validate_zod_rejects_empty_budget_id", () => {
    const result = ConfigurationSchema.safeParse({
      sidecarUrl: "http://localhost:8443",
      spendguardBudgetId: "",
      spendguardWindowInstanceId: "win",
      upstreamProvider: "openai",
      tenantId: FIXTURE_TENANT_ID,
    });
    expect(result.success).toBe(false);
  });

  test("L04 test_validate_rejects_unsupported_upstream", () => {
    const result = ConfigurationSchema.safeParse({
      sidecarUrl: "http://localhost:8443",
      spendguardBudgetId: "b",
      spendguardWindowInstanceId: "w",
      upstreamProvider: "cohere",
      tenantId: FIXTURE_TENANT_ID,
    });
    expect(result.success).toBe(false);
  });

  test("L05 test_validate_sidecar_deny_propagates_as_budget_denied", async () => {
    mock.setOptions({ verdict: "DENY", denyReasonCodes: ["BUDGET_EXCEEDED"] });
    const configuration = makeConfig({ sidecarUrl: mock.url });
    let caught: unknown;
    try {
      await validateConfiguration({ configuration });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(RuntimeError);
    expect((caught as Error).message).toContain("denied");
  });
});
