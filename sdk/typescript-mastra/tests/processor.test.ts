// COV_D38_02 — reserve-path processor tests (tests.md TP-11, TP-12,
// TP-17..TP-21) + COV_D38_03 — commit/failure/streaming tests (tests.md
// TP-23..TP-31).
//
// Most TPs drive the hooks directly with installed-shape synthetic args;
// TP-12/TP-23/TP-27/TP-28/TP-30/TP-31 use the REAL `@mastra/core` Agent +
// recording stub models to prove the full loop wiring (incl. the V4 hook
// ordering and the V7 `processAPIError` invocation) end to end.

import { Agent } from "@mastra/core/agent";
import type {
  ProcessAPIErrorArgs,
  ProcessInputStepArgs,
  ProcessLLMResponseArgs,
  ProcessOutputStepArgs,
} from "@mastra/core/processors";
import { createTool } from "@mastra/core/tools";
import { deriveUuidFromSignature } from "@spendguard/sdk";
import { afterEach, describe, expect, it, vi } from "vitest";
import { z } from "zod";
import { SpendGuardProcessor } from "../src/index.js";
import { MockSpendGuardClient, makeBudgetClaim } from "./_support/mockSidecar.js";
import {
  RecordingStubModel,
  ThrowingStubModel,
  ToolCallingStubModel,
} from "./_support/stubModel.js";

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

/**
 * Minimal V1-shaped args bag — the reserve path reads only `messages` (+
 * `state` for the COV_D38_03 inflight-key stash). Pass a shared `state`
 * object to correlate with the commit-hook builders below, mirroring the
 * request-scoped per-processor state bag the real ProcessorRunner threads
 * through every hook (V4 pin in src/processor.ts).
 */
function makeArgs(messages: unknown[], state: Record<string, unknown> = {}): ProcessInputStepArgs {
  return {
    messages,
    stepNumber: 0,
    steps: [],
    systemMessages: [],
    state,
    retryCount: 0,
    abort: (reason?: string) => {
      throw new Error(`unexpected abort: ${reason ?? ""}`);
    },
  } as unknown as ProcessInputStepArgs;
}

// ── COV_D38_03 synthetic commit-hook args (installed V4/V7 shapes) ────────

/** ProcessLLMResponseArgs-shaped bag: stripped chunks + shared state. */
function makeResponseArgs(
  state: Record<string, unknown>,
  chunks: unknown[],
): ProcessLLMResponseArgs {
  return {
    chunks,
    model: { modelId: "stub-model", provider: "spendguard-stub" },
    stepNumber: 0,
    steps: [],
    state,
    fromCache: false,
    retryCount: 0,
    abort: (reason?: string) => {
      throw new Error(`unexpected abort: ${reason ?? ""}`);
    },
  } as unknown as ProcessLLMResponseArgs;
}

/** ProcessOutputStepArgs-shaped bag: flat usage + shared state. */
function makeOutputStepArgs(
  state: Record<string, unknown>,
  usage?: Record<string, unknown>,
): ProcessOutputStepArgs {
  return {
    messages: [],
    messageList: { __testMessageList: true },
    stepNumber: 0,
    finishReason: "stop",
    text: "stub-reply",
    usage: usage ?? { inputTokens: undefined, outputTokens: undefined, totalTokens: undefined },
    systemMessages: [],
    steps: [],
    state,
    retryCount: 0,
    abort: (reason?: string) => {
      throw new Error(`unexpected abort: ${reason ?? ""}`);
    },
  } as unknown as ProcessOutputStepArgs;
}

/** ProcessAPIErrorArgs-shaped bag: error + shared state (V7 pin). */
function makeApiErrorArgs(state: Record<string, unknown>, error: unknown): ProcessAPIErrorArgs {
  return {
    error,
    messages: [],
    messageList: { __testMessageList: true },
    stepNumber: 0,
    steps: [],
    state,
    retryCount: 0,
    abort: (reason?: string) => {
      throw new Error(`unexpected abort: ${reason ?? ""}`);
    },
  } as unknown as ProcessAPIErrorArgs;
}

/** Build a fin chunk in the V4-pinned stripped `{type, payload}` shape. */
function finishChunk(usage: Record<string, unknown>): unknown {
  return { type: "finish", payload: { stepResult: { reason: "stop" }, output: { usage } } };
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

// ── COV_D38_03 — commit + failure settlement (TP-23..TP-31) ───────────────
//
// RATIFIED ERRATUM (design.md §6.7 amendment #2, 2026-06-10, orchestrator-
// ratified — was a DECLARED DEVIATION): tests.md TP-24's original one-liner
// read `estimatedAmountAtomic="0"`, but design §6.6 simultaneously LOCKS
// the wire shape to be "identical to the shipped D04 handler" — and the
// shipped D04 handler (HARDEN_D05_WI) sends estimate = input+output token
// SUM on SUCCESS because the ledger rejects `estimated_amount_atomic = 0`
// bookings. The HARDEN_D05_WI convention controls; TP-24/TP-25 below assert
// the token-sum estimate.

describe("COV_D38_03 commit + failure paths (TP-23..TP-31)", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("TP-23: real Agent happy path — reserve → response → exactly ONE SUCCESS commit with reserve-outcome ids", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp23" });
    const stub = new RecordingStubModel();
    const agent = new Agent({
      id: "tp23-agent",
      name: "tp23-agent",
      instructions: "reply",
      model: stub as never,
      inputProcessors: [guard],
    });

    const result = await agent.generate("ping");
    expect(result.text).toBe("stub-reply");

    expect(mock.reserveCalls).toHaveLength(1);
    expect(mock.commitCalls).toHaveLength(1);
    const reserve = mock.reserveCalls[0];
    const commit = mock.commitCalls[0]?.request;
    expect(commit?.outcome).toBe("SUCCESS");
    expect(commit?.outcomeKind).toBe("SUCCESS");
    // ids come from the reserve OUTCOME (decision/reservation) + the
    // reserve REQUEST tuple (runId/llmCallId/stepId).
    expect(commit?.decisionId).toBe(reserve?.resolved?.decisionId);
    expect(commit?.reservationId).toBe(reserve?.resolved?.reservationIds[0]);
    expect(commit?.runId).toBe(reserve?.request.runId);
    expect(commit?.llmCallId).toBe(reserve?.request.llmCallId);
    expect(commit?.stepId).toBe("llm_call");
    // Stub usage (10/5) flows through the loop's normalized finish chunk.
    expect(commit?.actualInputTokensWire).toBe("10");
    expect(commit?.actualOutputTokensWire).toBe("5");
    expect(commit?.estimatedAmountAtomic).toBe("15");
  }, 30_000);

  it("TP-24: usage exposed (V4 camelCase) → actuals on the wire; estimate = token sum (HARDEN_D05_WI)", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp24" });
    const state: Record<string, unknown> = {};

    await guard.processInputStep(makeArgs([dbMessage("user", ["ping"])], state));
    await guard.processLLMResponse(
      makeResponseArgs(state, [finishChunk({ inputTokens: 7, outputTokens: 3 })]),
    );

    expect(mock.commitCalls).toHaveLength(1);
    const commit = mock.commitCalls[0]?.request;
    expect(commit?.actualInputTokensWire).toBe("7");
    expect(commit?.actualOutputTokensWire).toBe("3");
    expect(commit?.estimatedAmountAtomic).toBe("10");
    expect(commit?.outcome).toBe("SUCCESS");
    expect(commit?.outcomeKind).toBe("SUCCESS");
  });

  it("TP-25: usage exposed snake_case → same as TP-24 (cross-shape parity)", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp25" });
    const state: Record<string, unknown> = {};

    await guard.processInputStep(makeArgs([dbMessage("user", ["ping"])], state));
    await guard.processLLMResponse(
      makeResponseArgs(state, [finishChunk({ input_tokens: 7, output_tokens: 3 })]),
    );

    expect(mock.commitCalls).toHaveLength(1);
    const commit = mock.commitCalls[0]?.request;
    expect(commit?.actualInputTokensWire).toBe("7");
    expect(commit?.actualOutputTokensWire).toBe("3");
    expect(commit?.estimatedAmountAtomic).toBe("10");
  });

  it("TP-26: usage ABSENT → estimate = reserve-time projectedAmountAtomic; NO actuals fields (§6.6 LOCKED)", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp26" });
    const state: Record<string, unknown> = {};

    // "abcdefgh" → ceil(8/4)=2 tokens → 2000 micros projection (TP-18).
    await guard.processInputStep(makeArgs([dbMessage("user", ["abcdefgh"])], state));
    // finish chunk without usage → extractUsage returns undefined.
    await guard.processLLMResponse(
      makeResponseArgs(state, [{ type: "finish", payload: { output: {} } }]),
    );

    expect(mock.commitCalls).toHaveLength(1);
    const commit = mock.commitCalls[0]?.request;
    expect(commit?.estimatedAmountAtomic).toBe("2000");
    expect(commit !== undefined && "actualInputTokensWire" in commit).toBe(false);
    expect(commit !== undefined && "actualOutputTokensWire" in commit).toBe(false);
    expect(commit?.outcome).toBe("SUCCESS");
  });

  it("custom claimEstimator with a DIFFERENT unit/unitId → commit carries the estimator's reserve-time unit (HARDEN_D05_WI tuple match; §6.5 amendment 2026-06-10)", async () => {
    const mock = new MockSpendGuardClient();
    // TP-17 estimator-fixture mirror: the estimator reserves under a unit
    // that differs from the default buildUnit() projection.
    const estimatorUnit = { unit: "USD_MICROS", denomination: 1, unitId: "unit-via-estimator" };
    const guard = new SpendGuardProcessor({
      client: mock.client,
      tenantId: "tenant-unit-match",
      claimEstimator: () => [
        {
          ...makeBudgetClaim("scope-custom", 123_456n),
          unit: estimatorUnit,
        },
      ],
    });
    const state: Record<string, unknown> = {};

    await guard.processInputStep(makeArgs([dbMessage("user", ["estimate me"])], state));
    await guard.processLLMResponse(
      makeResponseArgs(state, [finishChunk({ inputTokens: 7, outputTokens: 3 })]),
    );

    expect(mock.commitCalls).toHaveLength(1);
    const commit = mock.commitCalls[0]?.request;
    // The commit's unit tuple-matches the reservation's claim[0].unit —
    // NOT the default-options buildUnit() shape (which has no unitId here).
    expect(commit?.unit).toEqual(estimatorUnit);
    expect(mock.lastReserveRequest?.projectedClaims[0]?.unit).toEqual(estimatorUnit);
  });

  it("TP-27a: provider error → FAILURE commit via the V7-pinned processAPIError hook (direct)", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp27" });
    const state: Record<string, unknown> = {};

    await guard.processInputStep(makeArgs([dbMessage("user", ["abcdefgh"])], state));
    await guard.processAPIError(makeApiErrorArgs(state, new Error("provider exploded")));

    expect(mock.commitCalls).toHaveLength(1);
    const commit = mock.commitCalls[0]?.request;
    expect(commit?.outcome).toBe("PROVIDER_ERROR");
    expect(commit?.outcomeKind).toBe("FAILURE");
    expect(commit?.actualErrorMessage).toBe("provider exploded");
    // Usage is absent on the error path → §6.6 fallback estimate.
    expect(commit?.estimatedAmountAtomic).toBe("2000");
    expect(commit !== undefined && "actualInputTokensWire" in commit).toBe(false);
  });

  it("TP-27b: real Agent + throwing model → agent rejects, FAILURE settlement, NO success commit", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp27b" });
    const stub = new ThrowingStubModel("stub provider boom");
    const agent = new Agent({
      id: "tp27b-agent",
      name: "tp27b-agent",
      instructions: "reply",
      model: stub as never,
      inputProcessors: [guard],
    });

    await expect(agent.generate("ping")).rejects.toThrow(/stub provider boom/);

    // The reserve passed and the provider boundary WAS crossed.
    expect(mock.reserveCalls).toHaveLength(1);
    expect(stub.totalCalls).toBeGreaterThan(0);
    // V7 PRIMARY signal (pin in src/processor.ts): the model-execution
    // error rides the chunk stream as an `error` chunk and reaches
    // `processLLMResponse`, which settles FAILURE — exactly once, with NO
    // SUCCESS commit anywhere on the error path.
    expect(mock.commitCalls).toHaveLength(1);
    const commit = mock.commitCalls[0]?.request;
    expect(commit?.outcomeKind).toBe("FAILURE");
    expect(commit?.outcome).toBe("PROVIDER_ERROR");
    expect(commit?.actualErrorMessage).toMatch(/stub provider boom/);
    expect(commit !== undefined && "actualInputTokensWire" in commit).toBe(false);
  }, 30_000);

  it("TP-28: commit RPC failure after success → consumer still gets the result; logged; no throw", async () => {
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const mock = new MockSpendGuardClient({
      simulatedCommitError: new Error("sidecar commit RPC down"),
    });
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp28" });
    const stub = new RecordingStubModel();
    const agent = new Agent({
      id: "tp28-agent",
      name: "tp28-agent",
      instructions: "reply",
      model: stub as never,
      inputProcessors: [guard],
    });

    // §7.4 LOCKED pre/post asymmetry: the already-paid-for result is
    // delivered even though the commit RPC failed.
    const result = await agent.generate("ping");
    expect(result.text).toBe("stub-reply");

    expect(mock.commitCalls).toHaveLength(1);
    expect(mock.commitCalls[0]?.rejected?.message).toBe("sidecar commit RPC down");
    expect(errorSpy).toHaveBeenCalledWith(expect.stringContaining("TTL sweep will settle"));
  }, 30_000);

  it("TP-29: commit hook with no inflight entry → warn + no-op (no throw, no RPC)", async () => {
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp29" });

    // No reserve ever happened — no state stash, no runIdProvider.
    await guard.processLLMResponse(
      makeResponseArgs({}, [finishChunk({ inputTokens: 1, outputTokens: 1 })]),
    );
    // Stale stash key whose entry was already popped → same warn + no-op.
    const state: Record<string, unknown> = {};
    await guard.processInputStep(makeArgs([dbMessage("user", ["ping"])], state));
    await guard.processLLMResponse(
      makeResponseArgs(state, [finishChunk({ inputTokens: 1, outputTokens: 1 })]),
    );
    await guard.processLLMResponse(
      makeResponseArgs(state, [finishChunk({ inputTokens: 1, outputTokens: 1 })]),
    );

    // Exactly the ONE real settlement; the two orphan re-deliveries warned.
    expect(mock.commitCalls).toHaveLength(1);
    expect(warnSpy).toHaveBeenCalledTimes(2);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining("no inflight entry"));
  });

  it("TP-30: streaming step → exactly one reserve at open + one commit after completion; no per-chunk RPCs", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp30" });
    const stub = new RecordingStubModel({ replyText: "one two three" });
    const agent = new Agent({
      id: "tp30-agent",
      name: "tp30-agent",
      instructions: "count",
      model: stub as never,
      inputProcessors: [guard],
    });

    const out = await agent.stream("count to 3");
    let chunkCount = 0;
    let text = "";
    for await (const piece of out.textStream) {
      chunkCount += 1;
      text += piece;
      // Whole-step bracket (design §8): NO commit while chunks are flowing
      // before stream completion — per-chunk gating is out of scope.
      if (chunkCount === 1) {
        expect(mock.reserveCalls).toHaveLength(1);
        expect(mock.commitCalls).toHaveLength(0);
      }
    }
    await out.getFullOutput();

    expect(text).toBe("one two three");
    expect(mock.reserveCalls).toHaveLength(1);
    expect(mock.commitCalls).toHaveLength(1);
    const commit = mock.commitCalls[0]?.request;
    expect(commit?.outcome).toBe("SUCCESS");
    expect(commit?.actualInputTokensWire).toBe("10");
    expect(commit?.actualOutputTokensWire).toBe("5");
  }, 30_000);

  it("TP-31a: response AND output hooks both fire (direct) → exactly one commit RPC", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp31" });
    const state: Record<string, unknown> = {};

    await guard.processInputStep(makeArgs([dbMessage("user", ["ping"])], state));
    // V4-pinned order: response hook first, output-step backstop second.
    await guard.processLLMResponse(
      makeResponseArgs(state, [finishChunk({ inputTokens: 4, outputTokens: 6 })]),
    );
    const listBefore = makeOutputStepArgs(state, { inputTokens: 4, outputTokens: 6 });
    const returned = await guard.processOutputStep(listBefore);

    // Backstop found the reservation already settled → silent no-op; the
    // SAME messageList instance is returned (no step mutation).
    expect(mock.commitCalls).toHaveLength(1);
    expect(returned).toBe((listBefore as unknown as { messageList: unknown }).messageList);

    // Inverse coverage: output-mounted-only instance — the backstop COMMITS
    // for real (e.g. cached-response replay skipped the response hook).
    const state2: Record<string, unknown> = {};
    await guard.processInputStep(makeArgs([dbMessage("user", ["other step"])], state2));
    await guard.processOutputStep(makeOutputStepArgs(state2, { inputTokens: 2, outputTokens: 2 }));
    expect(mock.commitCalls).toHaveLength(2);
    expect(mock.commitCalls[1]?.request.actualInputTokensWire).toBe("2");
    expect(mock.commitCalls[1]?.request.estimatedAmountAtomic).toBe("4");
  });

  it("TP-31b: real Agent mounted in BOTH inputProcessors and outputProcessors → exactly one commit", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp31b" });
    const stub = new RecordingStubModel();
    const agent = new Agent({
      id: "tp31b-agent",
      name: "tp31b-agent",
      instructions: "reply",
      model: stub as never,
      inputProcessors: [guard],
      outputProcessors: [guard],
    });

    const result = await agent.generate("ping");
    expect(result.text).toBe("stub-reply");

    // Both hooks fired (V4 order: response first, output-step last); the
    // FIFO pop guarantees at-most-one-commit per reservation.
    expect(mock.reserveCalls).toHaveLength(1);
    expect(mock.commitCalls).toHaveLength(1);
    expect(mock.commitCalls[0]?.request.outcomeKind).toBe("SUCCESS");
  }, 30_000);
});

// ── COV_D38_04 — threading (TP-17/TP-19 family) + backstop-for-real ───────

describe("COV_D38_04 estimator threading + settlement details", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("TP-17 family (unitId): estimator-supplied claim unitId reaches the wire UNTOUCHED — even when options.unitId differs", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({
      client: mock.client,
      tenantId: "tenant-thread-unit",
      // The options-level unitId would drive the DEFAULT projection; the
      // estimator overrides the WHOLE projection (design §6.4 last line),
      // so the estimator's unit must win verbatim.
      unitId: "options-level-unit",
      claimEstimator: () => [
        {
          ...makeBudgetClaim("scope-thread", 42n),
          unit: { unit: "USD_MICROS", denomination: 1, unitId: "estimator-level-unit" },
        },
      ],
    });
    await guard.processInputStep(makeArgs([dbMessage("user", ["thread unit"])]));
    expect(mock.lastReserveRequest?.projectedClaims[0]?.unit.unitId).toBe("estimator-level-unit");
  });

  it("TP-17 family (windowInstanceId): estimator claim windowInstanceId threads verbatim onto the reserve wire (HARDEN_D05_WI)", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({
      client: mock.client,
      tenantId: "tenant-thread-wi",
      claimEstimator: () => [
        {
          ...makeBudgetClaim("scope-wi", 7n),
          windowInstanceId: "11111111-1111-4111-8111-111111111111",
        },
      ],
    });
    await guard.processInputStep(makeArgs([dbMessage("user", ["thread wi"])]));
    const claim = mock.lastReserveRequest?.projectedClaims[0];
    expect(claim?.windowInstanceId).toBe("11111111-1111-4111-8111-111111111111");

    // Default projection sends NO windowInstanceId (the §6.4 LOCKED shape
    // has none — recipe-style; ledger-backed callers supply it via the
    // estimator, exactly the substrate's documented contract).
    const mockDefault = new MockSpendGuardClient();
    const guardDefault = new SpendGuardProcessor({
      client: mockDefault.client,
      tenantId: "tenant-thread-wi",
    });
    await guardDefault.processInputStep(makeArgs([dbMessage("user", ["thread wi"])]));
    const defaultClaim = mockDefault.lastReserveRequest?.projectedClaims[0];
    expect(defaultClaim !== undefined && "windowInstanceId" in defaultClaim).toBe(false);
  });

  it("TP-19 family (commit-side unitId): options.unitId rides the inflight entry onto the commit unit (tuple match)", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({
      client: mock.client,
      tenantId: "tenant-commit-unit",
      unitId: "66666666-6666-4666-8666-666666666666",
    });
    const state: Record<string, unknown> = {};
    await guard.processInputStep(makeArgs([dbMessage("user", ["ping"])], state));
    await guard.processLLMResponse(
      makeResponseArgs(state, [finishChunk({ inputTokens: 1, outputTokens: 1 })]),
    );
    const commit = mock.commitCalls[0]?.request;
    expect(commit?.unit).toEqual({
      unit: "USD_MICROS",
      denomination: 1,
      unitId: "66666666-6666-4666-8666-666666666666",
    });
  });

  it("pricing threading back-compat: absent `pricing` option → the commit sends the empty freeze tuple (HARDEN_D05_WI; §6.7 amendment #3)", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-pricing" });
    const state: Record<string, unknown> = {};
    await guard.processInputStep(makeArgs([dbMessage("user", ["ping"])], state));
    await guard.processLLMResponse(
      makeResponseArgs(state, [finishChunk({ inputTokens: 1, outputTokens: 1 })]),
    );
    const commit = mock.commitCalls[0]?.request;
    // No `pricing` option → the commit's freeze tuple is the empty tuple
    // (pre-amendment wire shape — matches no-bundle reservations only).
    expect(commit?.pricing).toEqual({ pricingVersion: "", pricingHash: new Uint8Array(0) });
  });

  it("pricing threading: `pricing` option rides the inflight entry onto the commit wire verbatim (§6.7 amendment #3; D04 handler.ts:316/377 parity)", async () => {
    const mock = new MockSpendGuardClient();
    const pricing = {
      pricingVersion: "2026-06-01",
      pricingHash: new Uint8Array([0xde, 0xad, 0xbe, 0xef]),
      fxRateVersion: "fx-2026-06",
      unitConversionVersion: "uc-1",
    };
    const guard = new SpendGuardProcessor({
      client: mock.client,
      tenantId: "tenant-pricing-opt",
      pricing,
    });
    const state: Record<string, unknown> = {};
    await guard.processInputStep(makeArgs([dbMessage("user", ["ping"])], state));
    await guard.processLLMResponse(
      makeResponseArgs(state, [finishChunk({ inputTokens: 1, outputTokens: 1 })]),
    );
    const commit = mock.commitCalls[0]?.request;
    // The sidecar stamps the reservation with the loaded bundle's freeze;
    // the commit must repeat the SAME tuple (HARDEN_D05_WI — empty-tuple
    // commits are rejected with `pricing freeze mismatch`, proved live by
    // the COV_D38_05 demo).
    expect(commit?.pricing).toEqual(pricing);
  });

  it("pricing threading: the FAILURE settlement path also repeats the `pricing` option tuple (§6.7 amendment #3)", async () => {
    const mock = new MockSpendGuardClient();
    const pricing = { pricingVersion: "2026-06-01", pricingHash: new Uint8Array([7]) };
    const guard = new SpendGuardProcessor({
      client: mock.client,
      tenantId: "tenant-pricing-fail",
      pricing,
    });
    const state: Record<string, unknown> = {};
    await guard.processInputStep(makeArgs([dbMessage("user", ["ping"])], state));
    await guard.processLLMResponse(
      makeResponseArgs(state, [
        { type: "error", payload: { error: new Error("provider exploded") } },
      ]),
    );
    const commit = mock.commitCalls[0]?.request;
    expect(commit?.outcomeKind).toBe("FAILURE");
    expect(commit?.pricing).toEqual(pricing);
  });

  it("backstop-commits-for-real (D38_03 R1 minor 3): response-hook settlement suppressed → the OUTPUT runner backstop commits", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-backstop" });
    // Suppress the response-hook settlement on this INSTANCE (simulates the
    // V4-pinned cached-replay / output-mounted-only ordering where the
    // response hook does not settle) — no src change, pure test override.
    const responseSpy = vi
      .spyOn(guard, "processLLMResponse")
      .mockImplementation(async () => undefined);
    const outputSpy = vi.spyOn(guard, "processOutputStep");
    const stub = new RecordingStubModel();
    const agent = new Agent({
      id: "backstop-agent",
      name: "backstop-agent",
      instructions: "reply",
      model: stub as never,
      inputProcessors: [guard],
      outputProcessors: [guard],
    });

    const result = await agent.generate("ping");
    expect(result.text).toBe("stub-reply");

    // The suppressed response hook DID fire (proving suppression was the
    // only reason it didn't settle) and the backstop committed FOR REAL.
    expect(responseSpy).toHaveBeenCalled();
    expect(outputSpy).toHaveBeenCalled();
    expect(mock.reserveCalls).toHaveLength(1);
    expect(mock.commitCalls).toHaveLength(1);
    const commit = mock.commitCalls[0]?.request;
    expect(commit?.outcome).toBe("SUCCESS");
    expect(commit?.outcomeKind).toBe("SUCCESS");
    expect(commit?.decisionId).toBe(mock.reserveCalls[0]?.resolved?.decisionId);
    expect(commit?.reservationId).toBe(mock.reserveCalls[0]?.resolved?.reservationIds[0]);
    // The real loop feeds flat usage to processOutputStep (V4 pin): the
    // backstop settles with actuals, not the §6.6 fallback.
    expect(commit?.actualInputTokensWire).toBe("10");
    expect(commit?.actualOutputTokensWire).toBe("5");
    expect(commit?.estimatedAmountAtomic).toBe("15");
  }, 30_000);

  it("V7 error-chunk message flavours: string error / POJO without message / chunk without payload", async () => {
    // payload.error is a STRING → String(error) is the commit message.
    const mock1 = new MockSpendGuardClient();
    const guard1 = new SpendGuardProcessor({ client: mock1.client, tenantId: "tenant-ec1" });
    const state1: Record<string, unknown> = {};
    await guard1.processInputStep(makeArgs([dbMessage("user", ["ping"])], state1));
    await guard1.processLLMResponse(
      makeResponseArgs(state1, [{ type: "error", payload: { error: "string-flavoured boom" } }]),
    );
    expect(mock1.commitCalls[0]?.request.actualErrorMessage).toBe("string-flavoured boom");
    expect(mock1.commitCalls[0]?.request.outcomeKind).toBe("FAILURE");

    // payload.error is a POJO with a NON-string message → String(error).
    const mock2 = new MockSpendGuardClient();
    const guard2 = new SpendGuardProcessor({ client: mock2.client, tenantId: "tenant-ec2" });
    const state2: Record<string, unknown> = {};
    await guard2.processInputStep(makeArgs([dbMessage("user", ["ping"])], state2));
    await guard2.processLLMResponse(
      makeResponseArgs(state2, [{ type: "error", payload: { error: { message: 42 } } }]),
    );
    expect(mock2.commitCalls[0]?.request.actualErrorMessage).toBe("[object Object]");

    // error chunk WITHOUT a usable payload → placeholder message, still a
    // FAILURE settlement (the signal is the chunk type, not the payload).
    const mock3 = new MockSpendGuardClient();
    const guard3 = new SpendGuardProcessor({ client: mock3.client, tenantId: "tenant-ec3" });
    const state3: Record<string, unknown> = {};
    await guard3.processInputStep(makeArgs([dbMessage("user", ["ping"])], state3));
    await guard3.processLLMResponse(makeResponseArgs(state3, [{ type: "error" }]));
    expect(mock3.commitCalls[0]?.request.actualErrorMessage).toBe(
      "provider error (no message exposed)",
    );

    // payload present but NO error key → same placeholder.
    const mock4 = new MockSpendGuardClient();
    const guard4 = new SpendGuardProcessor({ client: mock4.client, tenantId: "tenant-ec4" });
    const state4: Record<string, unknown> = {};
    await guard4.processInputStep(makeArgs([dbMessage("user", ["ping"])], state4));
    await guard4.processLLMResponse(makeResponseArgs(state4, [{ type: "error", payload: {} }]));
    expect(mock4.commitCalls[0]?.request.actualErrorMessage).toBe(
      "provider error (no message exposed)",
    );
  });

  it("non-integer usage tokens → BigInt sum fails safe to the reserve-time projection (§6.6 fallback estimate)", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-frac" });
    const state: Record<string, unknown> = {};
    // "abcdefgh" → 2000-micros projection (TP-18 fixture).
    await guard.processInputStep(makeArgs([dbMessage("user", ["abcdefgh"])], state));
    // Fractional token counts are finite numbers (pass extractUsage) but
    // BigInt(1.5) throws — the settlement must fall back, not crash.
    await guard.processLLMResponse(
      makeResponseArgs(state, [finishChunk({ inputTokens: 1.5, outputTokens: 2.5 })]),
    );
    expect(mock.commitCalls).toHaveLength(1);
    const commit = mock.commitCalls[0]?.request;
    expect(commit?.estimatedAmountAtomic).toBe("2000");
    expect(commit?.outcome).toBe("SUCCESS");
  });

  it("commit-key recovery falls back to runIdProvider when the state stash is missing", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({
      client: mock.client,
      tenantId: "tenant-keyfall",
      runIdProvider: () => "provider-run-key",
    });
    // Reserve with one state bag…
    await guard.processInputStep(makeArgs([dbMessage("user", ["ping"])], {}));
    // …commit hook arrives with a DIFFERENT (empty) state bag — the
    // secondary recovery source (runIdProvider) finds the entry.
    await guard.processLLMResponse(
      makeResponseArgs({}, [finishChunk({ inputTokens: 1, outputTokens: 1 })]),
    );
    expect(mock.commitCalls).toHaveLength(1);
    expect(mock.commitCalls[0]?.request.runId).toBe("provider-run-key");
  });

  it("processLLMRequest is a no-op in v1 (design §11.3 LOCKED): no RPC, undefined result", () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-noop" });
    expect(guard.processLLMRequest({} as never)).toBeUndefined();
    expect(mock.reserveCalls).toHaveLength(0);
    expect(mock.commitCalls).toHaveLength(0);
  });
});
