import { describe, expect, it } from "vitest";

import type { OpenClawSpendGuardOptions } from "../src/index.js";
import { createSpendGuardOpenClawProvider } from "../src/index.js";

const outcome = {
  decisionId: "decision_1",
  auditDecisionEventId: "audit_1",
  decision: "CONTINUE",
  mutationPatchJson: "{}",
  effectHash: new Uint8Array([1]),
  ledgerTransactionId: "ledger_1",
  reservationIds: ["reservation_1"],
  ttlExpiresAtSeconds: 1,
  reasonCodes: [],
  matchedRuleIds: [],
} as const;

function optionsWithReserve(reserve: (req: unknown) => Promise<unknown>) {
  return {
    client: {
      reserve,
      commitEstimated: async () => {},
    } as unknown as OpenClawSpendGuardOptions["client"],
    tenantId: "tenant_1",
    budgetId: "budget_1",
    windowInstanceId: "window_1",
    unitId: "unit_1",
    pricing: {
      pricingVersion: "2026-06-12",
      pricingHash: new Uint8Array([1]),
    },
    runIdProvider: () => "run_1",
  } satisfies OpenClawSpendGuardOptions;
}

describe("OpenClaw fail-closed reserve path", () => {
  it("DENY aborts before upstream provider stream dispatch", async () => {
    const denial = new Error("budget denied");
    const reserve = async () => {
      throw denial;
    };
    let innerCalls = 0;
    const inner = async () => {
      innerCalls += 1;
      return { ok: true };
    };
    const provider = createSpendGuardOpenClawProvider(
      { id: "upstream", label: "Upstream", auth: [] },
      optionsWithReserve(reserve),
    );
    const streamFn = provider.wrapStreamFn?.({ streamFn: inner, provider: "openai", modelId: "gpt" });

    let thrown: unknown;
    try {
      await streamFn?.({ messages: [{ role: "user", content: "hello" }] });
    } catch (err) {
      thrown = err;
    }
    expect(thrown).toBe(denial);
    expect(innerCalls).toBe(0);
  });

  it("sidecar outage aborts before upstream provider stream dispatch", async () => {
    const outage = new Error("sidecar unavailable");
    const reserve = async () => {
      throw outage;
    };
    let innerCalls = 0;
    const inner = async () => {
      innerCalls += 1;
      return { ok: true };
    };
    const provider = createSpendGuardOpenClawProvider(
      { id: "upstream", label: "Upstream", auth: [] },
      optionsWithReserve(reserve),
    );
    const streamFn = provider.wrapStreamFn?.({ streamFn: inner, provider: "openai", modelId: "gpt" });

    let thrown: unknown;
    try {
      await streamFn?.({ messages: [{ role: "user", content: "hello" }] });
    } catch (err) {
      thrown = err;
    }
    expect(thrown).toBe(outage);
    expect(innerCalls).toBe(0);
  });

  it("ALLOW reserves before upstream provider stream dispatch", async () => {
    const sequence: string[] = [];
    const reserve = async () => {
      sequence.push("reserve");
      return outcome;
    };
    const inner = async () => {
      sequence.push("inner");
      return { ok: true };
    };
    const provider = createSpendGuardOpenClawProvider(
      { id: "upstream", label: "Upstream", auth: [] },
      optionsWithReserve(reserve),
    );
    const streamFn = provider.wrapStreamFn?.({ streamFn: inner, provider: "openai", modelId: "gpt" });

    const result = await streamFn?.({ messages: [{ role: "user", content: "hello" }] });
    expect(result).toEqual({ ok: true });
    expect(sequence).toEqual(["reserve", "inner"]);
  });
});
