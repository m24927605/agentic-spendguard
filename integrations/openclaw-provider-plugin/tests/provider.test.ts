import { describe, expect, it } from "vitest";

import type { OpenClawSpendGuardOptions } from "../src/index.js";
import {
  OpenClawSpendGuardConfigError,
  OpenClawSpendGuardNotImplementedError,
  VERSION,
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

  it("preserves upstream identity while disabling unimplemented catalog dispatch", async () => {
    const provider = createSpendGuardOpenClawProvider(upstream, options);

    expect(provider.id).toBe(upstream.id);
    await expect(provider.catalog?.run({})).rejects.toThrow(OpenClawSpendGuardNotImplementedError);
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
});
