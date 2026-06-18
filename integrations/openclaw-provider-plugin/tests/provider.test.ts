import { describe, expect, it } from "vitest";

import type { OpenClawSpendGuardOptions } from "../src/index.js";
import {
  OpenClawSpendGuardConfigError,
  OpenClawSpendGuardSettlementError,
  VERSION,
  createSpendGuardOpenClawProvider,
} from "../src/index.js";
import { buildOpenClawReserveRequest } from "../src/provider.js";

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
    for (const field of ["tenantId", "budgetId", "windowInstanceId", "unitId"] as const) {
      expect(() =>
        createSpendGuardOpenClawProvider(upstream, {
          ...options,
          [field]: "",
        }),
      ).toThrow(OpenClawSpendGuardConfigError);
    }

    expect(() =>
      createSpendGuardOpenClawProvider(upstream, {
        ...options,
        client: undefined as unknown as OpenClawSpendGuardOptions["client"],
      }),
    ).toThrow(OpenClawSpendGuardConfigError);

    expect(() =>
      createSpendGuardOpenClawProvider(upstream, {
        ...options,
        pricing: undefined as unknown as OpenClawSpendGuardOptions["pricing"],
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

  it("commits SUCCESS with reserve-time unit and pricing tuple", async () => {
    const commits: unknown[] = [];
    const provider = createSpendGuardOpenClawProvider(
      { id: "upstream", label: "Upstream", auth: [] },
      {
        ...options,
        client: {
          reserve: async () => outcome,
          commitEstimated: async (req: unknown) => {
            commits.push(req);
          },
        } as unknown as OpenClawSpendGuardOptions["client"],
        runIdProvider: () => "run_1",
      },
    );
    const streamFn = provider.wrapStreamFn?.({
      streamFn: async () => ({
        id: "evt_1",
        usage: { inputTokens: 5, outputTokens: 7 },
      }),
      provider: "openai",
      modelId: "gpt-test",
    });

    await streamFn?.({ messages: [{ role: "user", content: "ping" }] });

    expect(commits.length).toBe(1);
    expect(commits[0]).toEqual({
      runId: "run_1",
      stepId: "llm_call",
      llmCallId: (commits[0] as { llmCallId: string }).llmCallId,
      decisionId: "decision_1",
      reservationId: "reservation_1",
      estimatedAmountAtomic: "12",
      unit: { unit: "USD_MICROS", denomination: 1, unitId: "usd_micros" },
      pricing: options.pricing,
      providerEventId: "evt_1",
      outcome: "SUCCESS",
      actualInputTokens: 5,
      actualOutputTokens: 7,
    });
  });

  it("commits PROVIDER_ERROR and rethrows the original provider error", async () => {
    const commits: unknown[] = [];
    const providerError = new Error("provider failed");
    const provider = createSpendGuardOpenClawProvider(
      { id: "upstream", label: "Upstream", auth: [] },
      {
        ...options,
        client: {
          reserve: async () => outcome,
          commitEstimated: async (req: unknown) => {
            commits.push(req);
          },
        } as unknown as OpenClawSpendGuardOptions["client"],
        runIdProvider: () => "run_1",
      },
    );
    const streamFn = provider.wrapStreamFn?.({
      streamFn: async () => {
        throw providerError;
      },
      provider: "openai",
      modelId: "gpt-test",
    });

    let thrown: unknown;
    try {
      await streamFn?.({ messages: [{ role: "user", content: "ping" }] });
    } catch (err) {
      thrown = err;
    }

    expect(thrown).toBe(providerError);
    expect((commits[0] as { outcome: string }).outcome).toBe("PROVIDER_ERROR");
    // Failed call must RELEASE the reservation (commit zero via the FAILURE
    // multi-event path), never book the full projected estimate ("3000").
    expect((commits[0] as { estimatedAmountAtomic: string }).estimatedAmountAtomic).toBe("0");
    expect((commits[0] as { unit: unknown }).unit).toEqual({
      unit: "USD_MICROS",
      denomination: 1,
      unitId: "usd_micros",
    });
    expect((commits[0] as { pricing: unknown }).pricing).toBe(options.pricing);
    expect((commits[0] as { outcomeKind?: unknown }).outcomeKind).toBe("FAILURE");
    expect((commits[0] as { actualErrorMessage?: unknown }).actualErrorMessage).toBe(
      "provider failed",
    );
  });

  it("classifies AbortError as RUN_ABORTED", async () => {
    const commits: unknown[] = [];
    const abortError = new Error("aborted");
    abortError.name = "AbortError";
    const provider = createSpendGuardOpenClawProvider(
      { id: "upstream", label: "Upstream", auth: [] },
      {
        ...options,
        client: {
          reserve: async () => outcome,
          commitEstimated: async (req: unknown) => {
            commits.push(req);
          },
        } as unknown as OpenClawSpendGuardOptions["client"],
        runIdProvider: () => "run_1",
      },
    );
    const streamFn = provider.wrapStreamFn?.({
      streamFn: async () => {
        throw abortError;
      },
      provider: "openai",
      modelId: "gpt-test",
    });

    let thrown: unknown;
    try {
      await streamFn?.({ messages: [{ role: "user", content: "ping" }] });
    } catch (err) {
      thrown = err;
    }

    expect(thrown).toBe(abortError);
    expect((commits[0] as { outcome: string }).outcome).toBe("RUN_ABORTED");
    expect((commits[0] as { estimatedAmountAtomic: string }).estimatedAmountAtomic).toBe("0");
    expect((commits[0] as { outcomeKind?: unknown }).outcomeKind).toBe("FAILURE");
  });

  it("classifies timeout failures as CLIENT_TIMEOUT", async () => {
    const commits: unknown[] = [];
    const timeoutError = new Error("provider request timed out");
    const provider = createSpendGuardOpenClawProvider(
      { id: "upstream", label: "Upstream", auth: [] },
      {
        ...options,
        client: {
          reserve: async () => outcome,
          commitEstimated: async (req: unknown) => {
            commits.push(req);
          },
        } as unknown as OpenClawSpendGuardOptions["client"],
        runIdProvider: () => "run_1",
      },
    );
    const streamFn = provider.wrapStreamFn?.({
      streamFn: async () => {
        throw timeoutError;
      },
      provider: "openai",
      modelId: "gpt-test",
    });

    let thrown: unknown;
    try {
      await streamFn?.({ messages: [{ role: "user", content: "ping" }] });
    } catch (err) {
      thrown = err;
    }

    expect(thrown).toBe(timeoutError);
    expect((commits[0] as { outcome: string }).outcome).toBe("CLIENT_TIMEOUT");
    expect((commits[0] as { estimatedAmountAtomic: string }).estimatedAmountAtomic).toBe("0");
    expect((commits[0] as { outcomeKind?: unknown }).outcomeKind).toBe("FAILURE");
  });

  it("propagates commitEstimated failure after a reserved success", async () => {
    const provider = createSpendGuardOpenClawProvider(
      { id: "upstream", label: "Upstream", auth: [] },
      {
        ...options,
        client: {
          reserve: async () => outcome,
          commitEstimated: async () => {
            throw new Error("commit failed");
          },
        } as unknown as OpenClawSpendGuardOptions["client"],
        runIdProvider: () => "run_1",
      },
    );
    const streamFn = provider.wrapStreamFn?.({
      streamFn: async () => ({
        usage: { inputTokens: 1, outputTokens: 1 },
      }),
      provider: "openai",
      modelId: "gpt-test",
    });

    let thrown: unknown;
    try {
      await streamFn?.({ messages: [{ role: "user", content: "ping" }] });
    } catch (err) {
      thrown = err;
    }

    expect(thrown instanceof OpenClawSpendGuardSettlementError).toBe(true);
  });

  it("attempts every reservation settlement before reporting commit failure", async () => {
    const committedReservationIds: string[] = [];
    const provider = createSpendGuardOpenClawProvider(
      { id: "upstream", label: "Upstream", auth: [] },
      {
        ...options,
        client: {
          reserve: async () => ({
            ...outcome,
            reservationIds: ["reservation_1", "reservation_2"],
          }),
          commitEstimated: async (req: unknown) => {
            const reservationId = (req as { reservationId: string }).reservationId;
            committedReservationIds.push(reservationId);
            if (reservationId === "reservation_1") {
              throw new Error("first commit failed");
            }
          },
        } as unknown as OpenClawSpendGuardOptions["client"],
        claimEstimator: () => [
          {
            scopeId: "budget_1",
            amountAtomic: "1000",
            unit: { unit: "USD_MICROS", denomination: 1, unitId: "usd_micros" },
            windowInstanceId: "window_1",
          },
          {
            scopeId: "budget_2",
            amountAtomic: "2000",
            unit: { unit: "USD_MICROS", denomination: 1, unitId: "usd_micros" },
            windowInstanceId: "window_1",
          },
        ],
        runIdProvider: () => "run_1",
      },
    );
    const streamFn = provider.wrapStreamFn?.({
      streamFn: async () => ({
        usage: { inputTokens: 1, outputTokens: 1 },
      }),
      provider: "openai",
      modelId: "gpt-test",
    });

    let thrown: unknown;
    try {
      await streamFn?.({ messages: [{ role: "user", content: "ping" }] });
    } catch (err) {
      thrown = err;
    }

    expect(thrown instanceof OpenClawSpendGuardSettlementError).toBe(true);
    expect(committedReservationIds).toEqual(["reservation_1", "reservation_2"]);
  });
});
