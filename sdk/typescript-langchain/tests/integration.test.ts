// SLICE 4 — End-to-end integration tests: SpendGuardCallbackHandler driven
// through a real `@langchain/openai` ChatOpenAI against a stubbed fetch + an
// in-process MockSpendGuardClient.
//
// Scope (per docs/specs/coverage/D04_langchain_ts/{design,implementation,
// review-standards}.md SLICE 4):
//   - Prove the LangChain ↔ adapter ↔ substrate handshake is correct across
//     the full happy/sad-path matrix without ever hitting the real OpenAI API
//     or the real SpendGuard sidecar UDS.
//   - 20 scenarios spanning happy-path lifecycle, terminal denial, provider
//     errors, sidecar degradation, concurrency, idempotency, payload shape,
//     and handler reuse — all enumerated in the SLICE 4 prompt and pinned to
//     review-standards.md §2-§5.
//
// Mock surface owned by `./_support/mockSidecar.ts` and `./_support/openAiStub.ts`.
// The tests themselves carry no UDS bind, no gRPC channel, and no network I/O.

import { BaseCallbackHandler } from "@langchain/core/callbacks/base";
import { HumanMessage } from "@langchain/core/messages";
import { ChatOpenAI } from "@langchain/openai";
import {
  ApprovalRequired,
  DecisionDenied,
  DecisionStopped,
  type ReserveRequest,
} from "@spendguard/sdk";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SpendGuardCallbackHandler } from "../src/handler.js";
import type { SpendGuardCallbackHandlerOptions } from "../src/options.js";
import { MockSpendGuardClient } from "./_support/mockSidecar.js";
import { OpenAiStub } from "./_support/openAiStub.js";

// ── Helpers ───────────────────────────────────────────────────────────────

/**
 * Build a `ChatOpenAI` wired against an `OpenAiStub` + `SpendGuardCallbackHandler`.
 * The `apiKey` is a synthetic value — the real OpenAI client never sees it
 * because every HTTP call routes through `stub.fetch`.
 */
function makeStubbedChat(
  stub: OpenAiStub,
  handler: SpendGuardCallbackHandler,
  overrides: { model?: string; temperature?: number } = {},
): ChatOpenAI {
  return new ChatOpenAI({
    apiKey: "sk-test-not-real",
    model: overrides.model ?? "gpt-4o-mini",
    temperature: overrides.temperature ?? 0,
    maxRetries: 0,
    callbacks: [handler],
    configuration: {
      // `openai`'s `Fetch` type uses an `unknown`-degraded `RequestInfo`
      // shim; double-cast through `unknown` is the standard escape hatch
      // documented in their README.
      fetch: stub.fetch as unknown as never,
    },
  });
}

function makeOptions(
  client: MockSpendGuardClient,
  overrides: Partial<SpendGuardCallbackHandlerOptions> = {},
): SpendGuardCallbackHandlerOptions {
  const base: SpendGuardCallbackHandlerOptions = { client: client.client };
  if (overrides.tenantId !== undefined) base.tenantId = overrides.tenantId;
  if (overrides.defaultBudgetMicrosCap !== undefined) {
    base.defaultBudgetMicrosCap = overrides.defaultBudgetMicrosCap;
  }
  return base;
}

// ── Suite: happy-path lifecycle ───────────────────────────────────────────

describe("SLICE 4 integration — happy-path lifecycle", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  // Test 1
  it("happy path: invoke → reserve → upstream stub → END → commit SUCCESS", async () => {
    const sidecar = new MockSpendGuardClient();
    const stub = new OpenAiStub({
      defaultResponse: { content: "hello back", promptTokens: 12, completionTokens: 8 },
    });
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const chat = makeStubbedChat(stub, handler);

    const response = await chat.invoke([new HumanMessage("hello")]);

    expect(typeof response.content).toBe("string");
    expect(sidecar.reserveCalls).toHaveLength(1);
    expect(sidecar.reserveCalls[0]?.request.trigger).toBe("LLM_CALL_PRE");
    expect(stub.fetchCalls).toHaveLength(1);
    expect(sidecar.commitCalls).toHaveLength(1);
    const commit = sidecar.commitCalls[0]?.request;
    expect(commit?.outcome).toBe("SUCCESS");
    expect(commit?.outcomeKind).toBe("SUCCESS");
    expect(commit?.actualInputTokensWire).toBe("12");
    expect(commit?.actualOutputTokensWire).toBe("8");
  });

  // Test 19 — verified inside test 1 too, but kept as a dedicated assertion
  // so a wire-shape regression on outcomeKind surfaces with a focused failure.
  it('commitEstimated SUCCESS wire field is `outcomeKind: "SUCCESS"` (review-standards §3.9)', async () => {
    const sidecar = new MockSpendGuardClient();
    const stub = new OpenAiStub();
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const chat = makeStubbedChat(stub, handler);

    await chat.invoke([new HumanMessage("hi")]);

    expect(sidecar.commitCalls[0]?.request.outcomeKind).toBe("SUCCESS");
    expect(sidecar.commitCalls[0]?.request.outcome).toBe("SUCCESS");
  });
});

// ── Suite: terminal-denial paths ──────────────────────────────────────────

describe("SLICE 4 integration — terminal-denial paths", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  // Test 2
  it("reserve DENY → invoke throws DecisionDenied → zero upstream calls", async () => {
    const sidecar = new MockSpendGuardClient({
      decisionQueue: [{ kind: "DENY", reasonCodes: ["BUDGET_EXCEEDED"] }],
    });
    const stub = new OpenAiStub();
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const chat = makeStubbedChat(stub, handler);

    await expect(chat.invoke([new HumanMessage("hi")])).rejects.toBeInstanceOf(DecisionDenied);

    expect(sidecar.reserveCalls).toHaveLength(1);
    expect(stub.fetchCalls).toHaveLength(0);
    expect(sidecar.commitCalls).toHaveLength(0);
  });

  // Test 13
  it("ApprovalRequired (subclass of DecisionDenied) propagates as DecisionDenied", async () => {
    const sidecar = new MockSpendGuardClient({
      decisionQueue: [{ kind: "APPROVAL_REQUIRED" }],
    });
    const stub = new OpenAiStub();
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const chat = makeStubbedChat(stub, handler);

    const err = await chat.invoke([new HumanMessage("hi")]).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApprovalRequired);
    expect(err).toBeInstanceOf(DecisionDenied);
    expect(stub.fetchCalls).toHaveLength(0);
  });

  // Test 14
  it("DecisionStopped (subclass of DecisionDenied) propagates and halts run", async () => {
    const sidecar = new MockSpendGuardClient({
      decisionQueue: [{ kind: "STOP", reasonCodes: ["projection.run.over_threshold"] }],
    });
    const stub = new OpenAiStub();
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const chat = makeStubbedChat(stub, handler);

    const err = await chat.invoke([new HumanMessage("hi")]).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(DecisionStopped);
    expect(err).toBeInstanceOf(DecisionDenied);
    expect(stub.fetchCalls).toHaveLength(0);
    expect(sidecar.commitCalls).toHaveLength(0);
  });
});

// ── Suite: provider error + sidecar degradation ──────────────────────────

describe("SLICE 4 integration — provider error + sidecar degradation", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  // Test 3
  it("reserve ALLOW + upstream 500 → handleLLMError → commit FAILURE", async () => {
    const sidecar = new MockSpendGuardClient();
    const stub = new OpenAiStub({
      defaultResponse: { errorStatus: 500, errorMessage: "synthetic upstream 500" },
    });
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const chat = makeStubbedChat(stub, handler);

    await expect(chat.invoke([new HumanMessage("hi")])).rejects.toBeDefined();

    expect(sidecar.reserveCalls).toHaveLength(1);
    expect(stub.fetchCalls).toHaveLength(1);
    expect(sidecar.commitCalls).toHaveLength(1);
    const commit = sidecar.commitCalls[0]?.request;
    expect(commit?.outcome).toBe("PROVIDER_ERROR");
    expect(commit?.outcomeKind).toBe("FAILURE");
    expect(commit?.actualErrorMessage).toBeDefined();
    expect((commit?.actualErrorMessage ?? "").length).toBeGreaterThan(0);
  });

  // Test 20 — focused FAILURE wire-shape regression net
  it("handleLLMError commit carries outcomeKind=FAILURE + non-empty actualErrorMessage", async () => {
    const sidecar = new MockSpendGuardClient();
    const stub = new OpenAiStub({
      defaultResponse: { errorStatus: 429, errorMessage: "rate limited" },
    });
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const chat = makeStubbedChat(stub, handler);

    await expect(chat.invoke([new HumanMessage("hi")])).rejects.toBeDefined();
    const commit = sidecar.commitCalls[0]?.request;
    expect(commit?.outcomeKind).toBe("FAILURE");
    expect(commit?.outcome).toBe("PROVIDER_ERROR");
    expect(commit?.actualErrorMessage).toEqual(expect.any(String));
    expect((commit?.actualErrorMessage ?? "").length).toBeGreaterThan(0);
  });

  // Test 8
  it("SidecarUnavailable on reserve → invoke proceeds; upstream still hit", async () => {
    const sidecar = new MockSpendGuardClient({
      decisionQueue: [{ kind: "SIDECAR_UNAVAILABLE", message: "UDS gone" }],
    });
    const stub = new OpenAiStub();
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const chat = makeStubbedChat(stub, handler);

    const response = await chat.invoke([new HumanMessage("hi")]);

    expect(response).toBeDefined();
    expect(sidecar.reserveCalls).toHaveLength(1);
    // Upstream stub MUST have been hit — degradation does NOT block the call.
    expect(stub.fetchCalls).toHaveLength(1);
    // No inflight slot was stashed → handleLLMEnd takes the no-op warn path.
    expect(sidecar.commitCalls).toHaveLength(0);
    expect(warnSpy).toHaveBeenCalled();
  });
});

// ── Suite: token-usage + payload extraction ──────────────────────────────

describe("SLICE 4 integration — token usage + payload shape", () => {
  // Test 7
  it("token usage from mock response shape threads onto commit actuals", async () => {
    const sidecar = new MockSpendGuardClient();
    const stub = new OpenAiStub({
      defaultResponse: { promptTokens: 37, completionTokens: 91 },
    });
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const chat = makeStubbedChat(stub, handler);

    await chat.invoke([new HumanMessage("hello world")]);

    const commit = sidecar.commitCalls[0]?.request;
    expect(commit?.actualInputTokensWire).toBe("37");
    expect(commit?.actualOutputTokensWire).toBe("91");
  });
});

// ── Suite: sequential + concurrent invocations ───────────────────────────

describe("SLICE 4 integration — sequential + concurrent invokes", () => {
  // Test 5
  it("two sequential invokes on the same handler clear inflight between calls", async () => {
    const sidecar = new MockSpendGuardClient();
    const stub = new OpenAiStub({
      responseQueue: [
        { content: "first", promptTokens: 4, completionTokens: 2 },
        { content: "second", promptTokens: 7, completionTokens: 3 },
      ],
    });
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const chat = makeStubbedChat(stub, handler);

    await chat.invoke([new HumanMessage("one")]);
    await chat.invoke([new HumanMessage("two")]);

    expect(sidecar.reserveCalls).toHaveLength(2);
    expect(sidecar.commitCalls).toHaveLength(2);
    // Inflight is private but we can prove it's empty by reading via cast —
    // identical pattern to handler.test.ts internal probe.
    const inflight = (handler as unknown as { inflight: Map<string, unknown> }).inflight;
    expect(inflight.size).toBe(0);
  });

  // Test 6
  it("three concurrent invokes maintain independent inflight slots end-to-end", async () => {
    const sidecar = new MockSpendGuardClient({
      decisionQueue: [
        { kind: "ALLOW", decisionId: "d-A", reservationId: "r-A" },
        { kind: "ALLOW", decisionId: "d-B", reservationId: "r-B" },
        { kind: "ALLOW", decisionId: "d-C", reservationId: "r-C" },
      ],
      simulatedReserveLatencyMs: 5,
    });
    const stub = new OpenAiStub({
      responseQueue: [
        { content: "A", promptTokens: 1, completionTokens: 1 },
        { content: "B", promptTokens: 2, completionTokens: 2 },
        { content: "C", promptTokens: 3, completionTokens: 3 },
      ],
    });
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const chat = makeStubbedChat(stub, handler);

    const results = await Promise.all([
      chat.invoke([new HumanMessage("a")]),
      chat.invoke([new HumanMessage("b")]),
      chat.invoke([new HumanMessage("c")]),
    ]);

    expect(results).toHaveLength(3);
    expect(sidecar.reserveCalls).toHaveLength(3);
    expect(stub.fetchCalls).toHaveLength(3);
    expect(sidecar.commitCalls).toHaveLength(3);
    // Every reserve got a distinct runId — review-standards §5.4.
    const runIds = new Set(sidecar.reserveCalls.map((c) => c.request.runId));
    expect(runIds.size).toBe(3);
    // Every commit's decisionId+reservationId pair came back from its own reserve.
    const decisionIds = new Set(sidecar.commitCalls.map((c) => c.request.decisionId));
    expect(decisionIds.size).toBe(3);
  });
});

// ── Suite: identity / idempotency / tenant propagation ───────────────────

describe("SLICE 4 integration — identity + idempotency invariants", () => {
  // Test 9
  it("parentRunId threads onto reserve.idempotencyKey derivation", async () => {
    // We can't inject a parentRunId from outside a top-level invoke (LangChain
    // assigns it inside RunManager dispatch), but we CAN prove the propagation
    // by chaining with `Runnable.invoke` from a parent runnable. The simpler
    // way: invoke the handler directly with a fixed parentRunId and assert
    // against the sidecar's recorded request. This still exercises the
    // adapter ↔ substrate boundary the SLICE 4 scope owns.
    const sidecar = new MockSpendGuardClient();
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));

    const FAKE_SERIALIZED = {
      lc: 1,
      type: "constructor",
      id: ["test"],
      kwargs: {},
    } as unknown as Parameters<typeof handler.handleChatModelStart>[0];
    const messages = [
      [
        { content: "hi" } as unknown as Parameters<
          typeof handler.handleChatModelStart
        >[1][number][number],
      ],
    ];
    const RUN_ID = "11111111-2222-3333-4444-555555555555";
    const PARENT = "00000000-1111-2222-3333-444444444444";

    await handler.handleChatModelStart(FAKE_SERIALIZED, messages, RUN_ID, PARENT);

    const req = sidecar.lastReserveRequest as ReserveRequest;
    expect(req.parentRunId).toBe(PARENT);
    expect(req.idempotencyKey).toMatch(/^sg-[0-9a-f]+$/);
  });

  // Test 10
  it("idempotency key is byte-identical across two invokes with same runId", async () => {
    const sidecarA = new MockSpendGuardClient();
    const sidecarB = new MockSpendGuardClient();
    const handlerA = new SpendGuardCallbackHandler(makeOptions(sidecarA));
    const handlerB = new SpendGuardCallbackHandler(makeOptions(sidecarB));

    const RUN_ID = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    const PARENT = "00000000-1111-2222-3333-444444444444";
    const FAKE_SERIALIZED = {
      lc: 1,
      type: "constructor",
      id: ["test"],
      kwargs: {},
    } as unknown as Parameters<typeof handlerA.handleChatModelStart>[0];
    const messages = [
      [
        { content: "anything" } as unknown as Parameters<
          typeof handlerA.handleChatModelStart
        >[1][number][number],
      ],
    ];

    await handlerA.handleChatModelStart(FAKE_SERIALIZED, messages, RUN_ID, PARENT);
    await handlerB.handleChatModelStart(FAKE_SERIALIZED, messages, RUN_ID, PARENT);

    expect(sidecarA.lastReserveRequest?.idempotencyKey).toBe(
      sidecarB.lastReserveRequest?.idempotencyKey,
    );
  });

  // Test 11
  it("opts.tenantId override propagates to the projected claim scope", async () => {
    const sidecar = new MockSpendGuardClient({ tenantId: "client-side-default" });
    const handler = new SpendGuardCallbackHandler(
      makeOptions(sidecar, { tenantId: "tenant-explicit-override" }),
    );
    const stub = new OpenAiStub();
    const chat = makeStubbedChat(stub, handler);

    await chat.invoke([new HumanMessage("hi")]);

    expect(sidecar.lastReserveRequest?.projectedClaims[0]?.scopeId).toBe(
      "tenant-explicit-override",
    );
  });

  // Test 12
  it("defaultBudgetMicrosCap is respected in the claim amount", async () => {
    const sidecar = new MockSpendGuardClient();
    const cap = 12_345_678n;
    const handler = new SpendGuardCallbackHandler(
      makeOptions(sidecar, { defaultBudgetMicrosCap: cap }),
    );
    const stub = new OpenAiStub();
    const chat = makeStubbedChat(stub, handler);

    await chat.invoke([new HumanMessage("hi")]);

    expect(sidecar.lastClaimAmountAtomic).toBe(cap);
  });

  // Test 15
  it("long content (>10KB) scales the claim amount above the small-prompt baseline", async () => {
    const sidecar = new MockSpendGuardClient();
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const stub = new OpenAiStub();
    const chat = makeStubbedChat(stub, handler);

    const longContent = "x".repeat(12_000);
    await chat.invoke([new HumanMessage(longContent)]);

    const baseSidecar = new MockSpendGuardClient();
    const baseHandler = new SpendGuardCallbackHandler(makeOptions(baseSidecar));
    const baseStub = new OpenAiStub();
    const baseChat = makeStubbedChat(baseStub, baseHandler);
    await baseChat.invoke([new HumanMessage("hi")]);

    const longAmount = sidecar.lastClaimAmountAtomic ?? 0n;
    const baseAmount = baseSidecar.lastClaimAmountAtomic ?? 0n;
    expect(longAmount).toBeGreaterThan(baseAmount);
    // Heuristic is `chars/4 * 1000` micros → ~12000/4 = 3000 tokens =
    // 3_000_000 micros. Floor at 1_000_000 to allow margin.
    expect(longAmount).toBeGreaterThanOrEqual(1_000_000n);
  });

  // Test 16
  it("near-empty content keeps claim amount at the small-prompt floor", async () => {
    const sidecar = new MockSpendGuardClient();
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const stub = new OpenAiStub();
    const chat = makeStubbedChat(stub, handler);

    await chat.invoke([new HumanMessage("")]);

    const amount = sidecar.lastClaimAmountAtomic ?? 0n;
    // Heuristic floors at 1 token → 1000 micros for empty input.
    expect(amount).toBeGreaterThan(0n);
    expect(amount).toBeLessThan(10_000n);
  });
});

// ── Suite: ChatOpenAI option propagation + handler reuse ─────────────────

describe("SLICE 4 integration — handler reuse + option pass-through", () => {
  // Test 17
  it("ChatOpenAI options (model, temperature) pass through to the request body", async () => {
    const sidecar = new MockSpendGuardClient();
    const stub = new OpenAiStub();
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const chat = makeStubbedChat(stub, handler, { model: "gpt-4o", temperature: 0.42 });

    await chat.invoke([new HumanMessage("hi")]);

    expect(stub.fetchCalls).toHaveLength(1);
    const body = stub.fetchCalls[0]?.body as { model?: string; temperature?: number };
    expect(body?.model).toBe("gpt-4o");
    expect(body?.temperature).toBe(0.42);
  });

  // Test 18
  it("handler can be reused across different ChatOpenAI instances", async () => {
    const sidecar = new MockSpendGuardClient();
    const stub = new OpenAiStub();
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));

    const chatA = makeStubbedChat(stub, handler, { model: "gpt-4o-mini" });
    const chatB = makeStubbedChat(stub, handler, { model: "gpt-4o" });

    await chatA.invoke([new HumanMessage("a")]);
    await chatB.invoke([new HumanMessage("b")]);

    expect(sidecar.reserveCalls).toHaveLength(2);
    expect(sidecar.commitCalls).toHaveLength(2);
    const bodies = stub.fetchCalls.map((c) => c.body as { model?: string });
    expect(bodies[0]?.model).toBe("gpt-4o-mini");
    expect(bodies[1]?.model).toBe("gpt-4o");
    // Review-standards §2.10 — handler is reusable + identity preserved.
    expect(handler).toBeInstanceOf(BaseCallbackHandler);
  });

  // Test 4 — covered as a focused chunked-style assertion. Streaming mid-call
  // gating is anti-scope (review-standards §10), but a single `.invoke()` is
  // semantically a "stream → END" trip the handler MUST treat as one
  // reserve + one commit.
  it("single invoke yields one reserve + one commit (streaming-style POST aggregate)", async () => {
    const sidecar = new MockSpendGuardClient();
    const stub = new OpenAiStub({
      defaultResponse: { content: "streamed-aggregate", promptTokens: 9, completionTokens: 11 },
    });
    const handler = new SpendGuardCallbackHandler(makeOptions(sidecar));
    const chat = makeStubbedChat(stub, handler);

    await chat.invoke([new HumanMessage("stream me")]);

    expect(sidecar.reserveCalls).toHaveLength(1);
    expect(sidecar.commitCalls).toHaveLength(1);
    const commit = sidecar.commitCalls[0]?.request;
    // Aggregate tokens land on the single commit (SUCCESS).
    expect(commit?.actualInputTokensWire).toBe("9");
    expect(commit?.actualOutputTokensWire).toBe("11");
  });
});
