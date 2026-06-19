// SLICE 3 — SpendGuardCallbackHandler reserve/commit wiring tests.
//
// Scope (per docs/internal/slices/COV_D04_S3_reserve_commit_wiring.md):
//   - Reserve success → inflight stashed (decisionId + reservationId).
//   - Reserve DENY → throws `DecisionDenied` (instanceof preserved).
//   - Reserve `SidecarUnavailable` → does NOT block, logs warn, no inflight.
//   - `handleLLMEnd` → `commitEstimated` SUCCESS with right tokenUsage;
//     inflight cleared.
//   - `handleLLMEnd` missing inflight → warn + no commit call.
//   - `handleLLMEnd` missing tokenUsage → commit with 0 actuals + warn.
//   - `handleLLMError` → `commitEstimated` FAILURE with err.message +
//     reservationId; inflight cleared.
//   - `handleLLMError` missing inflight → warn + no commit.
//   - 3 concurrent runs maintain independent inflight entries.
//   - Token-usage extraction handles snake_case AND camelCase variants.
//   - `idempotencyKey` derivation deterministic for same input triple.
//   - SLICE 2 patterns preserved (name, BaseCallbackHandler ancestry,
//     inflight Map shape).
//
// Anti-scope (slice doc §Anti-scope):
//   - No real fetch / no mock sidecar over UDS — SLICE 4 owns that.
//   - No streaming-specific assertions — SLICE 5.
//   - No demo or docs assertions — SLICE 5 / SLICE 6.

import { BaseCallbackHandler } from "@langchain/core/callbacks/base";
import type { Serialized } from "@langchain/core/load/serializable";
import type { BaseMessage } from "@langchain/core/messages";
import type { LLMResult } from "@langchain/core/outputs";
import {
  type CommitEstimatedRequest,
  DecisionDenied,
  type DecisionOutcome,
  type ReserveRequest,
  SidecarUnavailable,
  type SpendGuardClient,
  deriveIdempotencyKey as sdkDeriveIdempotencyKey,
} from "@spendguard/sdk";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SpendGuardCallbackHandler } from "../src/handler.js";
import type { SpendGuardCallbackHandlerOptions } from "../src/options.js";

// ── Fixtures ──────────────────────────────────────────────────────────────

const TENANT_ID = "tenant-slice3-test";
const RUN_ID_A = "11111111-2222-3333-4444-555555555555";
const RUN_ID_B = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
const RUN_ID_C = "99999999-8888-7777-6666-555555555555";
const PARENT_RUN_ID = "00000000-1111-2222-3333-444444444444";

const FAKE_SERIALIZED = {
  lc: 1,
  type: "constructor",
  id: ["test"],
  kwargs: {},
} as unknown as Serialized;

/**
 * Minimal `BaseMessage`-shaped object. The adapter only reads `.content`,
 * so a cast keeps the fixture terse without dragging in the real message
 * class hierarchy.
 */
function makeMessage(content: string): BaseMessage {
  return { content } as unknown as BaseMessage;
}

function makeMessages(...texts: string[]): BaseMessage[][] {
  return [texts.map(makeMessage)];
}

/**
 * Default `DecisionOutcome` returned by the mock `reserve()`. The adapter
 * only reads `decisionId` + `reservationIds[0]`; everything else is filler.
 */
function makeOutcome(overrides: Partial<DecisionOutcome> = {}): DecisionOutcome {
  return {
    decisionId: "decision-id-substrate-minted",
    auditDecisionEventId: "audit-evt-1",
    decision: "CONTINUE",
    mutationPatchJson: "{}",
    effectHash: new Uint8Array(0),
    ledgerTransactionId: "ledger-tx-1",
    reservationIds: ["reservation-id-substrate-minted"],
    ttlExpiresAtSeconds: 0,
    reasonCodes: [],
    matchedRuleIds: [],
    ...overrides,
  };
}

/**
 * Hand-rolled `SpendGuardClient` double. Implements the two RPCs SLICE 3
 * touches (`reserve`, `commitEstimated`) plus the `tenantId` getter the
 * adapter reads for idempotency-key derivation. Everything else is left
 * undefined — touching it from the SLICE 3 path fails the test loudly.
 */
function makeMockClient(): SpendGuardClient {
  const mock = {
    tenantId: TENANT_ID,
    reserve: vi.fn<(req: ReserveRequest) => Promise<DecisionOutcome>>(),
    commitEstimated: vi.fn<(req: CommitEstimatedRequest) => Promise<void>>(),
  };
  // Default happy-path returns; individual tests override via mockReset/mockReturnValueOnce.
  mock.reserve.mockResolvedValue(makeOutcome());
  mock.commitEstimated.mockResolvedValue(undefined);
  return mock as unknown as SpendGuardClient;
}

function makeOptions(
  overrides: Partial<SpendGuardCallbackHandlerOptions> = {},
): SpendGuardCallbackHandlerOptions {
  const base: SpendGuardCallbackHandlerOptions = {
    client: overrides.client ?? makeMockClient(),
  };
  if (overrides.tenantId !== undefined) {
    base.tenantId = overrides.tenantId;
  }
  if (overrides.defaultBudgetMicrosCap !== undefined) {
    base.defaultBudgetMicrosCap = overrides.defaultBudgetMicrosCap;
  }
  return base;
}

function getInflight(
  handler: SpendGuardCallbackHandler,
): Map<string, { decisionId: string; reservationId: string }> {
  return (
    handler as unknown as {
      inflight: Map<string, { decisionId: string; reservationId: string }>;
    }
  ).inflight;
}

function getMock(handler: SpendGuardCallbackHandler): {
  reserve: ReturnType<typeof vi.fn>;
  commitEstimated: ReturnType<typeof vi.fn>;
  tenantId: string;
} {
  return (
    handler as unknown as {
      client: {
        reserve: ReturnType<typeof vi.fn>;
        commitEstimated: ReturnType<typeof vi.fn>;
        tenantId: string;
      };
    }
  ).client;
}

// ── Test suites ───────────────────────────────────────────────────────────

describe("SpendGuardCallbackHandler — locked surface (SLICE 2 carry-over)", () => {
  it("exposes `name = 'spendguard_callback_handler'` per design.md §4", () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    expect(handler.name).toBe("spendguard_callback_handler");
  });

  it("constructor accepts a `SpendGuardCallbackHandlerOptions` object", () => {
    const opts = makeOptions();
    const handler = new SpendGuardCallbackHandler(opts);
    expect(handler).toBeDefined();
    expect(typeof handler.raiseError).toBe("boolean");
    expect(typeof handler.awaitHandlers).toBe("boolean");
  });

  it("starts with an empty `inflight` Map (no PRE has fired yet)", () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const inflight = getInflight(handler);
    expect(inflight).toBeInstanceOf(Map);
    expect(inflight.size).toBe(0);
  });

  it("`extends BaseCallbackHandler` — instance passes the LangChain identity check", () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    expect(handler).toBeInstanceOf(BaseCallbackHandler);
  });

  it("multiple instances each own their own `inflight` Map (no shared state)", () => {
    const a = new SpendGuardCallbackHandler(makeOptions());
    const b = new SpendGuardCallbackHandler(makeOptions());
    const inflightA = getInflight(a);
    const inflightB = getInflight(b);
    expect(inflightA).not.toBe(inflightB);
    inflightA.set("runid-a", { decisionId: "d", reservationId: "r" });
    expect(inflightA.size).toBe(1);
    expect(inflightB.size).toBe(0);
  });
});

describe("handleChatModelStart — reserve wiring", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("reserve success → inflight stashed with substrate decisionId + reservationId", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);
    mock.reserve.mockResolvedValueOnce(
      makeOutcome({
        decisionId: "decision-xyz",
        reservationIds: ["reservation-xyz"],
      }),
    );

    await handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hello"), RUN_ID_A);

    const inflight = getInflight(handler);
    expect(inflight.size).toBe(1);
    expect(inflight.get(RUN_ID_A)).toEqual({
      decisionId: "decision-xyz",
      reservationId: "reservation-xyz",
      // HARDEN_D05_WI — reserve-time unit + claim estimate stashed for the
      // commit path ("hello" → 2 tokens × 1000 micros).
      unit: { unit: "USD_MICROS", denomination: 1 },
      estimatedAmountAtomic: "2000",
    });
    expect(mock.reserve).toHaveBeenCalledTimes(1);
  });

  it("reserve request carries `trigger=LLM_CALL_PRE` + runId-derived ids + computed claim", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);

    await handler.handleChatModelStart(
      FAKE_SERIALIZED,
      makeMessages("hello world"),
      RUN_ID_A,
      PARENT_RUN_ID,
      undefined,
      undefined,
      undefined,
      "ChatOpenAI",
    );

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.trigger).toBe("LLM_CALL_PRE");
    expect(req.runId).toBe(RUN_ID_A);
    expect(req.stepId).toBe("llm_call");
    expect(req.llmCallId).toBe(RUN_ID_A);
    expect(req.decisionId).toBe(RUN_ID_A);
    expect(req.route).toBe("ChatOpenAI");
    expect(req.parentRunId).toBe(PARENT_RUN_ID);
    expect(req.projectedClaims).toHaveLength(1);
    expect(req.projectedClaims[0]?.scopeId).toBe(TENANT_ID);
    expect(req.projectedClaims[0]?.unit.unit).toBe("USD_MICROS");
    // Default-route fallback when `name` is undefined is exercised in
    // another test; here we assert the consumer override propagates.
  });

  it("route defaults to 'langchain-llm' when no `name` is supplied", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);

    await handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID_A);

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.route).toBe("langchain-llm");
  });

  it("reserve DENY → throws DecisionDenied (instanceof check) + no inflight stashed", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);
    const deniedErr = new DecisionDenied("budget exceeded", {
      decisionId: "d-denied",
      reasonCodes: ["BUDGET_EXCEEDED"],
    });
    mock.reserve.mockRejectedValueOnce(deniedErr);

    await expect(
      handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID_A),
    ).rejects.toBeInstanceOf(DecisionDenied);

    expect(getInflight(handler).size).toBe(0);
  });

  it("reserve SidecarUnavailable → does NOT block; logs warn; no inflight entry", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);
    mock.reserve.mockRejectedValueOnce(new SidecarUnavailable("UDS gone"));

    await expect(
      handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID_A),
    ).resolves.toBeUndefined();

    expect(getInflight(handler).size).toBe(0);
    expect(warnSpy).toHaveBeenCalledTimes(1);
    expect(String(warnSpy.mock.calls[0]?.[0])).toContain("UDS gone");
  });

  it("forwards `metadata.traceparent` onto the ReserveRequest", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);
    const traceparent = "00-aaaabbbb000000000000000000000000-cccc000000000000-01";

    await handler.handleChatModelStart(
      FAKE_SERIALIZED,
      makeMessages("hi"),
      RUN_ID_A,
      PARENT_RUN_ID,
      undefined,
      undefined,
      { traceparent },
    );

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.traceparent).toBe(traceparent);
  });

  it("ignores non-string `metadata.traceparent`", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);

    await handler.handleChatModelStart(
      FAKE_SERIALIZED,
      makeMessages("hi"),
      RUN_ID_A,
      undefined,
      undefined,
      undefined,
      { traceparent: 42 },
    );

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.traceparent).toBeUndefined();
  });

  it("opts.tenantId override wins over client.tenantId on the projected claim scope", async () => {
    const override = "tenant-override-xyz";
    const handler = new SpendGuardCallbackHandler(makeOptions({ tenantId: override }));
    const mock = getMock(handler);

    await handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID_A);

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims[0]?.scopeId).toBe(override);
  });

  it("defaultBudgetMicrosCap (when set) drives the projected claim amount", async () => {
    const cap = 5_000_000n;
    const handler = new SpendGuardCallbackHandler(makeOptions({ defaultBudgetMicrosCap: cap }));
    const mock = getMock(handler);

    await handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID_A);

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims[0]?.amountAtomic).toBe(cap.toString());
  });
});

describe("handleLLMEnd — commit SUCCESS wiring", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("commit SUCCESS called with reservationId + tokenUsage; inflight cleared", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);
    mock.reserve.mockResolvedValueOnce(makeOutcome({ decisionId: "d-1", reservationIds: ["r-1"] }));
    await handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID_A);
    expect(getInflight(handler).size).toBe(1);

    const result: LLMResult = {
      generations: [[]],
      llmOutput: { tokenUsage: { promptTokens: 17, completionTokens: 23, totalTokens: 40 } },
    };
    await handler.handleLLMEnd(result, RUN_ID_A);

    expect(mock.commitEstimated).toHaveBeenCalledTimes(1);
    const req = mock.commitEstimated.mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.runId).toBe(RUN_ID_A);
    expect(req.decisionId).toBe("d-1");
    expect(req.reservationId).toBe("r-1");
    expect(req.outcome).toBe("SUCCESS");
    expect(req.outcomeKind).toBe("SUCCESS");
    expect(req.actualInputTokensWire).toBe("17");
    expect(req.actualOutputTokensWire).toBe("23");
    expect(getInflight(handler).size).toBe(0);
  });

  it("handleLLMEnd missing inflight → warn + NO commit call", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);

    await handler.handleLLMEnd({ generations: [[]] }, "runid-never-reserved");

    expect(mock.commitEstimated).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledTimes(1);
    expect(String(warnSpy.mock.calls[0]?.[0])).toContain("no inflight entry");
  });

  it("handleLLMEnd missing tokenUsage → commit with 0 actuals + warn", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);
    await handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID_A);

    await handler.handleLLMEnd({ generations: [[]] }, RUN_ID_A);

    expect(mock.commitEstimated).toHaveBeenCalledTimes(1);
    const req = mock.commitEstimated.mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.actualInputTokensWire).toBe("0");
    expect(req.actualOutputTokensWire).toBe("0");
    expect(warnSpy).toHaveBeenCalledTimes(1);
    expect(String(warnSpy.mock.calls[0]?.[0])).toContain("no tokenUsage");
  });

  it("token-usage extraction accepts snake_case (prompt_tokens / completion_tokens)", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);
    await handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID_A);

    const result: LLMResult = {
      generations: [[]],
      llmOutput: { tokenUsage: { prompt_tokens: 11, completion_tokens: 22, total_tokens: 33 } },
    };
    await handler.handleLLMEnd(result, RUN_ID_A);

    const req = mock.commitEstimated.mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.actualInputTokensWire).toBe("11");
    expect(req.actualOutputTokensWire).toBe("22");
  });

  it("token-usage extraction accepts top-level snake_case `token_usage` envelope", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);
    await handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID_A);

    const result: LLMResult = {
      generations: [[]],
      llmOutput: { token_usage: { promptTokens: 7, completionTokens: 13 } },
    };
    await handler.handleLLMEnd(result, RUN_ID_A);

    const req = mock.commitEstimated.mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.actualInputTokensWire).toBe("7");
    expect(req.actualOutputTokensWire).toBe("13");
  });
});

describe("handleLLMError — commit FAILURE wiring", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("commit FAILURE called with err.message + reservationId; inflight cleared", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);
    mock.reserve.mockResolvedValueOnce(
      makeOutcome({ decisionId: "d-err", reservationIds: ["r-err"] }),
    );
    await handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID_A);

    await handler.handleLLMError(new Error("provider 503"), RUN_ID_A);

    expect(mock.commitEstimated).toHaveBeenCalledTimes(1);
    const req = mock.commitEstimated.mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.runId).toBe(RUN_ID_A);
    expect(req.decisionId).toBe("d-err");
    expect(req.reservationId).toBe("r-err");
    expect(req.outcome).toBe("PROVIDER_ERROR");
    expect(req.outcomeKind).toBe("FAILURE");
    expect(req.actualErrorMessage).toBe("provider 503");
    expect(getInflight(handler).size).toBe(0);
  });

  it("handleLLMError missing inflight → warn + NO commit call", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);

    await handler.handleLLMError(new Error("boom"), "runid-never-reserved");

    expect(mock.commitEstimated).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledTimes(1);
    expect(String(warnSpy.mock.calls[0]?.[0])).toContain("no inflight entry");
  });
});

describe("Concurrency + idempotency invariants", () => {
  it("three concurrent runs maintain independent inflight entries", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);
    mock.reserve
      .mockResolvedValueOnce(makeOutcome({ decisionId: "d-A", reservationIds: ["r-A"] }))
      .mockResolvedValueOnce(makeOutcome({ decisionId: "d-B", reservationIds: ["r-B"] }))
      .mockResolvedValueOnce(makeOutcome({ decisionId: "d-C", reservationIds: ["r-C"] }));

    await Promise.all([
      handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("a"), RUN_ID_A),
      handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("b"), RUN_ID_B),
      handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("c"), RUN_ID_C),
    ]);

    const inflight = getInflight(handler);
    expect(inflight.size).toBe(3);
    // HARDEN_D05_WI — inflight entries also stash the reserve-time unit +
    // claim estimate (commit-path fallback when usage is absent).
    const unit = { unit: "USD_MICROS", denomination: 1 };
    const estimatedAmountAtomic = expect.any(String);
    expect(inflight.get(RUN_ID_A)).toEqual({
      decisionId: "d-A",
      reservationId: "r-A",
      unit,
      estimatedAmountAtomic,
    });
    expect(inflight.get(RUN_ID_B)).toEqual({
      decisionId: "d-B",
      reservationId: "r-B",
      unit,
      estimatedAmountAtomic,
    });
    expect(inflight.get(RUN_ID_C)).toEqual({
      decisionId: "d-C",
      reservationId: "r-C",
      unit,
      estimatedAmountAtomic,
    });

    // Commit one runId; the other two MUST stay stashed.
    await handler.handleLLMEnd(
      {
        generations: [[]],
        llmOutput: { tokenUsage: { promptTokens: 1, completionTokens: 2 } },
      },
      RUN_ID_B,
    );
    expect(inflight.size).toBe(2);
    expect(inflight.has(RUN_ID_A)).toBe(true);
    expect(inflight.has(RUN_ID_B)).toBe(false);
    expect(inflight.has(RUN_ID_C)).toBe(true);
  });

  it("idempotencyKey derivation deterministic for same (tenantId, runId, parentRunId)", async () => {
    const opts = makeOptions();
    const handler = new SpendGuardCallbackHandler(opts);
    const mock = getMock(handler);

    await handler.handleChatModelStart(
      FAKE_SERIALIZED,
      makeMessages("hi"),
      RUN_ID_A,
      PARENT_RUN_ID,
    );
    const firstReq = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    const expected = sdkDeriveIdempotencyKey({
      tenantId: TENANT_ID,
      sessionId: PARENT_RUN_ID,
      runId: RUN_ID_A,
      stepId: "llm_call",
      llmCallId: RUN_ID_A,
      trigger: "LLM_CALL_PRE",
    });
    expect(firstReq.idempotencyKey).toBe(expected);

    // Re-run the same handshake on a fresh handler — the key MUST match.
    const handler2 = new SpendGuardCallbackHandler(makeOptions());
    const mock2 = getMock(handler2);
    await handler2.handleChatModelStart(
      FAKE_SERIALIZED,
      makeMessages("totally different content"),
      RUN_ID_A,
      PARENT_RUN_ID,
    );
    const secondReq = mock2.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(secondReq.idempotencyKey).toBe(firstReq.idempotencyKey);
  });

  it("idempotencyKey falls back to runId when parentRunId is undefined", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    const mock = getMock(handler);

    await handler.handleChatModelStart(FAKE_SERIALIZED, makeMessages("hi"), RUN_ID_A);

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    const expected = sdkDeriveIdempotencyKey({
      tenantId: TENANT_ID,
      sessionId: RUN_ID_A,
      runId: RUN_ID_A,
      stepId: "llm_call",
      llmCallId: RUN_ID_A,
      trigger: "LLM_CALL_PRE",
    });
    expect(req.idempotencyKey).toBe(expected);
  });
});
