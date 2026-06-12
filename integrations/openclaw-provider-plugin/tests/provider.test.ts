import { describe, expect, it } from "vitest";

import type { OpenClawSpendGuardOptions } from "../src/index.js";
import {
  OpenClawSpendGuardConfigError,
  VERSION,
  buildOpenClawReserveRequest,
  createSpendGuardOpenClawProvider,
} from "../src/index.js";

const upstream = {
  id: "upstream-openai-compatible",
  label: "Upstream OpenAI-compatible provider",
  auth: [],
};

const options = {
  client: {} as OpenClawSpendGuardOptions["client"],
  tenantId: "tenant_1",
  budgetId: "budget_1",
  windowInstanceId: "window_1",
  unitId: "usd_micros",
  pricing: {
    pricingVersion: "2026-06-12",
    pricingHash: new Uint8Array([1, 2, 3]),
  },
} satisfies OpenClawSpendGuardOptions;

describe("createSpendGuardOpenClawProvider skeleton", () => {
  it("exports the package version", () => {
    expect(VERSION).toBe("0.1.0-pre");
  });

  it("preserves upstream identity and catalog behavior", async () => {
    const provider = createSpendGuardOpenClawProvider(
      {
        ...upstream,
        catalog: { run: async () => ({ provider: { id: "upstream" } }) },
      },
      options,
    );

    expect(provider.id).toBe(upstream.id);
    const result = await provider.catalog?.run({});
    expect(result).toEqual({ provider: { id: "upstream" } });
  });

  it("requires the day-1 unit/window/pricing tuple", () => {
    expect(() =>
      createSpendGuardOpenClawProvider(upstream, {
        ...options,
        windowInstanceId: "",
      }),
    ).toThrow(OpenClawSpendGuardConfigError);

    expect(() =>
      createSpendGuardOpenClawProvider(upstream, {
        ...options,
        pricing: {
          pricingVersion: "",
          pricingHash: new Uint8Array([1]),
        },
      }),
    ).toThrow(OpenClawSpendGuardConfigError);
  });

  it("builds the locked reserve request shape", () => {
    const req = buildOpenClawReserveRequest(
      { messages: [{ role: "user", content: "ping" }] },
      { provider: "openai", modelId: "gpt-test" },
      {
        ...options,
        client: { reserve: async () => ({}) } as unknown as OpenClawSpendGuardOptions["client"],
        runIdProvider: () => "run_1",
      },
    );

    expect(req.trigger).toBe("LLM_CALL_PRE");
    expect(req.stepId).toBe("llm_call");
    expect(req.route).toBe("openclaw-provider");
    expect(req.runId).toBe("run_1");
    expect(req.projectedClaims.length).toBe(1);
    expect(req.projectedClaims[0]).toEqual({
      scopeId: "budget_1",
      amountAtomic: "3000",
      unit: { unit: "USD_MICROS", denomination: 1, unitId: "usd_micros" },
      windowInstanceId: "window_1",
    });
  });
});
