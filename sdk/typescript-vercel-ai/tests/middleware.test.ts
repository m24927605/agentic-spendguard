// SLICE 2 + SLICE 3 — `createSpendGuardMiddleware` factory + `transformParams`
// reserve wiring tests.
//
// Scope (bundled D06/2 + D06/3 per the marathon dispatch):
//   - Factory returns a `LanguageModelV1Middleware`-shaped object
//     (`middlewareVersion: "v1"` + `transformParams` + `wrapGenerate` +
//     `wrapStream`).
//   - Validation: missing `client` / `tenantId` throws on construction
//     (no lazy failure inside `transformParams`).
//   - `transformParams` ALLOW path: substrate `reserve()` succeeds →
//     `(decisionId, reservationId)` stashed on a module-level WeakMap
//     keyed by the params reference itself; returned params unchanged.
//   - `transformParams` DENY path: `client.reserve()` throws
//     `DecisionDenied` → middleware rethrows; no stash entry.
//   - `transformParams` `SidecarUnavailable` path: middleware passes
//     through (warn + return params), no stash entry — review-standards
//     §7 / D04 SLICE 3 "operational degradation, not enforcement".
//   - WeakMap discipline: two concurrent calls with distinct params
//     references stash independently — no collision (review-standards §8.1).
//   - Idempotency-key determinism: same params content + same tenant →
//     same idempotency key + same derived runId (review-standards §4.1).
//   - Tenant propagation: `tenantId` reaches the substrate as the projected
//     claim's `scopeId` (and the first field of the idempotency-key tuple).
//   - `wrapGenerate` / `wrapStream` SLICE 2/3 stubs throw a clear
//     "not implemented" signal so a SLICE-2/3 build doesn't silently
//     succeed in production.
//   - Multiple factory instances are independent (no shared state via the
//     module-level WeakMap key clobbering, since the keys are the params
//     references themselves).
//
// Anti-scope (SLICE 2/3 doc):
//   - No real fetch / no mock sidecar over UDS — SLICE 6 owns provider
//     matrix testing.
//   - No streaming-specific assertions — SLICE 5.
//   - No demo or docs assertions — SLICE 7.

import {
  type BudgetClaim,
  DecisionDenied,
  type DecisionOutcome,
  type ReserveRequest,
  SidecarUnavailable,
  type SpendGuardClient,
  deriveIdempotencyKey as sdkDeriveIdempotencyKey,
} from "@spendguard/sdk";
import type { LanguageModelV1CallOptions } from "ai";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { _internalStashFor, createSpendGuardMiddleware } from "../src/middleware.js";
import type { SpendGuardMiddlewareOptions } from "../src/options.js";

// ── Fixtures ──────────────────────────────────────────────────────────────

const TENANT_ID = "tenant-d06-slice23-test";
const TENANT_ID_OTHER = "tenant-d06-slice23-other";

/**
 * Hand-rolled `SpendGuardClient` double. Implements only the RPCs SLICE 3
 * touches (`reserve`) plus the `tenantId` getter the adapter never directly
 * reads (it relies on the LOCKED options-surface `tenantId` instead).
 * Everything else stays undefined so touching it from the SLICE-3 path
 * fails the test loudly.
 */
function makeMockClient(): SpendGuardClient {
  const mock = {
    tenantId: TENANT_ID,
    reserve: vi.fn<(req: ReserveRequest) => Promise<DecisionOutcome>>(),
  };
  mock.reserve.mockResolvedValue(makeOutcome());
  return mock as unknown as SpendGuardClient;
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

describe("wrapGenerate / wrapStream stubs (SLICE 2/3 — deferred to 4/5)", () => {
  it("wrapGenerate throws SpendGuardMiddlewareNotImplemented (SLICE 4 deferred)", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    expect(mw.wrapGenerate).toBeDefined();
    await expect(
      // biome-ignore lint/suspicious/noExplicitAny: stub doesn't use args
      (mw.wrapGenerate as any)({} as any),
    ).rejects.toThrow(/wrapGenerate is not implemented/);
  });

  it("wrapStream throws SpendGuardMiddlewareNotImplemented (SLICE 5 deferred)", async () => {
    const opts = makeOptions();
    const mw = createSpendGuardMiddleware(opts);
    expect(mw.wrapStream).toBeDefined();
    await expect(
      // biome-ignore lint/suspicious/noExplicitAny: stub doesn't use args
      (mw.wrapStream as any)({} as any),
    ).rejects.toThrow(/wrapStream is not implemented/);
  });
});
