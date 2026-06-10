// COV_D38_02 — reserve-path processor tests (tests.md TP-11, TP-12,
// TP-17..TP-21; slice doc NOTE 2: the commit-path TPs extend this file in
// COV_D38_03).
//
// Most TPs drive `processInputStep` directly with V1-shaped synthetic args
// (the hook only reads `args.messages` on the reserve path); TP-12 uses the
// REAL `@mastra/core` Agent + a tool-calling stub model to prove the hook
// fires on tool-call continuation steps too.

import { Agent } from "@mastra/core/agent";
import type { ProcessInputStepArgs } from "@mastra/core/processors";
import { createTool } from "@mastra/core/tools";
import { deriveUuidFromSignature } from "@spendguard/sdk";
import { describe, expect, it } from "vitest";
import { z } from "zod";
import { SpendGuardProcessor } from "../src/index.js";
import { MockSpendGuardClient, makeBudgetClaim } from "./_support/mockSidecar.js";
import { ToolCallingStubModel } from "./_support/stubModel.js";

// ── Synthetic V1-shaped hook args ─────────────────────────────────────────

let messageCounter = 0;

/** Build a MastraDBMessage-shaped step message (V1 pin — see flatten.ts). */
function dbMessage(role: "user" | "assistant", texts: string[]): Record<string, unknown> {
  messageCounter += 1;
  return {
    id: `msg-${messageCounter}`,
    role,
    createdAt: new Date(0),
    content: {
      format: 2,
      parts: texts.map((text) => ({ type: "text", text })),
    },
  };
}

/** Minimal V1-shaped args bag — the reserve path reads only `messages`. */
function makeArgs(messages: unknown[]): ProcessInputStepArgs {
  return {
    messages,
    stepNumber: 0,
    steps: [],
    systemMessages: [],
    state: {},
    retryCount: 0,
    abort: (reason?: string) => {
      throw new Error(`unexpected abort: ${reason ?? ""}`);
    },
  } as unknown as ProcessInputStepArgs;
}

describe("COV_D38_02 reserve path (TP-11, TP-12, TP-17..TP-21)", () => {
  it("TP-11: reserve wire shape — trigger/stepId/route defaults, decisionId === llmCallId", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp11" });
    await guard.processInputStep(makeArgs([dbMessage("user", ["ping"])]));

    expect(mock.reserveCalls).toHaveLength(1);
    const req = mock.lastReserveRequest;
    expect(req?.trigger).toBe("LLM_CALL_PRE");
    expect(req?.stepId).toBe("llm_call");
    expect(req?.route).toBe("mastra-llm");
    expect(req?.decisionId).toBe(req?.llmCallId);
    expect(req?.llmCallId).toBe(
      deriveUuidFromSignature("v1|tenant-tp11|ping", { scope: "mastra_llm_call_id" }),
    );
    expect(req?.idempotencyKey).toMatch(/^sg-[0-9a-f]{32}$/);
    expect(req?.projectedClaims).toHaveLength(1);

    // Route override threads through.
    const mock2 = new MockSpendGuardClient();
    const guard2 = new SpendGuardProcessor({
      client: mock2.client,
      tenantId: "tenant-tp11",
      route: "custom-route",
    });
    await guard2.processInputStep(makeArgs([dbMessage("user", ["ping"])]));
    expect(mock2.lastReserveRequest?.route).toBe("custom-route");
  });

  it("TP-12: processInputStep fires per step incl. tool-call continuation (1 tool call → 2 reserves)", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp12" });
    const stub = new ToolCallingStubModel("echoTool");
    const echoTool = createTool({
      id: "echoTool",
      description: "echoes its input",
      inputSchema: z.object({}).passthrough(),
      execute: async () => ({ echoed: true }),
    });
    const agent = new Agent({
      id: "tp12-agent",
      name: "tp12-agent",
      instructions: "use the tool",
      model: stub as never,
      tools: { echoTool },
      inputProcessors: [guard],
    });

    const result = await agent.generate("call the tool please");
    // Accumulated text: step-1 assistant text + step-2 final reply.
    expect(result.text).toBe("calling the toolstub-reply");

    // Step 1 (initial) + step 2 (tool-call continuation) → 2 reserves.
    expect(mock.reserveCalls).toHaveLength(2);
    // Each loop step appends messages → distinct stepText → distinct
    // llmCallId / decisionId / idempotencyKey per step (design §6.3).
    const [first, second] = mock.reserveCalls;
    expect(second?.request.llmCallId).not.toBe(first?.request.llmCallId);
    expect(second?.request.idempotencyKey).not.toBe(first?.request.idempotencyKey);
  }, 30_000);

  it("TP-17: claimEstimator called exactly once per reserve with {stepText, runId, llmCallId}; claims forwarded verbatim", async () => {
    const mock = new MockSpendGuardClient();
    const estimatorCalls: Array<{ stepText: string; runId: string; llmCallId: string }> = [];
    const customClaims = [
      {
        ...makeBudgetClaim("scope-custom", 123_456n),
        unit: { unit: "USD_MICROS", denomination: 1, unitId: "unit-via-estimator" },
        windowInstanceId: "wi-via-estimator",
      },
      makeBudgetClaim("scope-second", 1n),
    ];
    const guard = new SpendGuardProcessor({
      client: mock.client,
      tenantId: "tenant-tp17",
      claimEstimator: (input) => {
        estimatorCalls.push({ ...input });
        return customClaims;
      },
    });

    await guard.processInputStep(makeArgs([dbMessage("user", ["estimate me"])]));

    expect(estimatorCalls).toHaveLength(1);
    const req = mock.lastReserveRequest;
    expect(estimatorCalls[0]).toEqual({
      stepText: "estimate me",
      runId: req?.runId,
      llmCallId: req?.llmCallId,
    });
    // Forwarded verbatim — same claim objects, nothing rewritten (incl. the
    // estimator-supplied unitId + windowInstanceId pass-through).
    expect(req?.projectedClaims).toHaveLength(2);
    expect(req?.projectedClaims[0]).toBe(customClaims[0]);
    expect(req?.projectedClaims[1]).toBe(customClaims[1]);

    // Exactly once per reserve: a second step → a second estimator call.
    await guard.processInputStep(makeArgs([dbMessage("user", ["another step"])]));
    expect(estimatorCalls).toHaveLength(2);
  });

  it("TP-18: default projection — chars/4 heuristic, cap override, scopeId = budgetId ?? tenantId", async () => {
    // 8 chars → ceil(8/4)=2 tokens → 2 * 1000 micros.
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp18" });
    await guard.processInputStep(makeArgs([dbMessage("user", ["abcdefgh"])]));
    expect(mock.lastClaimAmountAtomic).toBe(2_000n);
    expect(mock.lastReserveRequest?.projectedClaims[0]?.scopeId).toBe("tenant-tp18");

    // Empty step text → max(1, ...) floor → 1000 micros.
    const mockEmpty = new MockSpendGuardClient();
    const guardEmpty = new SpendGuardProcessor({
      client: mockEmpty.client,
      tenantId: "tenant-tp18",
    });
    await guardEmpty.processInputStep(makeArgs([dbMessage("user", [""])]));
    expect(mockEmpty.lastClaimAmountAtomic).toBe(1_000n);

    // defaultBudgetMicrosCap > 0n replaces the heuristic amount.
    const mockCap = new MockSpendGuardClient();
    const guardCap = new SpendGuardProcessor({
      client: mockCap.client,
      tenantId: "tenant-tp18",
      defaultBudgetMicrosCap: 777n,
    });
    await guardCap.processInputStep(makeArgs([dbMessage("user", ["abcdefgh"])]));
    expect(mockCap.lastClaimAmountAtomic).toBe(777n);

    // cap === 0n → NOT > 0n → heuristic stays (design §6.4).
    const mockZero = new MockSpendGuardClient();
    const guardZero = new SpendGuardProcessor({
      client: mockZero.client,
      tenantId: "tenant-tp18",
      defaultBudgetMicrosCap: 0n,
    });
    await guardZero.processInputStep(makeArgs([dbMessage("user", ["abcdefgh"])]));
    expect(mockZero.lastClaimAmountAtomic).toBe(2_000n);

    // budgetId overrides the scopeId default.
    const mockBudget = new MockSpendGuardClient();
    const guardBudget = new SpendGuardProcessor({
      client: mockBudget.client,
      tenantId: "tenant-tp18",
      budgetId: "budget-override",
    });
    await guardBudget.processInputStep(makeArgs([dbMessage("user", ["abcdefgh"])]));
    expect(mockBudget.lastReserveRequest?.projectedClaims[0]?.scopeId).toBe("budget-override");
  });

  it("TP-19: unitId threading — set → claim unit.unitId equals it; unset → absent from wire UnitRef", async () => {
    const mockSet = new MockSpendGuardClient();
    const guardSet = new SpendGuardProcessor({
      client: mockSet.client,
      tenantId: "tenant-tp19",
      unitId: "66666666-6666-4666-8666-666666666666",
    });
    await guardSet.processInputStep(makeArgs([dbMessage("user", ["with unit"])]));
    const unitSet = mockSet.lastReserveRequest?.projectedClaims[0]?.unit;
    expect(unitSet).toEqual({
      unit: "USD_MICROS",
      denomination: 1,
      unitId: "66666666-6666-4666-8666-666666666666",
    });

    const mockUnset = new MockSpendGuardClient();
    const guardUnset = new SpendGuardProcessor({
      client: mockUnset.client,
      tenantId: "tenant-tp19",
    });
    await guardUnset.processInputStep(makeArgs([dbMessage("user", ["without unit"])]));
    const unitUnset = mockUnset.lastReserveRequest?.projectedClaims[0]?.unit;
    expect(unitUnset).toEqual({ unit: "USD_MICROS", denomination: 1 });
    expect(unitUnset !== undefined && "unitId" in unitUnset).toBe(false);
  });

  it("TP-20: runIdProvider wins; absent → runId === llmCallId (V3 pinned: no Mastra context id)", async () => {
    const mockProvider = new MockSpendGuardClient();
    const guardProvider = new SpendGuardProcessor({
      client: mockProvider.client,
      tenantId: "tenant-tp20",
      runIdProvider: () => "provider-run-id",
    });
    await guardProvider.processInputStep(makeArgs([dbMessage("user", ["run id test"])]));
    expect(mockProvider.lastReserveRequest?.runId).toBe("provider-run-id");

    const mockDerived = new MockSpendGuardClient();
    const guardDerived = new SpendGuardProcessor({
      client: mockDerived.client,
      tenantId: "tenant-tp20",
    });
    await guardDerived.processInputStep(makeArgs([dbMessage("user", ["run id test"])]));
    const req = mockDerived.lastReserveRequest;
    expect(req?.runId).toBe(req?.llmCallId);
  });

  it("TP-21: processor never mutates step messages (deep-equal before/after)", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp21" });
    const messages = [
      dbMessage("user", ["first message"]),
      dbMessage("assistant", ["reply", "second part"]),
      dbMessage("user", ["follow-up"]),
    ];
    const snapshot = structuredClone(messages);

    const result = await guard.processInputStep(makeArgs(messages));

    // Returning undefined = "no changes" under the installed hook contract.
    expect(result).toBeUndefined();
    expect(messages).toEqual(snapshot);
  });
});
