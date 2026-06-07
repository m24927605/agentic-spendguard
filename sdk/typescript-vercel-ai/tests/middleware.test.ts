// SLICE 2 + SLICE 3 + SLICE 4 + SLICE 5 — `createSpendGuardMiddleware`
// factory + `transformParams` reserve + `wrapGenerate` commit/rollback +
// `wrapStream` TransformStream commit-after-finish tests.
//
// Scope (bundled D06/2-3-4-5 per the marathon dispatch):
//   - SLICE 2/3: factory shape, validation, `transformParams` ALLOW / DENY /
//     SidecarUnavailable, WeakMap stash discipline, idempotency-key
//     determinism. See suites named "createSpendGuardMiddleware — factory
//     shape", "transformParams — …", "WeakMap stash discipline",
//     "Idempotency-key determinism".
//   - SLICE 4: `wrapGenerate` ALLOW path commits with token actuals,
//     FAILURE path commits + re-throws, no-stash degradation passthrough,
//     token-usage shape parity (camelCase vs snake_case), end-to-end
//     transformParams → wrapGenerate lifecycle. Suite "wrapGenerate (SLICE 4)".
//   - SLICE 5: `wrapStream` ALLOW path commits after stream finish with
//     aggregated usage, FAILURE path on stream errors, no-stash passthrough,
//     accumulator across multiple chunks, end-to-end transformParams →
//     wrapStream lifecycle. Suite "wrapStream (SLICE 5)".
//
// Anti-scope (SLICE 4/5 doc):
//   - No real fetch / no mock sidecar over UDS — SLICE 6 owns provider
//     matrix testing (`@ai-sdk/openai`, `@ai-sdk/anthropic` recorded fixtures).
//   - No demo or docs assertions — SLICE 7.
//   - No tool-call gating — v0.2 (design.md §3).
//   - No DEGRADE patch application — v0.2 (design.md §3).

import {
  type BudgetClaim,
  type CommitEstimatedRequest,
  DecisionDenied,
  type DecisionOutcome,
  type ReserveRequest,
  SidecarUnavailable,
  type SpendGuardClient,
  deriveIdempotencyKey as sdkDeriveIdempotencyKey,
} from "@spendguard/sdk";
import type { LanguageModelV1, LanguageModelV1CallOptions, LanguageModelV1StreamPart } from "ai";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { _internalStashFor, createSpendGuardMiddleware } from "../src/middleware.js";
import type { SpendGuardMiddlewareOptions } from "../src/options.js";

// ── Fixtures ──────────────────────────────────────────────────────────────

const TENANT_ID = "tenant-d06-slice23-test";
const TENANT_ID_OTHER = "tenant-d06-slice23-other";

/**
 * Hand-rolled `SpendGuardClient` double. Implements only the RPCs SLICE 3
 * + SLICE 4/5 touch (`reserve`, `commitEstimated`) plus the `tenantId`
 * getter the adapter never directly reads (it relies on the LOCKED options-
 * surface `tenantId` instead). Everything else stays undefined so touching
 * it from the SLICE-3/4/5 path fails the test loudly.
 */
function makeMockClient(): SpendGuardClient {
  const mock = {
    tenantId: TENANT_ID,
    reserve: vi.fn<(req: ReserveRequest) => Promise<DecisionOutcome>>(),
    commitEstimated: vi.fn<(req: CommitEstimatedRequest) => Promise<void>>(),
  };
  mock.reserve.mockResolvedValue(makeOutcome());
  mock.commitEstimated.mockResolvedValue(undefined);
  return mock as unknown as SpendGuardClient;
}

function getMockCommit(opts: SpendGuardMiddlewareOptions): ReturnType<typeof vi.fn> {
  return (opts.client as unknown as { commitEstimated: ReturnType<typeof vi.fn> }).commitEstimated;
}

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
 * Construct a `LanguageModelV1CallOptions` reference suitable for driving
 * `transformParams`. The middleware only reads `params.prompt`; the rest
 * is filler kept in the right shape so `as unknown as` casts stay tidy.
 *
 * Each call returns a fresh object reference so WeakMap key identity tests
 * exercise distinct keys.
 */
function makeParams(promptText: string): LanguageModelV1CallOptions {
  return {
    inputFormat: "messages",
    mode: { type: "regular" },
    prompt: [
      {
        role: "user",
        content: [{ type: "text", text: promptText }],
      },
    ],
  } as unknown as LanguageModelV1CallOptions;
}

function makeOptions(
  overrides: Partial<SpendGuardMiddlewareOptions> = {},
): SpendGuardMiddlewareOptions {
  const base: SpendGuardMiddlewareOptions = {
    client: overrides.client ?? makeMockClient(),
    tenantId: overrides.tenantId ?? TENANT_ID,
  };
  if (overrides.budgetId !== undefined) {
    base.budgetId = overrides.budgetId;
  }
  return base;
}

function getMockReserve(opts: SpendGuardMiddlewareOptions): ReturnType<typeof vi.fn> {
  return (opts.client as unknown as { reserve: ReturnType<typeof vi.fn> }).reserve;
}

/**
 * Invoke `transformParams` with `type: "generate"` defaulting. SLICE 3
 * does not branch on `type` so a single helper suffices.
 */
async function callTransformParams(
  middleware: ReturnType<typeof createSpendGuardMiddleware>,
  params: LanguageModelV1CallOptions,
  type: "generate" | "stream" = "generate",
): Promise<LanguageModelV1CallOptions> {
  if (!middleware.transformParams) {
    throw new Error("test fixture: transformParams hook missing");
  }
  return middleware.transformParams({ type, params });
}

// ── Test suites ───────────────────────────────────────────────────────────

describe("createSpendGuardMiddleware — factory shape (SLICE 2)", () => {
  it("returns a LanguageModelV1Middleware-shaped object with all three hooks", () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);

    expect(mw).toBeDefined();
    expect(mw.middlewareVersion).toBe("v1");
    expect(typeof mw.transformParams).toBe("function");
    expect(typeof mw.wrapGenerate).toBe("function");
    expect(typeof mw.wrapStream).toBe("function");
  });

  it("throws when opts.client is missing", () => {
    expect(() =>
      createSpendGuardMiddleware({
        // biome-ignore lint/suspicious/noExplicitAny: deliberate bad input
        client: undefined as any,
        tenantId: TENANT_ID,
      }),
    ).toThrow(/client is required/);
  });

  it("throws when opts.tenantId is missing or empty", () => {
    const client = makeMockClient();
    expect(() =>
      createSpendGuardMiddleware({
        client,
        // biome-ignore lint/suspicious/noExplicitAny: deliberate bad input
        tenantId: undefined as any,
      }),
    ).toThrow(/tenantId is required/);

    expect(() =>
      createSpendGuardMiddleware({
        client,
        tenantId: "",
      }),
    ).toThrow(/tenantId is required/);
  });

  it("multiple factory instances are independent — no shared substrate calls", async () => {
    const optsA = makeOptions();
    const optsB = makeOptions({ tenantId: TENANT_ID_OTHER });
    const mwA = createSpendGuardMiddleware(optsA);
    const mwB = createSpendGuardMiddleware(optsB);

    const paramsA = makeParams("call A");
    const paramsB = makeParams("call B");
    await callTransformParams(mwA, paramsA);
    await callTransformParams(mwB, paramsB);

    expect(getMockReserve(optsA)).toHaveBeenCalledTimes(1);
    expect(getMockReserve(optsB)).toHaveBeenCalledTimes(1);
    // Distinct tenants → distinct claim scopes.
    const reqA = getMockReserve(optsA).mock.calls[0]?.[0] as ReserveRequest;
    const reqB = getMockReserve(optsB).mock.calls[0]?.[0] as ReserveRequest;
    expect((reqA.projectedClaims[0] as BudgetClaim).scopeId).toBe(TENANT_ID);
    expect((reqB.projectedClaims[0] as BudgetClaim).scopeId).toBe(TENANT_ID_OTHER);
  });
});

describe("transformParams — reserve ALLOW path (SLICE 3)", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("ALLOW → stashes (decisionId, reservationId) on WeakMap keyed by params", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({
        decisionId: "decision-xyz",
        reservationIds: ["reservation-xyz"],
      }),
    );

    const params = makeParams("hello world");
    const returned = await callTransformParams(mw, params);

    // Reserve was called once.
    expect(getMockReserve(opts)).toHaveBeenCalledTimes(1);
    // Stash holds the substrate-minted ids.
    const entry = _internalStashFor(params);
    expect(entry).toBeDefined();
    expect(entry?.decisionId).toBe("decision-xyz");
    expect(entry?.reservationId).toBe("reservation-xyz");
    // Returned params are the SAME reference — never a clone.
    expect(returned).toBe(params);
  });

  it("ReserveRequest carries LLM_CALL_PRE + idempotencyKey + projected claim", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    const params = makeParams("hi");
    await callTransformParams(mw, params);

    const req = getMockReserve(opts).mock.calls[0]?.[0] as ReserveRequest;
    expect(req.trigger).toBe("LLM_CALL_PRE");
    expect(req.stepId).toBe("llm_call");
    // runId / llmCallId / decisionId all collapse onto the same derived UUID
    // for SLICE 3 — mirrors D04 SLICE 3 lock.
    expect(req.runId).toEqual(req.llmCallId);
    expect(req.runId).toEqual(req.decisionId);
    expect(req.idempotencyKey).toMatch(/^sg-[0-9a-f]{32}$/);
    expect(req.projectedClaims).toHaveLength(1);
    expect(req.projectedClaims[0]?.scopeId).toBe(TENANT_ID);
    expect(req.projectedClaims[0]?.unit.unit).toBe("USD_MICROS");
  });

  it("tenantId propagates onto the idempotency-key canonical tuple", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    const params = makeParams("hi");
    await callTransformParams(mw, params);

    const req = getMockReserve(opts).mock.calls[0]?.[0] as ReserveRequest;
    // Re-derive using @spendguard/sdk's helper directly — bytes must match.
    const expectedKey = sdkDeriveIdempotencyKey({
      tenantId: TENANT_ID,
      sessionId: req.runId,
      runId: req.runId,
      stepId: "llm_call",
      llmCallId: req.runId,
      trigger: "LLM_CALL_PRE",
    });
    expect(req.idempotencyKey).toBe(expectedKey);
  });

  it("budgetId override routes the projected claim's scopeId", async () => {
    const opts = makeOptions({ budgetId: "budget-team-finance-7" });
    const mw = createSpendGuardMiddleware(opts);
    const params = makeParams("hi");
    await callTransformParams(mw, params);

    const req = getMockReserve(opts).mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims[0]?.scopeId).toBe("budget-team-finance-7");
  });

  it("no stream-vs-generate divergence — both `type` values exercise the same path", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);

    const paramsGen = makeParams("gen");
    const paramsStr = makeParams("str");
    await callTransformParams(mw, paramsGen, "generate");
    await callTransformParams(mw, paramsStr, "stream");

    expect(getMockReserve(opts)).toHaveBeenCalledTimes(2);
    // Both got stashed.
    expect(_internalStashFor(paramsGen)).toBeDefined();
    expect(_internalStashFor(paramsStr)).toBeDefined();
  });
});

describe("transformParams — error propagation (SLICE 3)", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("DecisionDenied rethrows; no stash entry", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    const denied = new DecisionDenied("budget exceeded", {
      decisionId: "d-denied",
      reasonCodes: ["BUDGET_EXCEEDED"],
    });
    getMockReserve(opts).mockRejectedValueOnce(denied);

    const params = makeParams("hi");
    await expect(callTransformParams(mw, params)).rejects.toBeInstanceOf(DecisionDenied);
    expect(_internalStashFor(params)).toBeUndefined();
  });

  it("SidecarUnavailable does NOT block — passes through with warn, no stash", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockRejectedValueOnce(new SidecarUnavailable("UDS gone"));

    const params = makeParams("hi");
    const returned = await callTransformParams(mw, params);

    expect(returned).toBe(params);
    expect(_internalStashFor(params)).toBeUndefined();
    expect(warnSpy).toHaveBeenCalledTimes(1);
    expect(String(warnSpy.mock.calls[0]?.[0])).toContain("UDS gone");
  });

  it("generic non-DecisionDenied error → pass-through (operational degradation)", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockRejectedValueOnce(new Error("transport boom"));

    const params = makeParams("hi");
    const returned = await callTransformParams(mw, params);

    expect(returned).toBe(params);
    expect(_internalStashFor(params)).toBeUndefined();
    expect(warnSpy).toHaveBeenCalledTimes(1);
    expect(String(warnSpy.mock.calls[0]?.[0])).toContain("transport boom");
  });
});

describe("WeakMap stash discipline (SLICE 2/3 / review-standards §8)", () => {
  it("two concurrent transformParams calls with distinct params keys stash independently", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts)
      .mockResolvedValueOnce(makeOutcome({ decisionId: "d-A", reservationIds: ["r-A"] }))
      .mockResolvedValueOnce(makeOutcome({ decisionId: "d-B", reservationIds: ["r-B"] }));

    const paramsA = makeParams("call A");
    const paramsB = makeParams("call B");
    await Promise.all([callTransformParams(mw, paramsA), callTransformParams(mw, paramsB)]);

    const stashA = _internalStashFor(paramsA);
    const stashB = _internalStashFor(paramsB);
    expect(stashA?.decisionId).toBe("d-A");
    expect(stashA?.reservationId).toBe("r-A");
    expect(stashB?.decisionId).toBe("d-B");
    expect(stashB?.reservationId).toBe("r-B");
    // Sanity: distinct params references → distinct stash entries.
    expect(stashA).not.toBe(stashB);
  });

  it("stash is keyed by reference, not by content — content-equal but distinct refs DO NOT alias", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts)
      .mockResolvedValueOnce(makeOutcome({ decisionId: "d-first", reservationIds: ["r-first"] }))
      .mockResolvedValueOnce(makeOutcome({ decisionId: "d-second", reservationIds: ["r-second"] }));

    // Distinct refs, same content.
    const paramsA = makeParams("same content");
    const paramsB = makeParams("same content");
    expect(paramsA).not.toBe(paramsB);

    await callTransformParams(mw, paramsA);
    await callTransformParams(mw, paramsB);

    const stashA = _internalStashFor(paramsA);
    const stashB = _internalStashFor(paramsB);
    expect(stashA?.decisionId).toBe("d-first");
    expect(stashB?.decisionId).toBe("d-second");
  });
});

describe("Idempotency-key determinism (SLICE 3 / review-standards §4.1)", () => {
  it("same prompt content + same tenant → identical idempotencyKey + runId", async () => {
    const optsA = makeOptions();
    const optsB = makeOptions();
    const mwA = createSpendGuardMiddleware(optsA);
    const mwB = createSpendGuardMiddleware(optsB);

    await callTransformParams(mwA, makeParams("identical content"));
    await callTransformParams(mwB, makeParams("identical content"));

    const reqA = getMockReserve(optsA).mock.calls[0]?.[0] as ReserveRequest;
    const reqB = getMockReserve(optsB).mock.calls[0]?.[0] as ReserveRequest;
    expect(reqA.idempotencyKey).toBe(reqB.idempotencyKey);
    expect(reqA.runId).toBe(reqB.runId);
  });

  it("different prompts → different idempotencyKey + runId", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    await callTransformParams(mw, makeParams("alpha"));
    await callTransformParams(mw, makeParams("beta"));

    const reqA = getMockReserve(opts).mock.calls[0]?.[0] as ReserveRequest;
    const reqB = getMockReserve(opts).mock.calls[1]?.[0] as ReserveRequest;
    expect(reqA.idempotencyKey).not.toBe(reqB.idempotencyKey);
    expect(reqA.runId).not.toBe(reqB.runId);
  });

  it("same content but DIFFERENT tenants → different idempotencyKey", async () => {
    const optsT1 = makeOptions({ tenantId: "tenant-1" });
    const optsT2 = makeOptions({ tenantId: "tenant-2" });
    const mwT1 = createSpendGuardMiddleware(optsT1);
    const mwT2 = createSpendGuardMiddleware(optsT2);

    await callTransformParams(mwT1, makeParams("shared"));
    await callTransformParams(mwT2, makeParams("shared"));

    const req1 = getMockReserve(optsT1).mock.calls[0]?.[0] as ReserveRequest;
    const req2 = getMockReserve(optsT2).mock.calls[0]?.[0] as ReserveRequest;
    expect(req1.idempotencyKey).not.toBe(req2.idempotencyKey);
  });
});

// ── SLICE 4 + SLICE 5 fixtures ─────────────────────────────────────────────

/** Minimal `LanguageModelV1` placeholder so the v4-middleware `model` arg is
 * a stable reference. The wrapper never inspects it; it merely needs to be
 * passed through unchanged.
 */
const FAKE_MODEL = {
  specificationVersion: "v1",
  provider: "test",
  modelId: "test-model",
} as unknown as LanguageModelV1;

/** Construct a `wrapGenerate` callbag matching the AI SDK v4 middleware shape. */
function makeGenerateArgs(
  params: LanguageModelV1CallOptions,
  doGenerate: () => Promise<unknown>,
): {
  doGenerate: () => ReturnType<LanguageModelV1["doGenerate"]>;
  doStream: () => ReturnType<LanguageModelV1["doStream"]>;
  params: LanguageModelV1CallOptions;
  model: LanguageModelV1;
} {
  return {
    doGenerate: doGenerate as () => ReturnType<LanguageModelV1["doGenerate"]>,
    doStream: (() => {
      throw new Error("doStream should not be called in wrapGenerate path");
    }) as () => ReturnType<LanguageModelV1["doStream"]>,
    params,
    model: FAKE_MODEL,
  };
}

/** Construct a `wrapStream` callbag matching the AI SDK v4 middleware shape. */
function makeStreamArgs(
  params: LanguageModelV1CallOptions,
  doStream: () => Promise<{
    stream: ReadableStream<LanguageModelV1StreamPart>;
    rawCall: { rawPrompt: unknown; rawSettings: Record<string, unknown> };
  }>,
): {
  doGenerate: () => ReturnType<LanguageModelV1["doGenerate"]>;
  doStream: () => ReturnType<LanguageModelV1["doStream"]>;
  params: LanguageModelV1CallOptions;
  model: LanguageModelV1;
} {
  return {
    doGenerate: (() => {
      throw new Error("doGenerate should not be called in wrapStream path");
    }) as () => ReturnType<LanguageModelV1["doGenerate"]>,
    doStream: doStream as () => ReturnType<LanguageModelV1["doStream"]>,
    params,
    model: FAKE_MODEL,
  };
}

/** Build a synthetic `doGenerate` result with the AI SDK v4 canonical shape. */
function makeGenerateResult(
  overrides: Partial<{
    text: string;
    promptTokens: number;
    completionTokens: number;
  }> = {},
): {
  text: string;
  finishReason: "stop";
  usage: { promptTokens: number; completionTokens: number };
  rawCall: { rawPrompt: unknown; rawSettings: Record<string, unknown> };
} {
  return {
    text: overrides.text ?? "hello",
    finishReason: "stop",
    usage: {
      promptTokens: overrides.promptTokens ?? 12,
      completionTokens: overrides.completionTokens ?? 24,
    },
    rawCall: { rawPrompt: null, rawSettings: {} },
  };
}

/**
 * Build a `ReadableStream<LanguageModelV1StreamPart>` from an array of
 * parts. Helper used across SLICE 5 streaming tests.
 */
function makePartsStream(
  parts: readonly LanguageModelV1StreamPart[],
): ReadableStream<LanguageModelV1StreamPart> {
  let i = 0;
  return new ReadableStream<LanguageModelV1StreamPart>({
    pull(controller) {
      if (i >= parts.length) {
        controller.close();
        return;
      }
      controller.enqueue(parts[i]!);
      i += 1;
    },
  });
}

/** Drain a stream and return the parts it emitted in order. */
async function drainStream<T>(stream: ReadableStream<T>): Promise<T[]> {
  const out: T[] = [];
  const reader = stream.getReader();
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      out.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  return out;
}

/** Drain a stream that is expected to throw mid-stream. */
async function drainStreamExpectError<T>(stream: ReadableStream<T>): Promise<{
  parts: T[];
  error: unknown;
}> {
  const parts: T[] = [];
  const reader = stream.getReader();
  let error: unknown;
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      parts.push(value);
    }
  } catch (err) {
    error = err;
  } finally {
    reader.releaseLock();
  }
  return { parts, error };
}

// ── SLICE 4 — wrapGenerate suite ───────────────────────────────────────────

describe("wrapGenerate (SLICE 4)", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("ALLOW + success → commitEstimated(SUCCESS) with provider-reported tokens", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({ decisionId: "dec-gen-1", reservationIds: ["res-gen-1"] }),
    );

    const params = makeParams("hello");
    await callTransformParams(mw, params);
    const inner = vi
      .fn()
      .mockResolvedValueOnce(
        makeGenerateResult({ text: "world", promptTokens: 7, completionTokens: 11 }),
      );

    const result = await mw.wrapGenerate!(makeGenerateArgs(params, inner));

    expect(inner).toHaveBeenCalledTimes(1);
    // Result returned unchanged.
    expect((result as { text: string }).text).toBe("world");

    const commit = getMockCommit(opts);
    expect(commit).toHaveBeenCalledTimes(1);
    const req = commit.mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.decisionId).toBe("dec-gen-1");
    expect(req.reservationId).toBe("res-gen-1");
    expect(req.outcome).toBe("SUCCESS");
    expect(req.outcomeKind).toBe("SUCCESS");
    expect(req.actualInputTokensWire).toBe("7");
    expect(req.actualOutputTokensWire).toBe("11");
    expect(req.stepId).toBe("llm_call");
  });

  it("ALLOW + provider error → commitEstimated(FAILURE) with err.message + re-throws", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({ decisionId: "dec-err", reservationIds: ["res-err"] }),
    );

    const params = makeParams("hi");
    await callTransformParams(mw, params);
    const providerErr = new Error("provider 503 backend down");
    const inner = vi.fn().mockRejectedValueOnce(providerErr);

    await expect(mw.wrapGenerate!(makeGenerateArgs(params, inner))).rejects.toBe(providerErr);

    const commit = getMockCommit(opts);
    expect(commit).toHaveBeenCalledTimes(1);
    const req = commit.mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.outcome).toBe("PROVIDER_ERROR");
    expect(req.outcomeKind).toBe("FAILURE");
    expect(req.actualErrorMessage).toBe("provider 503 backend down");
    expect(req.decisionId).toBe("dec-err");
    expect(req.reservationId).toBe("res-err");
  });

  it("no stash (transformParams degraded) → passthrough, no commit", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    // transformParams degrades: sidecar throws, no stash entry.
    getMockReserve(opts).mockRejectedValueOnce(new SidecarUnavailable("UDS gone"));

    const params = makeParams("hi");
    await callTransformParams(mw, params);
    expect(_internalStashFor(params)).toBeUndefined();

    const innerResult = makeGenerateResult({ text: "passed through" });
    const inner = vi.fn().mockResolvedValueOnce(innerResult);

    const result = await mw.wrapGenerate!(makeGenerateArgs(params, inner));

    expect(inner).toHaveBeenCalledTimes(1);
    expect(result).toBe(innerResult);
    expect(getMockCommit(opts)).not.toHaveBeenCalled();
  });

  it("token-usage extraction accepts AI SDK v4 canonical camelCase shape", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({ decisionId: "d", reservationIds: ["r"] }),
    );
    const params = makeParams("hi");
    await callTransformParams(mw, params);

    const inner = vi
      .fn()
      .mockResolvedValueOnce({ usage: { promptTokens: 100, completionTokens: 200 } });

    await mw.wrapGenerate!(makeGenerateArgs(params, inner));

    const req = getMockCommit(opts).mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.actualInputTokensWire).toBe("100");
    expect(req.actualOutputTokensWire).toBe("200");
  });

  it("token-usage extraction accepts snake_case provider passthrough shape", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({ decisionId: "d", reservationIds: ["r"] }),
    );
    const params = makeParams("hi");
    await callTransformParams(mw, params);

    const inner = vi
      .fn()
      .mockResolvedValueOnce({ usage: { prompt_tokens: 9, completion_tokens: 18 } });

    await mw.wrapGenerate!(makeGenerateArgs(params, inner));

    const req = getMockCommit(opts).mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.actualInputTokensWire).toBe("9");
    expect(req.actualOutputTokensWire).toBe("18");
  });

  it("missing usage field → commit with 0 actuals (defensive fallback)", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({ decisionId: "d", reservationIds: ["r"] }),
    );
    const params = makeParams("hi");
    await callTransformParams(mw, params);

    const inner = vi.fn().mockResolvedValueOnce({ text: "no usage at all" });

    await mw.wrapGenerate!(makeGenerateArgs(params, inner));

    const req = getMockCommit(opts).mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.actualInputTokensWire).toBe("0");
    expect(req.actualOutputTokensWire).toBe("0");
    expect(req.outcomeKind).toBe("SUCCESS");
  });

  it("commit-side failure does NOT corrupt the result — generate result still returned", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({ decisionId: "d", reservationIds: ["r"] }),
    );
    getMockCommit(opts).mockRejectedValueOnce(new SidecarUnavailable("commit-side gone"));

    const params = makeParams("hi");
    await callTransformParams(mw, params);
    const innerResult = makeGenerateResult({ text: "still here" });
    const inner = vi.fn().mockResolvedValueOnce(innerResult);

    const result = await mw.wrapGenerate!(makeGenerateArgs(params, inner));

    expect(result).toBe(innerResult);
    // Warned but did not throw.
    expect(warnSpy).toHaveBeenCalled();
  });

  it("concurrent wrapGenerate calls maintain independent stash lookups", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts)
      .mockResolvedValueOnce(makeOutcome({ decisionId: "d-A", reservationIds: ["r-A"] }))
      .mockResolvedValueOnce(makeOutcome({ decisionId: "d-B", reservationIds: ["r-B"] }));

    const paramsA = makeParams("call A");
    const paramsB = makeParams("call B");
    await Promise.all([callTransformParams(mw, paramsA), callTransformParams(mw, paramsB)]);

    const innerA = vi
      .fn()
      .mockResolvedValueOnce(makeGenerateResult({ promptTokens: 1, completionTokens: 2 }));
    const innerB = vi
      .fn()
      .mockResolvedValueOnce(makeGenerateResult({ promptTokens: 3, completionTokens: 4 }));

    await Promise.all([
      mw.wrapGenerate!(makeGenerateArgs(paramsA, innerA)),
      mw.wrapGenerate!(makeGenerateArgs(paramsB, innerB)),
    ]);

    const commit = getMockCommit(opts);
    expect(commit).toHaveBeenCalledTimes(2);
    const reqs = commit.mock.calls.map((c) => c[0] as CommitEstimatedRequest);
    const reqA = reqs.find((r) => r.decisionId === "d-A");
    const reqB = reqs.find((r) => r.decisionId === "d-B");
    expect(reqA?.actualInputTokensWire).toBe("1");
    expect(reqA?.actualOutputTokensWire).toBe("2");
    expect(reqB?.actualInputTokensWire).toBe("3");
    expect(reqB?.actualOutputTokensWire).toBe("4");
  });

  it("Combined: transformParams + wrapGenerate full lifecycle (reserve→generate→commit)", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    const reserve = getMockReserve(opts);
    reserve.mockResolvedValueOnce(
      makeOutcome({ decisionId: "dec-combined", reservationIds: ["res-combined"] }),
    );

    const params = makeParams("end-to-end");
    await callTransformParams(mw, params);

    // After transformParams: stash present, reserve called once.
    expect(reserve).toHaveBeenCalledTimes(1);
    const reserveReq = reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(reserveReq.trigger).toBe("LLM_CALL_PRE");
    const stash = _internalStashFor(params);
    expect(stash?.decisionId).toBe("dec-combined");
    expect(stash?.reservationId).toBe("res-combined");

    // wrapGenerate uses the stash for commit.
    const inner = vi
      .fn()
      .mockResolvedValueOnce(makeGenerateResult({ promptTokens: 30, completionTokens: 60 }));
    await mw.wrapGenerate!(makeGenerateArgs(params, inner));

    const commit = getMockCommit(opts);
    expect(commit).toHaveBeenCalledTimes(1);
    const commitReq = commit.mock.calls[0]?.[0] as CommitEstimatedRequest;
    // Same runId threads through reserve → commit.
    expect(commitReq.runId).toBe(reserveReq.runId);
    expect(commitReq.decisionId).toBe("dec-combined");
    expect(commitReq.actualInputTokensWire).toBe("30");
    expect(commitReq.actualOutputTokensWire).toBe("60");
  });

  it("Multiple sequential wrapGenerate via same middleware (distinct params)", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts)
      .mockResolvedValueOnce(makeOutcome({ decisionId: "d-1", reservationIds: ["r-1"] }))
      .mockResolvedValueOnce(makeOutcome({ decisionId: "d-2", reservationIds: ["r-2"] }))
      .mockResolvedValueOnce(makeOutcome({ decisionId: "d-3", reservationIds: ["r-3"] }));

    for (let i = 1; i <= 3; i += 1) {
      const params = makeParams(`seq-${i}`);
      await callTransformParams(mw, params);
      const inner = vi
        .fn()
        .mockResolvedValueOnce(makeGenerateResult({ promptTokens: i, completionTokens: i * 2 }));
      await mw.wrapGenerate!(makeGenerateArgs(params, inner));
    }

    expect(getMockCommit(opts)).toHaveBeenCalledTimes(3);
  });
});

// ── SLICE 5 — wrapStream suite ─────────────────────────────────────────────

describe("wrapStream (SLICE 5)", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("ALLOW + complete stream → commitEstimated(SUCCESS) with aggregated tokens", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({ decisionId: "dec-str", reservationIds: ["res-str"] }),
    );

    const params = makeParams("stream hello");
    await callTransformParams(mw, params);

    const parts: LanguageModelV1StreamPart[] = [
      { type: "text-delta", textDelta: "hello " },
      { type: "text-delta", textDelta: "world" },
      {
        type: "finish",
        finishReason: "stop",
        usage: { promptTokens: 50, completionTokens: 25 },
      },
    ];
    const inner = vi.fn().mockResolvedValueOnce({
      stream: makePartsStream(parts),
      rawCall: { rawPrompt: null, rawSettings: {} },
    });
    const wrapped = await mw.wrapStream!(makeStreamArgs(params, inner));

    // Consumer sees the full part sequence.
    const drained = await drainStream(wrapped.stream);
    expect(drained).toHaveLength(3);
    expect(drained[0]).toEqual({ type: "text-delta", textDelta: "hello " });
    expect(drained[2]?.type).toBe("finish");

    // After stream drain, commit has fired with the finish-part usage.
    // Use a microtask flush to make sure the async flush() handler has settled.
    await new Promise((resolve) => setTimeout(resolve, 0));
    const commit = getMockCommit(opts);
    expect(commit).toHaveBeenCalledTimes(1);
    const req = commit.mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.outcome).toBe("SUCCESS");
    expect(req.outcomeKind).toBe("SUCCESS");
    expect(req.actualInputTokensWire).toBe("50");
    expect(req.actualOutputTokensWire).toBe("25");
    expect(req.decisionId).toBe("dec-str");
    expect(req.reservationId).toBe("res-str");
  });

  it("ALLOW + stream error part → commitEstimated(FAILURE) with err.message", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({ decisionId: "dec-strerr", reservationIds: ["res-strerr"] }),
    );

    const params = makeParams("stream error");
    await callTransformParams(mw, params);

    const parts: LanguageModelV1StreamPart[] = [
      { type: "text-delta", textDelta: "partial " },
      { type: "error", error: new Error("upstream rate-limit") },
    ];
    const inner = vi.fn().mockResolvedValueOnce({
      stream: makePartsStream(parts),
      rawCall: { rawPrompt: null, rawSettings: {} },
    });
    const wrapped = await mw.wrapStream!(makeStreamArgs(params, inner));

    const drained = await drainStream(wrapped.stream);
    // The error part was still forwarded downstream (provider visibility).
    expect(drained).toHaveLength(2);
    expect(drained[1]?.type).toBe("error");

    await new Promise((resolve) => setTimeout(resolve, 0));
    const commit = getMockCommit(opts);
    expect(commit).toHaveBeenCalledTimes(1);
    const req = commit.mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.outcomeKind).toBe("FAILURE");
    expect(req.outcome).toBe("PROVIDER_ERROR");
    expect(req.actualErrorMessage).toContain("upstream rate-limit");
  });

  it("no stash (transformParams degraded) → passthrough, no commit", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockRejectedValueOnce(new SidecarUnavailable("gone"));

    const params = makeParams("hi");
    await callTransformParams(mw, params);
    expect(_internalStashFor(params)).toBeUndefined();

    const innerStream = makePartsStream([
      { type: "text-delta", textDelta: "raw" },
      {
        type: "finish",
        finishReason: "stop",
        usage: { promptTokens: 1, completionTokens: 1 },
      },
    ]);
    const inner = vi.fn().mockResolvedValueOnce({
      stream: innerStream,
      rawCall: { rawPrompt: null, rawSettings: {} },
    });
    const wrapped = await mw.wrapStream!(makeStreamArgs(params, inner));

    // Stream object is the inner one verbatim (passthrough — no
    // TransformStream wrapping).
    expect(wrapped.stream).toBe(innerStream);
    const drained = await drainStream(wrapped.stream);
    expect(drained).toHaveLength(2);
    expect(getMockCommit(opts)).not.toHaveBeenCalled();
  });

  it("accumulator across multiple chunks — only the final `finish` usage wins", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({ decisionId: "d", reservationIds: ["r"] }),
    );

    const params = makeParams("multi-chunk");
    await callTransformParams(mw, params);

    // Six text-delta parts then one finish — the wrapper's accumulator
    // only snapshots the finish-part usage.
    const parts: LanguageModelV1StreamPart[] = [
      { type: "text-delta", textDelta: "one " },
      { type: "text-delta", textDelta: "two " },
      { type: "text-delta", textDelta: "three " },
      { type: "text-delta", textDelta: "four " },
      { type: "text-delta", textDelta: "five " },
      { type: "text-delta", textDelta: "six" },
      {
        type: "finish",
        finishReason: "stop",
        usage: { promptTokens: 6, completionTokens: 42 },
      },
    ];
    const inner = vi.fn().mockResolvedValueOnce({
      stream: makePartsStream(parts),
      rawCall: { rawPrompt: null, rawSettings: {} },
    });
    const wrapped = await mw.wrapStream!(makeStreamArgs(params, inner));

    const drained = await drainStream(wrapped.stream);
    expect(drained).toHaveLength(7);

    await new Promise((resolve) => setTimeout(resolve, 0));
    const req = getMockCommit(opts).mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.actualInputTokensWire).toBe("6");
    expect(req.actualOutputTokensWire).toBe("42");
  });

  it("empty-stream case (no parts) → commit with 0 actuals (review-standards §3.6)", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({ decisionId: "d", reservationIds: ["r"] }),
    );
    const params = makeParams("empty");
    await callTransformParams(mw, params);

    const inner = vi.fn().mockResolvedValueOnce({
      stream: makePartsStream([]),
      rawCall: { rawPrompt: null, rawSettings: {} },
    });
    const wrapped = await mw.wrapStream!(makeStreamArgs(params, inner));
    const drained = await drainStream(wrapped.stream);
    expect(drained).toHaveLength(0);

    await new Promise((resolve) => setTimeout(resolve, 0));
    const req = getMockCommit(opts).mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.outcomeKind).toBe("SUCCESS");
    expect(req.actualInputTokensWire).toBe("0");
    expect(req.actualOutputTokensWire).toBe("0");
  });

  it("upstream throw inside the stream → FAILURE commit + error propagates downstream", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({ decisionId: "d", reservationIds: ["r"] }),
    );

    const params = makeParams("thrower");
    await callTransformParams(mw, params);

    // Stream that throws mid-flight on the second pull (vs an `error` part
    // flowing through normally). Two separate pulls so the enqueue lands
    // before the error tears the stream down.
    let pulls = 0;
    const innerStream = new ReadableStream<LanguageModelV1StreamPart>({
      pull(controller) {
        pulls += 1;
        if (pulls === 1) {
          controller.enqueue({ type: "text-delta", textDelta: "first" });
          return;
        }
        controller.error(new Error("provider socket reset"));
      },
    });
    const inner = vi.fn().mockResolvedValueOnce({
      stream: innerStream,
      rawCall: { rawPrompt: null, rawSettings: {} },
    });
    const wrapped = await mw.wrapStream!(makeStreamArgs(params, inner));

    const { parts, error } = await drainStreamExpectError(wrapped.stream);
    expect(parts.length).toBeGreaterThanOrEqual(1);
    expect(error).toBeInstanceOf(Error);
    expect((error as Error).message).toContain("provider socket reset");

    await new Promise((resolve) => setTimeout(resolve, 0));
    const commit = getMockCommit(opts);
    expect(commit).toHaveBeenCalledTimes(1);
    const req = commit.mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.outcomeKind).toBe("FAILURE");
    expect(req.actualErrorMessage).toContain("provider socket reset");
  });

  it("commit-side failure does NOT corrupt the stream — consumer still drains successfully", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({ decisionId: "d", reservationIds: ["r"] }),
    );
    getMockCommit(opts).mockRejectedValueOnce(new SidecarUnavailable("post-finish commit gone"));

    const params = makeParams("commit-side fault");
    await callTransformParams(mw, params);

    const parts: LanguageModelV1StreamPart[] = [
      { type: "text-delta", textDelta: "ok" },
      {
        type: "finish",
        finishReason: "stop",
        usage: { promptTokens: 1, completionTokens: 1 },
      },
    ];
    const inner = vi.fn().mockResolvedValueOnce({
      stream: makePartsStream(parts),
      rawCall: { rawPrompt: null, rawSettings: {} },
    });
    const wrapped = await mw.wrapStream!(makeStreamArgs(params, inner));

    const drained = await drainStream(wrapped.stream);
    expect(drained).toHaveLength(2);
    expect(drained[1]?.type).toBe("finish");

    // Commit failure surfaced via warn, not thrown.
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(warnSpy).toHaveBeenCalled();
  });

  it("Combined: transformParams + wrapStream full lifecycle (reserve→stream→commit)", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    const reserve = getMockReserve(opts);
    reserve.mockResolvedValueOnce(
      makeOutcome({ decisionId: "dec-combined-str", reservationIds: ["res-combined-str"] }),
    );

    const params = makeParams("e2e-stream");
    await callTransformParams(mw, params);
    const reserveReq = reserve.mock.calls[0]?.[0] as ReserveRequest;

    const parts: LanguageModelV1StreamPart[] = [
      { type: "text-delta", textDelta: "alpha" },
      { type: "text-delta", textDelta: "beta" },
      {
        type: "finish",
        finishReason: "stop",
        usage: { promptTokens: 100, completionTokens: 200 },
      },
    ];
    const inner = vi.fn().mockResolvedValueOnce({
      stream: makePartsStream(parts),
      rawCall: { rawPrompt: null, rawSettings: {} },
    });
    const wrapped = await mw.wrapStream!(makeStreamArgs(params, inner));
    await drainStream(wrapped.stream);

    await new Promise((resolve) => setTimeout(resolve, 0));
    const commit = getMockCommit(opts);
    expect(commit).toHaveBeenCalledTimes(1);
    const commitReq = commit.mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(commitReq.runId).toBe(reserveReq.runId);
    expect(commitReq.decisionId).toBe("dec-combined-str");
    expect(commitReq.actualInputTokensWire).toBe("100");
    expect(commitReq.actualOutputTokensWire).toBe("200");
  });

  it("Stream commits only fire once — race between finish-part and stream end", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({ decisionId: "d", reservationIds: ["r"] }),
    );
    const params = makeParams("race");
    await callTransformParams(mw, params);

    const parts: LanguageModelV1StreamPart[] = [
      {
        type: "finish",
        finishReason: "stop",
        usage: { promptTokens: 5, completionTokens: 7 },
      },
    ];
    const inner = vi.fn().mockResolvedValueOnce({
      stream: makePartsStream(parts),
      rawCall: { rawPrompt: null, rawSettings: {} },
    });
    const wrapped = await mw.wrapStream!(makeStreamArgs(params, inner));
    await drainStream(wrapped.stream);

    await new Promise((resolve) => setTimeout(resolve, 0));
    // Single commit — terminal flag prevents finish-fired-during-transform
    // and flush()-fired-on-close from both emitting.
    expect(getMockCommit(opts)).toHaveBeenCalledTimes(1);
  });

  it("Stream consumer cancel triggers FAILURE commit (release path)", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    getMockReserve(opts).mockResolvedValueOnce(
      makeOutcome({ decisionId: "d", reservationIds: ["r"] }),
    );
    const params = makeParams("cancelled");
    await callTransformParams(mw, params);

    // A stream that produces an infinite trickle until cancelled.
    let cancelled = false;
    const innerStream = new ReadableStream<LanguageModelV1StreamPart>({
      pull(controller) {
        if (cancelled) {
          controller.close();
          return;
        }
        controller.enqueue({ type: "text-delta", textDelta: "tick" });
      },
      cancel() {
        cancelled = true;
      },
    });

    const inner = vi.fn().mockResolvedValueOnce({
      stream: innerStream,
      rawCall: { rawPrompt: null, rawSettings: {} },
    });
    const wrapped = await mw.wrapStream!(makeStreamArgs(params, inner));

    // Read one chunk, then cancel.
    const reader = wrapped.stream.getReader();
    const first = await reader.read();
    expect(first.value?.type).toBe("text-delta");
    await reader.cancel(new Error("user aborted"));
    reader.releaseLock();

    await new Promise((resolve) => setTimeout(resolve, 0));
    const commit = getMockCommit(opts);
    expect(commit).toHaveBeenCalledTimes(1);
    const req = commit.mock.calls[0]?.[0] as CommitEstimatedRequest;
    expect(req.outcomeKind).toBe("FAILURE");
    expect(req.actualErrorMessage).toContain("user aborted");
  });
});
