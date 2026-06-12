import { describe, expect, it } from "vitest";

import type { OpenClawSpendGuardOptions } from "../src/index.js";
import {
  OpenClawSpendGuardSettlementError,
  createSpendGuardOpenClawProvider,
} from "../src/index.js";
import { extractOpenClawUsage } from "../src/usage.js";

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

const options = {
  client: {} as OpenClawSpendGuardOptions["client"],
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

describe("OpenClaw usage and streaming settlement", () => {
  it("extracts common provider usage shapes", () => {
    expect(
      extractOpenClawUsage({
        id: "evt_1",
        usage: { prompt_tokens: 3, completion_tokens: 4 },
      }),
    ).toEqual({ inputTokens: 3, outputTokens: 4, providerEventId: "evt_1" });
  });

  it("commits exactly once when an async stream reaches terminal completion", async () => {
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
      },
    );
    async function* chunks() {
      yield { delta: "a" };
      yield { id: "evt_stream", usage: { inputTokens: 2, outputTokens: 5 } };
    }
    const streamFn = provider.wrapStreamFn?.({
      streamFn: async () => chunks(),
      provider: "openai",
      modelId: "gpt-test",
    });

    const result = await streamFn?.({ messages: [{ role: "user", content: "ping" }] });
    const observed: unknown[] = [];
    for await (const chunk of result as AsyncIterable<unknown>) {
      observed.push(chunk);
    }

    expect(observed.length).toBe(2);
    expect(commits.length).toBe(1);
    expect((commits[0] as { outcome: string }).outcome).toBe("SUCCESS");
    expect((commits[0] as { estimatedAmountAtomic: string }).estimatedAmountAtomic).toBe("7");
    expect((commits[0] as { providerEventId: string }).providerEventId).toBe("evt_stream");
    expect((commits[0] as { outcomeKind?: unknown }).outcomeKind).toBe(undefined);
  });

  it("classifies aborted-signal stream throws as RUN_ABORTED", async () => {
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
      },
    );
    async function* chunks() {
      yield { delta: "a" };
      throw new Error("transport closed");
    }
    const abort = new AbortController();
    abort.abort();
    const streamFn = provider.wrapStreamFn?.({
      streamFn: async () => chunks(),
      provider: "openai",
      modelId: "gpt-test",
    });

    const result = await streamFn?.({
      messages: [{ role: "user", content: "ping" }],
      signal: abort.signal,
    });
    let thrown: unknown;
    try {
      for await (const _chunk of result as AsyncIterable<unknown>) {
        // drain until the provider stream throws
      }
    } catch (err) {
      thrown = err;
    }

    expect(thrown instanceof Error).toBe(true);
    expect(commits.length).toBe(1);
    expect((commits[0] as { outcome: string }).outcome).toBe("RUN_ABORTED");
  });

  it("classifies provider stream throws as PROVIDER_ERROR exactly once", async () => {
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
      },
    );
    async function* chunks() {
      yield { delta: "a" };
      throw new Error("provider stream failed");
    }
    const streamFn = provider.wrapStreamFn?.({
      streamFn: async () => chunks(),
      provider: "openai",
      modelId: "gpt-test",
    });

    const result = await streamFn?.({ messages: [{ role: "user", content: "ping" }] });
    let thrown: unknown;
    try {
      for await (const _chunk of result as AsyncIterable<unknown>) {
        // drain until the provider stream throws
      }
    } catch (err) {
      thrown = err;
    }

    expect(thrown instanceof Error).toBe(true);
    expect(commits.length).toBe(1);
    expect((commits[0] as { outcome: string }).outcome).toBe("PROVIDER_ERROR");
  });

  it("settles RUN_ABORTED exactly once when the consumer returns early", async () => {
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
      },
    );
    async function* chunks() {
      yield { delta: "a" };
      yield { id: "evt_stream", usage: { inputTokens: 2, outputTokens: 5 } };
    }
    const streamFn = provider.wrapStreamFn?.({
      streamFn: async () => chunks(),
      provider: "openai",
      modelId: "gpt-test",
    });

    const result = await streamFn?.({ messages: [{ role: "user", content: "ping" }] });
    for await (const _chunk of result as AsyncIterable<unknown>) {
      break;
    }

    expect(commits.length).toBe(1);
    expect((commits[0] as { outcome: string }).outcome).toBe("RUN_ABORTED");
  });

  it("does not issue a second failure commit when terminal SUCCESS settlement rejects", async () => {
    const commits: unknown[] = [];
    const provider = createSpendGuardOpenClawProvider(
      { id: "upstream", label: "Upstream", auth: [] },
      {
        ...options,
        client: {
          reserve: async () => outcome,
          commitEstimated: async (req: unknown) => {
            commits.push(req);
            throw new Error("commit failed");
          },
        } as unknown as OpenClawSpendGuardOptions["client"],
      },
    );
    async function* chunks() {
      yield { id: "evt_stream", usage: { inputTokens: 2, outputTokens: 5 } };
    }
    const streamFn = provider.wrapStreamFn?.({
      streamFn: async () => chunks(),
      provider: "openai",
      modelId: "gpt-test",
    });

    const result = await streamFn?.({ messages: [{ role: "user", content: "ping" }] });
    let thrown: unknown;
    try {
      for await (const _chunk of result as AsyncIterable<unknown>) {
        // drain stream
      }
    } catch (err) {
      thrown = err;
    }

    expect(thrown instanceof OpenClawSpendGuardSettlementError).toBe(true);
    expect(commits.length).toBe(1);
    expect((commits[0] as { outcome: string }).outcome).toBe("SUCCESS");
  });
});
