// SLICE 2 — `withSpendGuard` factory + `SpendGuardAgentsModel` subclass +
// `runContext()` + `deriveAgentSignature()` + `extractUsage()` tests.
//
// Coverage targets (review-standards.md):
//   - §1 (Behaviour invariant — P0): PRE before INNER; DENY → inner
//     NEVER reached; commitEstimated only on CONTINUE outcomes.
//   - §3 (Public-surface lock — P0): factory + subclass signatures match
//     design.md §4 verbatim; missing opts surface TypeError.
//   - §7 (Run-context correctness — P1): AsyncLocalStorage threading;
//     `currentRunContext()` throws outside any scope; cross-subpath storage
//     equality.
//   - §10 (Error semantics — P2): DecisionDenied / SidecarUnavailable
//     propagate unchanged; commit-side failure does not corrupt inner
//     response.

import {
  type CommitEstimatedRequest,
  DecisionDenied,
  DecisionStopped,
  type ReserveRequest,
  SidecarUnavailable,
} from "@spendguard/sdk";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SpendGuardAgentsModel } from "../src/model.js";
import { currentRunContext, runContext } from "../src/runContext.js";
import { deriveAgentSignature } from "../src/signature.js";
import { extractUsage } from "../src/usage.js";
import { withSpendGuard } from "../src/withSpendGuard.js";
import { makeMockClient, makeOutcome } from "./_support/mockClient.js";
import {
  makeMockInnerModel,
  makeMockResponse,
  makeMockUsage,
  makeRequest,
} from "./_support/mockInnerModel.js";

const TENANT_ID = "tenant-d08-s2";
const TENANT_ID_OTHER = "tenant-d08-s2-other";
const RUN_ID = "01951f25-0000-7000-8000-000000000001";

// ── Factory shape ──────────────────────────────────────────────────────────

describe("withSpendGuard — factory shape (SLICE 2)", () => {
  it("returns a Model-shaped object with getResponse + getStreamedResponse", () => {
    const { client } = makeMockClient();
    const inner = makeMockInnerModel();
    const wrapped = withSpendGuard(inner, { client, tenantId: TENANT_ID });
    expect(typeof wrapped.getResponse).toBe("function");
    expect(typeof wrapped.getStreamedResponse).toBe("function");
  });

  it("throws when opts.client is missing", () => {
    const inner = makeMockInnerModel();
    expect(() =>
      withSpendGuard(inner, {
        // biome-ignore lint/suspicious/noExplicitAny: deliberate bad input
        client: undefined as any,
        tenantId: TENANT_ID,
      }),
    ).toThrow(/client is required/);
  });

  it("throws when opts.tenantId is missing or empty", () => {
    const { client } = makeMockClient();
    const inner = makeMockInnerModel();
    expect(() =>
      withSpendGuard(inner, {
        client,
        // biome-ignore lint/suspicious/noExplicitAny: deliberate bad input
        tenantId: undefined as any,
      }),
    ).toThrow(/tenantId is required/);
    expect(() => withSpendGuard(inner, { client, tenantId: "" })).toThrow(/tenantId is required/);
  });

  it("forwards getRetryAdvice when the inner model defines it", async () => {
    const { client } = makeMockClient();
    const inner = makeMockInnerModel();
    const advice = { suggested: true, retryAfterMs: 100 };
    // biome-ignore lint/suspicious/noExplicitAny: cross-shape stub
    (inner as any).getRetryAdvice = vi.fn().mockResolvedValue(advice);
    const wrapped = withSpendGuard(inner, { client, tenantId: TENANT_ID });
    expect(typeof wrapped.getRetryAdvice).toBe("function");
    const result = await wrapped.getRetryAdvice!({
      // biome-ignore lint/suspicious/noExplicitAny: minimal stub
      request: {} as any,
      error: new Error("provider"),
      stream: false,
      attempt: 1,
    });
    expect(result).toEqual(advice);
  });

  it("does NOT define getRetryAdvice when the inner model omits it", () => {
    const { client } = makeMockClient();
    const inner = makeMockInnerModel();
    const wrapped = withSpendGuard(inner, { client, tenantId: TENANT_ID });
    expect(wrapped.getRetryAdvice).toBeUndefined();
  });
});

// ── runContext / currentRunContext ─────────────────────────────────────────

describe("runContext + currentRunContext (SLICE 2 / review-standards §7)", () => {
  it("currentRunContext throws outside an active scope", () => {
    expect(() => currentRunContext()).toThrow(/outside an active runContext/);
  });

  it("currentRunContext returns the active ctx inside runContext()", async () => {
    await runContext({ runId: RUN_ID }, async () => {
      expect(currentRunContext().runId).toBe(RUN_ID);
    });
  });

  it("inner runContext wins; outer restored after inner resolves", async () => {
    await runContext({ runId: "outer" }, async () => {
      expect(currentRunContext().runId).toBe("outer");
      await runContext({ runId: "inner" }, async () => {
        expect(currentRunContext().runId).toBe("inner");
      });
      expect(currentRunContext().runId).toBe("outer");
    });
  });

  it("ctx survives awaits + setImmediate boundaries", async () => {
    await runContext({ runId: RUN_ID }, async () => {
      await Promise.resolve();
      expect(currentRunContext().runId).toBe(RUN_ID);
      await new Promise<void>((resolve) => setImmediate(resolve));
      expect(currentRunContext().runId).toBe(RUN_ID);
    });
  });
});

// ── Reserve ALLOW path ─────────────────────────────────────────────────────

describe("withSpendGuard — reserve ALLOW path (SLICE 2)", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("ALLOW → reserve called once → inner called once → commit fires SUCCESS", async () => {
    const mock = makeMockClient();
    mock.reserve.mockResolvedValueOnce(
      makeOutcome({ decisionId: "dec-A", reservationIds: ["res-A"] }),
    );
    const inner = makeMockInnerModel({
      model: "gpt-4o-mini",
      response: {
        usage: makeMockUsage({ inputTokens: 7, outputTokens: 11 }),
        responseId: "resp-A",
      },
    });
    const wrapped = withSpendGuard(inner, { client: mock.client, tenantId: TENANT_ID });

    await runContext({ runId: RUN_ID }, async () => {
      const response = await wrapped.getResponse(makeRequest({ input: "hello" }));
      // Inner returns its response verbatim.
      // biome-ignore lint/suspicious/noExplicitAny: ModelResponse runtime shape
      expect((response as any).responseId).toBe("resp-A");
    });

    expect(mock.reserve).toHaveBeenCalledTimes(1);
    expect(inner.callCount).toBe(1);
    expect(mock.commitEstimated).toHaveBeenCalledTimes(1);

    const reserveReq = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(reserveReq.trigger).toBe("LLM_CALL_PRE");
    expect(reserveReq.runId).toBe(RUN_ID);
    expect(reserveReq.route).toBe("llm.call");
    expect(reserveReq.idempotencyKey).toMatch(/^sg-[0-9a-f]{32}$/);
    expect(reserveReq.stepId).toMatch(/^01951f25-.+:oai-call:[0-9a-f]{16}$/);

    const commitReq = mock.commitEstimated.mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(commitReq.outcome).toBe("SUCCESS");
    expect(commitReq.runId).toBe(RUN_ID);
    expect(commitReq.decisionId).toBe("dec-A");
    expect(commitReq.reservationId).toBe("res-A");
    expect(commitReq.estimatedAmountAtomic).toBe("18"); // 7+11 totalled
    expect(commitReq.providerEventId).toBe("resp-A");
  });

  it("reserve fired BEFORE inner — bracket discipline (reviewer gate 1.1)", async () => {
    const mock = makeMockClient();
    const order: string[] = [];
    mock.reserve.mockImplementationOnce(async () => {
      order.push("reserve");
      return makeOutcome();
    });
    const inner = makeMockInnerModel();
    const originalGetResponse = inner.getResponse.bind(inner);
    inner.getResponse = async (req) => {
      order.push("inner");
      return originalGetResponse(req);
    };
    const wrapped = withSpendGuard(inner, { client: mock.client, tenantId: TENANT_ID });
    await runContext({ runId: RUN_ID }, () => wrapped.getResponse(makeRequest()));
    expect(order).toEqual(["reserve", "inner"]);
  });

  it("budgetId override routes the projected claim scopeId", async () => {
    const mock = makeMockClient();
    const inner = makeMockInnerModel();
    const wrapped = withSpendGuard(inner, {
      client: mock.client,
      tenantId: TENANT_ID,
      budgetId: "budget-team-7",
    });
    await runContext({ runId: RUN_ID }, () => wrapped.getResponse(makeRequest({ input: "hi" })));
    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims[0]?.scopeId).toBe("budget-team-7");
  });

  it("falls back to tenantId as scopeId when budgetId omitted", async () => {
    const mock = makeMockClient();
    const inner = makeMockInnerModel();
    const wrapped = withSpendGuard(inner, { client: mock.client, tenantId: TENANT_ID });
    await runContext({ runId: RUN_ID }, () => wrapped.getResponse(makeRequest({ input: "hi" })));
    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims[0]?.scopeId).toBe(TENANT_ID);
  });

  it("identical input → identical decisionId / llmCallId (cross-call determinism)", async () => {
    const mock = makeMockClient();
    const inner = makeMockInnerModel();
    const wrapped = withSpendGuard(inner, { client: mock.client, tenantId: TENANT_ID });
    await runContext({ runId: RUN_ID }, async () => {
      await wrapped.getResponse(makeRequest({ input: "same" }));
      await wrapped.getResponse(makeRequest({ input: "same" }));
    });
    expect(mock.reserve).toHaveBeenCalledTimes(2);
    const reqA = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    const reqB = mock.reserve.mock.calls[1]?.[0] as ReserveRequest;
    expect(reqA.decisionId).toBe(reqB.decisionId);
    expect(reqA.llmCallId).toBe(reqB.llmCallId);
    expect(reqA.idempotencyKey).toBe(reqB.idempotencyKey);
  });

  it("different inputs → different decisionId / llmCallId", async () => {
    const mock = makeMockClient();
    const inner = makeMockInnerModel();
    const wrapped = withSpendGuard(inner, { client: mock.client, tenantId: TENANT_ID });
    await runContext({ runId: RUN_ID }, async () => {
      await wrapped.getResponse(makeRequest({ input: "alpha" }));
      await wrapped.getResponse(makeRequest({ input: "beta" }));
    });
    const reqA = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    const reqB = mock.reserve.mock.calls[1]?.[0] as ReserveRequest;
    expect(reqA.decisionId).not.toBe(reqB.decisionId);
  });

  it("calling outside runContext throws and does NOT reach inner", async () => {
    const mock = makeMockClient();
    const inner = makeMockInnerModel();
    const wrapped = withSpendGuard(inner, { client: mock.client, tenantId: TENANT_ID });
    await expect(wrapped.getResponse(makeRequest())).rejects.toThrow(
      /outside an active runContext/,
    );
    expect(mock.reserve).not.toHaveBeenCalled();
    expect(inner.callCount).toBe(0);
  });

  it("skips commit when reservationIds is empty", async () => {
    const mock = makeMockClient();
    mock.reserve.mockResolvedValueOnce(makeOutcome({ reservationIds: [] }));
    const inner = makeMockInnerModel();
    const wrapped = withSpendGuard(inner, { client: mock.client, tenantId: TENANT_ID });
    await runContext({ runId: RUN_ID }, () => wrapped.getResponse(makeRequest()));
    expect(inner.callCount).toBe(1);
    expect(mock.commitEstimated).not.toHaveBeenCalled();
  });
});

// ── DENY / STOP / APPROVAL — inner NEVER reached ───────────────────────────

describe("withSpendGuard — non-CONTINUE outcomes (SLICE 2 / reviewer gate 1.3)", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("DecisionDenied rethrown; inner NEVER reached; no commit", async () => {
    const mock = makeMockClient();
    const inner = makeMockInnerModel();
    const denied = new DecisionDenied("budget exceeded", {
      decisionId: "dec-d",
      reasonCodes: ["BUDGET_EXCEEDED"],
    });
    mock.reserve.mockRejectedValueOnce(denied);
    const wrapped = withSpendGuard(inner, { client: mock.client, tenantId: TENANT_ID });
    await expect(
      runContext({ runId: RUN_ID }, () => wrapped.getResponse(makeRequest())),
    ).rejects.toBeInstanceOf(DecisionDenied);
    expect(inner.callCount).toBe(0);
    expect(mock.commitEstimated).not.toHaveBeenCalled();
  });

  it("DecisionStopped rethrown; inner NEVER reached", async () => {
    const mock = makeMockClient();
    const inner = makeMockInnerModel();
    const stopped = new DecisionStopped("projection over threshold", {
      decisionId: "dec-s",
      reasonCodes: ["projection.run.over_threshold"],
    });
    mock.reserve.mockRejectedValueOnce(stopped);
    const wrapped = withSpendGuard(inner, { client: mock.client, tenantId: TENANT_ID });
    await expect(
      runContext({ runId: RUN_ID }, () => wrapped.getResponse(makeRequest())),
    ).rejects.toBeInstanceOf(DecisionStopped);
    expect(inner.callCount).toBe(0);
  });

  it("SidecarUnavailable propagates UNCHANGED (v0.1.x does NOT degrade)", async () => {
    const mock = makeMockClient();
    const inner = makeMockInnerModel();
    const outage = new SidecarUnavailable("UDS gone");
    mock.reserve.mockRejectedValueOnce(outage);
    const wrapped = withSpendGuard(inner, { client: mock.client, tenantId: TENANT_ID });
    await expect(
      runContext({ runId: RUN_ID }, () => wrapped.getResponse(makeRequest())),
    ).rejects.toBeInstanceOf(SidecarUnavailable);
    expect(inner.callCount).toBe(0);
  });
});

// ── Provider error on inner — POST FAILURE ─────────────────────────────────

describe("withSpendGuard — provider error path (SLICE 2)", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("inner throws → commit fires PROVIDER_ERROR → error rethrows", async () => {
    const mock = makeMockClient();
    mock.reserve.mockResolvedValueOnce(makeOutcome({ decisionId: "d", reservationIds: ["r"] }));
    const inner = makeMockInnerModel();
    inner.errorToThrow = new Error("upstream 503");
    const wrapped = withSpendGuard(inner, { client: mock.client, tenantId: TENANT_ID });
    await expect(
      runContext({ runId: RUN_ID }, () => wrapped.getResponse(makeRequest())),
    ).rejects.toThrow(/upstream 503/);
    expect(inner.callCount).toBe(1);
    expect(mock.commitEstimated).toHaveBeenCalledTimes(1);
    const req = mock.commitEstimated.mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.outcome).toBe("PROVIDER_ERROR");
    expect(req.estimatedAmountAtomic).toBe("0");
  });

  it("commit-side failure does NOT corrupt the inner response", async () => {
    const mock = makeMockClient();
    mock.commitEstimated.mockRejectedValueOnce(new SidecarUnavailable("commit gone"));
    const inner = makeMockInnerModel({
      response: { usage: makeMockUsage({ inputTokens: 1, outputTokens: 2 }), responseId: "r-1" },
    });
    const wrapped = withSpendGuard(inner, { client: mock.client, tenantId: TENANT_ID });
    const response = await runContext({ runId: RUN_ID }, () => wrapped.getResponse(makeRequest()));
    // biome-ignore lint/suspicious/noExplicitAny: ModelResponse runtime shape
    expect((response as any).responseId).toBe("r-1");
    // warn fired, but no throw
  });
});

// ── Multiple wraps independent ─────────────────────────────────────────────

describe("withSpendGuard — multiple wraps independent (SLICE 2)", () => {
  it("two distinct factory instances do not share substrate calls", async () => {
    const mockA = makeMockClient(TENANT_ID);
    const mockB = makeMockClient(TENANT_ID_OTHER);
    const innerA = makeMockInnerModel();
    const innerB = makeMockInnerModel();
    const wrappedA = withSpendGuard(innerA, { client: mockA.client, tenantId: TENANT_ID });
    const wrappedB = withSpendGuard(innerB, { client: mockB.client, tenantId: TENANT_ID_OTHER });
    await runContext({ runId: RUN_ID }, async () => {
      await wrappedA.getResponse(makeRequest({ input: "A" }));
      await wrappedB.getResponse(makeRequest({ input: "B" }));
    });
    expect(mockA.reserve).toHaveBeenCalledTimes(1);
    expect(mockB.reserve).toHaveBeenCalledTimes(1);
    const reqA = mockA.reserve.mock.calls[0]?.[0] as ReserveRequest;
    const reqB = mockB.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(reqA.projectedClaims[0]?.scopeId).toBe(TENANT_ID);
    expect(reqB.projectedClaims[0]?.scopeId).toBe(TENANT_ID_OTHER);
  });
});

// ── Stream pass-through ────────────────────────────────────────────────────

describe("withSpendGuard — stream pass-through (SLICE 2 / reviewer gate 1.5)", () => {
  it("getStreamedResponse delegates to inner without PRE/POST", () => {
    const mock = makeMockClient();
    const inner = makeMockInnerModel();
    const fakeStream = (async function* () {
      yield { type: "marker" } as unknown as Awaited<
        ReturnType<typeof inner.getStreamedResponse>
      > extends AsyncIterable<infer E>
        ? E
        : never;
    })();
    inner.getStreamedResponse = vi.fn().mockReturnValue(fakeStream);
    const wrapped = withSpendGuard(inner, { client: mock.client, tenantId: TENANT_ID });
    const result = wrapped.getStreamedResponse(makeRequest());
    expect(result).toBe(fakeStream);
    expect(inner.getStreamedResponse).toHaveBeenCalledTimes(1);
    // No reserve / commit fired around the stream call — v0.1.x scope.
    expect(mock.reserve).not.toHaveBeenCalled();
    expect(mock.commitEstimated).not.toHaveBeenCalled();
  });
});

// ── SpendGuardAgentsModel parity ──────────────────────────────────────────

describe("SpendGuardAgentsModel — subclass form (SLICE 2 / reviewer gate 1.2)", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("validates opts at construction time", () => {
    const { client } = makeMockClient();
    const inner = makeMockInnerModel();
    expect(() => new SpendGuardAgentsModel({ inner, client, tenantId: "" })).toThrow(
      /tenantId is required/,
    );
    expect(
      () =>
        new SpendGuardAgentsModel({
          // biome-ignore lint/suspicious/noExplicitAny: deliberate bad input
          inner: undefined as any,
          client,
          tenantId: TENANT_ID,
        }),
    ).toThrow(/inner is required/);
  });

  it("ALLOW path matches factory shape — inner called once + commit fires", async () => {
    const mock = makeMockClient();
    mock.reserve.mockResolvedValueOnce(makeOutcome({ decisionId: "d-X", reservationIds: ["r-X"] }));
    const inner = makeMockInnerModel({
      response: { usage: makeMockUsage({ inputTokens: 3, outputTokens: 5 }), responseId: "resp-X" },
    });
    const m = new SpendGuardAgentsModel({ inner, client: mock.client, tenantId: TENANT_ID });
    const response = await runContext({ runId: RUN_ID }, () => m.getResponse(makeRequest()));
    // biome-ignore lint/suspicious/noExplicitAny: ModelResponse runtime shape
    expect((response as any).responseId).toBe("resp-X");
    expect(inner.callCount).toBe(1);
    expect(mock.commitEstimated).toHaveBeenCalledTimes(1);
  });

  it("DENY path — inner NEVER reached (parity with factory)", async () => {
    const mock = makeMockClient();
    const inner = makeMockInnerModel();
    mock.reserve.mockRejectedValueOnce(
      new DecisionDenied("nope", { decisionId: "d-deny", reasonCodes: ["X"] }),
    );
    const m = new SpendGuardAgentsModel({ inner, client: mock.client, tenantId: TENANT_ID });
    await expect(
      runContext({ runId: RUN_ID }, () => m.getResponse(makeRequest())),
    ).rejects.toBeInstanceOf(DecisionDenied);
    expect(inner.callCount).toBe(0);
  });

  it("getStreamedResponse pass-through (parity with factory)", () => {
    const mock = makeMockClient();
    const inner = makeMockInnerModel();
    const fakeStream = (async function* () {
      /* empty */
    })();
    inner.getStreamedResponse = vi.fn().mockReturnValue(fakeStream);
    const m = new SpendGuardAgentsModel({ inner, client: mock.client, tenantId: TENANT_ID });
    const result = m.getStreamedResponse(makeRequest());
    expect(result).toBe(fakeStream);
  });
});

// ── deriveAgentSignature ──────────────────────────────────────────────────

describe("deriveAgentSignature (SLICE 2)", () => {
  it("returns a 32-char lowercase hex digest", () => {
    const sig = deriveAgentSignature("hello", null);
    expect(sig).toMatch(/^[0-9a-f]{32}$/);
  });

  it("same input → same signature (deterministic)", () => {
    expect(deriveAgentSignature("hello", "be polite")).toBe(
      deriveAgentSignature("hello", "be polite"),
    );
  });

  it("different system instructions → different signature", () => {
    expect(deriveAgentSignature("hello", "A")).not.toBe(deriveAgentSignature("hello", "B"));
  });

  it("different input → different signature", () => {
    expect(deriveAgentSignature("alpha", null)).not.toBe(deriveAgentSignature("beta", null));
  });

  it("null vs undefined system instructions treated equally", () => {
    expect(deriveAgentSignature("x", null)).toBe(deriveAgentSignature("x", undefined));
  });

  it("list-of-message input rendered through JSON.stringify", () => {
    const messages = [{ role: "user", content: [{ type: "input_text", text: "hi" }] }];
    expect(deriveAgentSignature(messages, null)).toMatch(/^[0-9a-f]{32}$/);
  });
});

// ── extractUsage ──────────────────────────────────────────────────────────

describe("extractUsage (SLICE 2)", () => {
  it("camelCase shape returned as-is", () => {
    const response = makeMockResponse({
      usage: makeMockUsage({ inputTokens: 100, outputTokens: 200 }),
    });
    expect(extractUsage(response)).toEqual({
      inputTokens: 100,
      outputTokens: 200,
      totalTokens: 300,
    });
  });

  it("snake_case shape supported via fallback", () => {
    const response = {
      usage: { prompt_tokens: 50, completion_tokens: 75, total_tokens: 125 },
    } as unknown as Parameters<typeof extractUsage>[0];
    expect(extractUsage(response)).toEqual({
      inputTokens: 50,
      outputTokens: 75,
      totalTokens: 125,
    });
  });

  it("totalTokens falls back to inputTokens + outputTokens when missing", () => {
    const response = {
      usage: { inputTokens: 7, outputTokens: 13 },
    } as unknown as Parameters<typeof extractUsage>[0];
    expect(extractUsage(response).totalTokens).toBe(20);
  });

  it("missing usage → zero shape", () => {
    // ModelResponse.usage is technically required by the spec — construct
    // a runtime-valid but type-shaped-missing object via `as` cast.
    const noUsage = { output: [] } as unknown as Parameters<typeof extractUsage>[0];
    expect(extractUsage(noUsage)).toEqual({
      inputTokens: 0,
      outputTokens: 0,
      totalTokens: 0,
    });
  });

  it("null response → zero shape", () => {
    expect(extractUsage(null)).toEqual({ inputTokens: 0, outputTokens: 0, totalTokens: 0 });
  });

  it("string token counts coerce to numbers", () => {
    const response = {
      usage: { inputTokens: "10", outputTokens: "20", totalTokens: "30" },
    } as unknown as Parameters<typeof extractUsage>[0];
    expect(extractUsage(response)).toEqual({
      inputTokens: 10,
      outputTokens: 20,
      totalTokens: 30,
    });
  });
});
