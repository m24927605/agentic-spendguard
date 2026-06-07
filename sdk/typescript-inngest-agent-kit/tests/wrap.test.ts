// SLICE 2+3 — `wrapWithSpendGuard` factory + reserve / commit / retry-dedup
// wiring tests.
//
// Coverage map vs. tests.md §3.1 / §3.2 / §3.3 / §3.4:
//
//   wrap.test.ts (this file):
//     - W-01 .. W-17 (factory + reserve/commit unit tests)
//     - R-01 .. R-08 (retry-dedup contract — the headline gate)
//     - E-01 .. E-10 (error propagation)
//     - I-01 .. I-07 (identity-derivation invariants)
//     - X-01 .. X-08 (token-usage extract probe order)
//
// The bundled SLICE 2+3 ships with a single wrap.test.ts so the headline
// retry-dedup contract lives next to its bracket. Later slices may split
// into the layout `tests.md §1` suggests; the substance + assertions are
// 1:1 with the spec.

import {
  ApprovalRequired,
  DecisionDenied,
  type DecisionOutcome,
  DecisionSkipped,
  DecisionStopped,
  InMemoryIdempotencyCache,
  type ReserveRequest,
  SidecarUnavailable,
  type SpendGuardClient,
  deriveIdempotencyKey as sdkDeriveIdempotencyKey,
  deriveUuidFromSignature as sdkDeriveUuidFromSignature,
} from "@spendguard/sdk";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { deriveIdentity, deriveStepIdempotencyKey } from "../src/ids.js";
import type {
  ClaimEstimator,
  ClaimEstimatorInput,
  WrapWithSpendGuardOptions,
} from "../src/options.js";
import { wrapWithSpendGuard } from "../src/wrapWithSpendGuard.js";
import { makeMockStepAi, makeRuntimeCtx } from "./_support/mockAgentKit.js";
import { makeMockClient, makeOutcome } from "./_support/mockClient.js";

const TENANT_ID = "tenant-d29-test";
const RUN_ID = "11111111-2222-3333-4444-555555555555";
const STEP_ID = "step-d29-llm-call";
const BUDGET_ID = "budget-d29-test";

function makeOptions(
  overrides: Partial<WrapWithSpendGuardOptions> = {},
): WrapWithSpendGuardOptions {
  return {
    tenantId: TENANT_ID,
    budgetId: BUDGET_ID,
    claimEstimator: () => [
      {
        scopeId: BUDGET_ID,
        amountAtomic: "1000000",
        unit: { unit: "USD_MICROS", denomination: 1 },
      },
    ],
    ...overrides,
  };
}

// ───────────────────────────────────────────────────────────────────────────
// Locked surface — review-standards §1.
// ───────────────────────────────────────────────────────────────────────────

describe("@spendguard/inngest-agent-kit locked surface", () => {
  it("exports VERSION as a non-empty semver-shaped string", async () => {
    const mod = await import("../src/index.js");
    expect(typeof mod.VERSION).toBe("string");
    expect(mod.VERSION).toMatch(/^\d+\.\d+\.\d+/);
  });

  it("exposes wrapWithSpendGuard as the headline factory", async () => {
    const mod = await import("../src/index.js");
    expect(typeof mod.wrapWithSpendGuard).toBe("function");
  });

  it("re-exports the locked typed errors so consumers do not double-import @spendguard/sdk", async () => {
    const mod = await import("../src/index.js");
    expect(mod.DecisionDenied).toBe(DecisionDenied);
    expect(mod.DecisionStopped).toBe(DecisionStopped);
    expect(mod.DecisionSkipped).toBe(DecisionSkipped);
    expect(mod.ApprovalRequired).toBe(ApprovalRequired);
    expect(mod.SidecarUnavailable).toBe(SidecarUnavailable);
  });

  it("has no default export — review-standards §1.6", async () => {
    const mod = await import("../src/index.js");
    expect((mod as { default?: unknown }).default).toBeUndefined();
  });
});

// ───────────────────────────────────────────────────────────────────────────
// W-01..W-17 — factory + reserve/commit unit tests
// ───────────────────────────────────────────────────────────────────────────

describe("wrapWithSpendGuard.infer — reserve + commit happy path", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("W-01: fires client.reserve with trigger=LLM_CALL_PRE", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient(TENANT_ID);
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await sg.infer(
      "call-openai",
      { model: { kind: "openai" }, body: { messages: [] } },
      makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID, attempt: 0 } }),
    );

    expect(mock.reserve).toHaveBeenCalledTimes(1);
    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.trigger).toBe("LLM_CALL_PRE");
  });

  it("W-02: reserve fires BEFORE the inner stepAi.infer body runs", async () => {
    const events: string[] = [];
    const { stepAi } = makeMockStepAi({
      inferReturns: () => {
        events.push("provider");
        return { usage: { total_tokens: 1 } };
      },
    });
    const mock = makeMockClient();
    mock.reserve.mockImplementation(async () => {
      events.push("reserve");
      return makeOutcome();
    });
    mock.commitEstimated.mockImplementation(async () => {
      events.push("commit");
    });
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
    );

    // Strict in-order sequencing: reserve, provider, commit.
    expect(events).toEqual(["reserve", "provider", "commit"]);
  });

  it("W-03/W-04: llmCallId === stepId === ctx.step.id", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
    );

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.llmCallId).toBe(STEP_ID);
    expect(req.stepId).toBe(STEP_ID);
  });

  it("W-05: decisionId === deriveUuidFromSignature(seed, {scope:'decision_id'}) — seed = idempotencyKey ?? stepId", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    // Case A: explicit Inngest idempotency key wins.
    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({
        runId: RUN_ID,
        step: { id: STEP_ID, idempotencyKey: "inngest-key-A" },
      }),
    );
    const reqA = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(reqA.decisionId).toBe(
      sdkDeriveUuidFromSignature("inngest-key-A", { scope: "decision_id" }),
    );

    // Case B: fallback to stepId.
    mock.reserve.mockClear();
    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
    );
    const reqB = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(reqB.decisionId).toBe(sdkDeriveUuidFromSignature(STEP_ID, { scope: "decision_id" }));
  });

  it("W-06: idempotencyKey byte-matches sdkDeriveIdempotencyKey({...})", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID, attempt: 0 } }),
    );

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    const expected = sdkDeriveIdempotencyKey({
      tenantId: TENANT_ID,
      sessionId: RUN_ID,
      runId: RUN_ID,
      stepId: STEP_ID,
      llmCallId: STEP_ID,
      trigger: "LLM_CALL_PRE",
    });
    expect(req.idempotencyKey).toBe(expected);
  });

  it("W-07: claimEstimator(input) invoked exactly once per infer call", async () => {
    const estimator = vi.fn<ClaimEstimator>(() => [
      {
        scopeId: BUDGET_ID,
        amountAtomic: "1000000",
        unit: { unit: "USD_MICROS", denomination: 1 },
      },
    ]);
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ claimEstimator: estimator }));

    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
    );

    expect(estimator).toHaveBeenCalledTimes(1);
  });

  it("W-08: claimEstimator receives {stepId, attempt, runId, model, body, eventId, inngestIdempotencyKey}", async () => {
    const estimator = vi.fn<ClaimEstimator>(() => [
      {
        scopeId: BUDGET_ID,
        amountAtomic: "1",
        unit: { unit: "USD_MICROS", denomination: 1 },
      },
    ]);
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ claimEstimator: estimator }));

    const model = { kind: "openai", id: "gpt-4o-mini" };
    const body = { messages: [{ role: "user", content: "hi" }] };
    await sg.infer(
      "call",
      { model, body },
      makeRuntimeCtx({
        runId: RUN_ID,
        eventId: "evt-X",
        step: { id: STEP_ID, attempt: 2, idempotencyKey: "i-key" },
      }),
    );

    const arg = estimator.mock.calls[0]?.[0];
    expect(arg).toMatchObject({
      stepId: STEP_ID,
      attempt: 2,
      runId: RUN_ID,
      eventId: "evt-X",
      inngestIdempotencyKey: "i-key",
      model,
      body,
    });
  });

  it("W-09: runtimeCtx undefined → degrades to name as stepId + empty runId", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await sg.infer("name-as-stepid", { model: {}, body: {} });

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.stepId).toBe("name-as-stepid");
    expect(req.runId).toBe("");
  });

  it("W-10: route defaults to 'llm.call.inngest'; consumer override propagates", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());
    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
    );
    expect((mock.reserve.mock.calls[0]?.[0] as ReserveRequest).route).toBe("llm.call.inngest");

    mock.reserve.mockClear();
    const sg2 = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ route: "custom.route" }));
    await sg2.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
    );
    expect((mock.reserve.mock.calls[0]?.[0] as ReserveRequest).route).toBe("custom.route");
  });

  it("W-11: commitEstimated fires AFTER infer with outcome=SUCCESS, usage from extractTotalTokens(result)", async () => {
    const { stepAi } = makeMockStepAi({
      inferReturns: () => ({
        id: "chatcmpl-W11",
        usage: { total_tokens: 137 },
      }),
    });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
    );

    expect(mock.commitEstimated).toHaveBeenCalledTimes(1);
    const req = mock.commitEstimated.mock.calls[0]?.[0];
    expect(req?.outcome).toBe("SUCCESS");
    expect(req?.estimatedAmountAtomic).toBe("137");
  });

  it("W-12: providerEventId on commit comes from extractProviderEventId(result)", async () => {
    const { stepAi } = makeMockStepAi({
      inferReturns: () => ({ id: "chatcmpl-evt-id" }),
    });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
    );

    expect(mock.commitEstimated.mock.calls[0]?.[0]?.providerEventId).toBe("chatcmpl-evt-id");
  });

  it("W-13: sg.wrap(name, fn, ...args) also fires reserve before fn runs and commit after", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    const fn = vi.fn(async () => ({ id: "wrap-result", usage: { total_tokens: 5 } }));
    await sg.wrap("wrap-step", fn);

    expect(mock.reserve).toHaveBeenCalledTimes(1);
    expect(mock.commitEstimated).toHaveBeenCalledTimes(1);
    expect(mock.commitEstimated.mock.calls[0]?.[0]?.outcome).toBe("SUCCESS");
  });

  it("W-15: two concurrent infer calls with different step.ids do not cross-correlate", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    const [a, b] = await Promise.all([
      sg.infer(
        "A",
        { model: {}, body: {} },
        makeRuntimeCtx({ runId: RUN_ID, step: { id: "step-A" } }),
      ),
      sg.infer(
        "B",
        { model: {}, body: {} },
        makeRuntimeCtx({ runId: RUN_ID, step: { id: "step-B" } }),
      ),
    ]);

    expect(a).toBeDefined();
    expect(b).toBeDefined();
    expect(mock.reserve).toHaveBeenCalledTimes(2);
    const reservedStepIds = mock.reserve.mock.calls.map((c) => (c[0] as ReserveRequest).stepId);
    expect(new Set(reservedStepIds)).toEqual(new Set(["step-A", "step-B"]));
  });

  it("W-16: adapter does NOT mutate the options object passed in", async () => {
    const opts = makeOptions();
    // Capture a stable snapshot of consumer-facing fields.
    const snapshot = { ...opts };
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, opts);
    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
    );
    expect(opts).toEqual(snapshot);
  });

  it("W-17: claimEstimator input.attempt = 0 on first execution", async () => {
    const estimator = vi.fn<ClaimEstimator>(() => []);
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ claimEstimator: estimator }));
    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID, attempt: 0 } }),
    );
    expect(estimator.mock.calls[0]?.[0]?.attempt).toBe(0);
  });
});

// ───────────────────────────────────────────────────────────────────────────
// R-01..R-08 — retry-dedup headline contract
// ───────────────────────────────────────────────────────────────────────────

describe("wrapWithSpendGuard — retry-dedup contract (review-standards §4)", () => {
  it("R-01: same step.id + same inngestIdempotencyKey → same idempotencyKey", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({
        runId: RUN_ID,
        step: { id: STEP_ID, attempt: 0, idempotencyKey: "K1" },
      }),
    );
    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({
        runId: RUN_ID,
        step: { id: STEP_ID, attempt: 0, idempotencyKey: "K1" },
      }),
    );

    const k1 = (mock.reserve.mock.calls[0]?.[0] as ReserveRequest).idempotencyKey;
    const k2 = (mock.reserve.mock.calls[1]?.[0] as ReserveRequest).idempotencyKey;
    expect(k1).toBe(k2);
  });

  it("R-02: same step.id + same idempotencyKey + DIFFERENT attempt → same idempotencyKey (attempt-invariance)", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({
        runId: RUN_ID,
        step: { id: STEP_ID, attempt: 0, idempotencyKey: "K1" },
      }),
    );
    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({
        runId: RUN_ID,
        step: { id: STEP_ID, attempt: 1, idempotencyKey: "K1" },
      }),
    );
    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({
        runId: RUN_ID,
        step: { id: STEP_ID, attempt: 7, idempotencyKey: "K1" },
      }),
    );

    const k0 = (mock.reserve.mock.calls[0]?.[0] as ReserveRequest).idempotencyKey;
    const k1 = (mock.reserve.mock.calls[1]?.[0] as ReserveRequest).idempotencyKey;
    const k7 = (mock.reserve.mock.calls[2]?.[0] as ReserveRequest).idempotencyKey;
    expect(k0).toBe(k1);
    expect(k1).toBe(k7);
  });

  it("R-03: with idempotencyCache, 3 retries hit reserve EXACTLY ONCE", async () => {
    const cache = new InMemoryIdempotencyCache();
    const { stepAi } = makeMockStepAi({ throwOnAttempts: [0, 1] });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ idempotencyCache: cache }));

    // Attempt 0 — provider throws → reserve fires, commit PROVIDER_ERROR.
    await expect(
      sg.infer(
        "call",
        { model: {}, body: {} },
        makeRuntimeCtx({
          runId: RUN_ID,
          step: { id: STEP_ID, attempt: 0, idempotencyKey: "K1" },
        }),
      ),
    ).rejects.toThrow(/provider-error-attempt-0/);
    // Attempt 1 — provider throws → reserve SKIPPED (cache hit), commit PROVIDER_ERROR.
    await expect(
      sg.infer(
        "call",
        { model: {}, body: {} },
        makeRuntimeCtx({
          runId: RUN_ID,
          step: { id: STEP_ID, attempt: 1, idempotencyKey: "K1" },
        }),
      ),
    ).rejects.toThrow(/provider-error-attempt-1/);
    // Attempt 2 — succeeds. reserve still SKIPPED (cache hit).
    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({
        runId: RUN_ID,
        step: { id: STEP_ID, attempt: 2, idempotencyKey: "K1" },
      }),
    );

    expect(mock.reserve).toHaveBeenCalledTimes(1);
  });

  it("R-04: attempt 0 commits PROVIDER_ERROR; attempt 1 succeeds with SAME decision_id", async () => {
    const cache = new InMemoryIdempotencyCache();
    const { stepAi } = makeMockStepAi({ throwOnAttempts: [0] });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ idempotencyCache: cache }));

    await expect(
      sg.infer(
        "call",
        { model: {}, body: {} },
        makeRuntimeCtx({
          runId: RUN_ID,
          step: { id: STEP_ID, attempt: 0, idempotencyKey: "K1" },
        }),
      ),
    ).rejects.toThrow();
    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({
        runId: RUN_ID,
        step: { id: STEP_ID, attempt: 1, idempotencyKey: "K1" },
      }),
    );

    expect(mock.commitEstimated).toHaveBeenCalledTimes(2);
    const c0 = mock.commitEstimated.mock.calls[0]?.[0];
    const c1 = mock.commitEstimated.mock.calls[1]?.[0];
    expect(c0?.outcome).toBe("PROVIDER_ERROR");
    expect(c1?.outcome).toBe("SUCCESS");
    // Same decision id across both commits — cached outcome flows through.
    expect(c0?.decisionId).toBe(c1?.decisionId);
  });

  it("R-05: missing inngestIdempotencyKey → step.id is the seed; dedup still works", async () => {
    const cache = new InMemoryIdempotencyCache();
    const { stepAi } = makeMockStepAi({ throwOnAttempts: [0] });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ idempotencyCache: cache }));

    await expect(
      sg.infer(
        "call",
        { model: {}, body: {} },
        makeRuntimeCtx({
          runId: RUN_ID,
          step: { id: STEP_ID, attempt: 0 },
        }),
      ),
    ).rejects.toThrow();
    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({
        runId: RUN_ID,
        step: { id: STEP_ID, attempt: 1 },
      }),
    );

    // Reserve fires ONCE; the cache returned the cached outcome on attempt 1.
    expect(mock.reserve).toHaveBeenCalledTimes(1);
  });

  it("R-06: WITHOUT idempotencyCache, both attempts hit the sidecar but BYTE-IDENTICAL idempotencyKey lets the sidecar dedup", async () => {
    const { stepAi } = makeMockStepAi({ throwOnAttempts: [0] });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await expect(
      sg.infer(
        "call",
        { model: {}, body: {} },
        makeRuntimeCtx({
          runId: RUN_ID,
          step: { id: STEP_ID, attempt: 0, idempotencyKey: "K" },
        }),
      ),
    ).rejects.toThrow();
    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({
        runId: RUN_ID,
        step: { id: STEP_ID, attempt: 1, idempotencyKey: "K" },
      }),
    );

    // Both attempts hit the sidecar (no in-process cache),
    // but the wire idempotencyKey is byte-identical → sidecar dedup catches it.
    expect(mock.reserve).toHaveBeenCalledTimes(2);
    const k0 = (mock.reserve.mock.calls[0]?.[0] as ReserveRequest).idempotencyKey;
    const k1 = (mock.reserve.mock.calls[1]?.[0] as ReserveRequest).idempotencyKey;
    expect(k0).toBe(k1);
  });

  it("R-08: a NEW Inngest function invocation (NEW runId) produces a DIFFERENT idempotencyKey", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({
        runId: "run-A",
        step: { id: STEP_ID, attempt: 0, idempotencyKey: "shared" },
      }),
    );
    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({
        runId: "run-B",
        step: { id: STEP_ID, attempt: 0, idempotencyKey: "shared" },
      }),
    );

    const kA = (mock.reserve.mock.calls[0]?.[0] as ReserveRequest).idempotencyKey;
    const kB = (mock.reserve.mock.calls[1]?.[0] as ReserveRequest).idempotencyKey;
    expect(kA).not.toBe(kB);
  });
});

// ───────────────────────────────────────────────────────────────────────────
// E-01..E-10 — error propagation
// ───────────────────────────────────────────────────────────────────────────

describe("wrapWithSpendGuard — error propagation (review-standards §5)", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("E-01/E-02: DecisionStopped propagates AND inner stepAi.infer is NEVER invoked", async () => {
    const { stepAi, inferBody } = makeMockStepAi();
    const mock = makeMockClient();
    mock.reserve.mockRejectedValueOnce(
      new DecisionStopped("hard stop", {
        decisionId: "d-stop",
        reasonCodes: ["STOP_POLICY"],
      }),
    );
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await expect(
      sg.infer(
        "call",
        { model: {}, body: {} },
        makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
      ),
    ).rejects.toBeInstanceOf(DecisionStopped);
    expect(inferBody).not.toHaveBeenCalled();
    expect(mock.commitEstimated).not.toHaveBeenCalled();
  });

  it("E-03: DecisionDenied propagates", async () => {
    const { stepAi, inferBody } = makeMockStepAi();
    const mock = makeMockClient();
    mock.reserve.mockRejectedValueOnce(
      new DecisionDenied("budget exhausted", {
        decisionId: "d-deny",
        reasonCodes: ["BUDGET_EXCEEDED"],
      }),
    );
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await expect(
      sg.infer(
        "call",
        { model: {}, body: {} },
        makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
      ),
    ).rejects.toBeInstanceOf(DecisionDenied);
    expect(inferBody).not.toHaveBeenCalled();
  });

  it("E-04: ApprovalRequired without onApprovalRequired propagates", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    mock.reserve.mockRejectedValueOnce(
      new ApprovalRequired("manual approval", {
        decisionId: "d-appr",
        reasonCodes: ["APPROVAL_REQUIRED"],
        approvalRequestId: "appr-req-1",
      }),
    );
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());
    await expect(
      sg.infer(
        "call",
        { model: {}, body: {} },
        makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
      ),
    ).rejects.toBeInstanceOf(ApprovalRequired);
  });

  it("E-05: ApprovalRequired with onApprovalRequired returning DecisionOutcome continues", async () => {
    const resumed: DecisionOutcome = makeOutcome({
      decisionId: "d-resumed",
      reservationIds: ["r-resumed"],
    });
    const { stepAi, inferBody } = makeMockStepAi();
    const mock = makeMockClient();
    const apprErr = new ApprovalRequired("needs approval", {
      decisionId: "d-appr",
      reasonCodes: ["APPROVAL_REQUIRED"],
      approvalRequestId: "appr-req-1",
    });
    mock.reserve.mockRejectedValueOnce(apprErr);
    const onApprovalRequired = vi.fn(async () => resumed);
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ onApprovalRequired }));

    await sg.infer(
      "call",
      { model: {}, body: {} },
      makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
    );
    expect(onApprovalRequired).toHaveBeenCalledTimes(1);
    expect(inferBody).toHaveBeenCalledTimes(1);
    // Commit MUST carry the resumed decision id.
    expect(mock.commitEstimated.mock.calls[0]?.[0]?.decisionId).toBe("d-resumed");
  });

  it("E-06: ApprovalRequired with onApprovalRequired returning null propagates original error", async () => {
    const { stepAi, inferBody } = makeMockStepAi();
    const mock = makeMockClient();
    const apprErr = new ApprovalRequired("needs approval", {
      decisionId: "d-appr",
      reasonCodes: ["APPROVAL_REQUIRED"],
      approvalRequestId: "appr-req-1",
    });
    mock.reserve.mockRejectedValueOnce(apprErr);
    const onApprovalRequired = vi.fn(async () => null);
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ onApprovalRequired }));

    await expect(
      sg.infer(
        "call",
        { model: {}, body: {} },
        makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
      ),
    ).rejects.toBe(apprErr);
    expect(inferBody).not.toHaveBeenCalled();
  });

  it("E-07: SidecarUnavailable propagates as-is (strict mode)", async () => {
    const { stepAi, inferBody } = makeMockStepAi();
    const mock = makeMockClient();
    mock.reserve.mockRejectedValueOnce(new SidecarUnavailable("UDS gone"));
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await expect(
      sg.infer(
        "call",
        { model: {}, body: {} },
        makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
      ),
    ).rejects.toBeInstanceOf(SidecarUnavailable);
    expect(inferBody).not.toHaveBeenCalled();
  });

  it("E-08: claimEstimator throwing propagates through infer", async () => {
    const boom = new Error("estimator-boom");
    const estimator = vi.fn<ClaimEstimator>(() => {
      throw boom;
    });
    const { stepAi, inferBody } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ claimEstimator: estimator }));

    await expect(
      sg.infer(
        "call",
        { model: {}, body: {} },
        makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
      ),
    ).rejects.toBe(boom);
    expect(inferBody).not.toHaveBeenCalled();
  });

  it("E-09: provider error → commit(PROVIDER_ERROR, 0) fires THEN re-throws", async () => {
    const { stepAi } = makeMockStepAi({ throwOnAttempts: [0] });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await expect(
      sg.infer(
        "call",
        { model: {}, body: {} },
        makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID, attempt: 0 } }),
      ),
    ).rejects.toThrow(/provider-error-attempt-0/);

    expect(mock.commitEstimated).toHaveBeenCalledTimes(1);
    const commit = mock.commitEstimated.mock.calls[0]?.[0];
    expect(commit?.outcome).toBe("PROVIDER_ERROR");
    expect(commit?.estimatedAmountAtomic).toBe("0");
  });

  it("E-10: commitEstimated failure on PROVIDER_ERROR path does NOT mask the original provider error", async () => {
    const { stepAi } = makeMockStepAi({ throwOnAttempts: [0] });
    const mock = makeMockClient();
    mock.commitEstimated.mockRejectedValueOnce(new Error("commit-storm"));
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await expect(
      sg.infer(
        "call",
        { model: {}, body: {} },
        makeRuntimeCtx({ runId: RUN_ID, step: { id: STEP_ID } }),
      ),
    ).rejects.toThrow(/provider-error-attempt-0/);
    expect(warnSpy).toHaveBeenCalled();
  });
});

// ───────────────────────────────────────────────────────────────────────────
// I-01..I-07 — identity-derivation invariants
// ───────────────────────────────────────────────────────────────────────────

describe("deriveIdentity / deriveStepIdempotencyKey", () => {
  it("I-01: deterministic for same input", () => {
    const a = deriveIdentity({
      tenantId: TENANT_ID,
      input: {
        stepId: STEP_ID,
        runId: RUN_ID,
        attempt: 0,
        model: {},
        body: {},
      },
    });
    const b = deriveIdentity({
      tenantId: TENANT_ID,
      input: {
        stepId: STEP_ID,
        runId: RUN_ID,
        attempt: 0,
        model: {},
        body: {},
      },
    });
    expect(a).toEqual(b);
  });

  it("I-02: same stepId + different attempt → same idempotencyKey", () => {
    const a = deriveIdentity({
      tenantId: TENANT_ID,
      input: { stepId: STEP_ID, runId: RUN_ID, attempt: 0, model: {}, body: {} },
    });
    const b = deriveIdentity({
      tenantId: TENANT_ID,
      input: { stepId: STEP_ID, runId: RUN_ID, attempt: 9, model: {}, body: {} },
    });
    expect(a.idempotencyKey).toBe(b.idempotencyKey);
  });

  it("I-03: different inngestIdempotencyKey → different decisionId (seed precedence)", () => {
    const a = deriveIdentity({
      tenantId: TENANT_ID,
      input: {
        stepId: STEP_ID,
        runId: RUN_ID,
        attempt: 0,
        model: {},
        body: {},
        inngestIdempotencyKey: "A",
      },
    });
    const b = deriveIdentity({
      tenantId: TENANT_ID,
      input: {
        stepId: STEP_ID,
        runId: RUN_ID,
        attempt: 0,
        model: {},
        body: {},
        inngestIdempotencyKey: "B",
      },
    });
    expect(a.decisionId).not.toBe(b.decisionId);
  });

  it("I-04: missing inngestIdempotencyKey falls back to stepId as seed", () => {
    const fallback = deriveIdentity({
      tenantId: TENANT_ID,
      input: { stepId: STEP_ID, runId: RUN_ID, attempt: 0, model: {}, body: {} },
    });
    expect(fallback.decisionId).toBe(sdkDeriveUuidFromSignature(STEP_ID, { scope: "decision_id" }));
  });

  it("I-05: different runId → different idempotencyKey", () => {
    const a = deriveIdentity({
      tenantId: TENANT_ID,
      input: { stepId: STEP_ID, runId: "run-A", attempt: 0, model: {}, body: {} },
    });
    const b = deriveIdentity({
      tenantId: TENANT_ID,
      input: { stepId: STEP_ID, runId: "run-B", attempt: 0, model: {}, body: {} },
    });
    expect(a.idempotencyKey).not.toBe(b.idempotencyKey);
  });

  it("I-06: decisionId is a valid UUIDv4-shape string", () => {
    const id = deriveIdentity({
      tenantId: TENANT_ID,
      input: { stepId: STEP_ID, runId: RUN_ID, attempt: 0, model: {}, body: {} },
    });
    expect(id.decisionId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
    );
  });

  it("I-07: idempotencyKey matches sg-[0-9a-f]{32}", () => {
    const id = deriveIdentity({
      tenantId: TENANT_ID,
      input: { stepId: STEP_ID, runId: RUN_ID, attempt: 0, model: {}, body: {} },
    });
    expect(id.idempotencyKey).toMatch(/^sg-[0-9a-f]{32}$/);
  });

  it("deriveStepIdempotencyKey is parity with deriveIdentity.idempotencyKey", () => {
    const k = deriveStepIdempotencyKey({
      tenantId: TENANT_ID,
      runId: RUN_ID,
      stepId: STEP_ID,
    });
    const id = deriveIdentity({
      tenantId: TENANT_ID,
      input: { stepId: STEP_ID, runId: RUN_ID, attempt: 0, model: {}, body: {} },
    });
    expect(k).toBe(id.idempotencyKey);
  });
});

// ───────────────────────────────────────────────────────────────────────────
// X-01..X-08 — extract probe order
// ───────────────────────────────────────────────────────────────────────────

describe("extractTotalTokens / extractProviderEventId", () => {
  it("X-01: OpenAI shape result.usage.total_tokens", async () => {
    const { extractTotalTokens } = await import("../src/extract.js");
    expect(extractTotalTokens({ usage: { total_tokens: 42 } })).toBe(42);
  });

  it("X-02: Anthropic shape result.usage_metadata.total_tokens", async () => {
    const { extractTotalTokens } = await import("../src/extract.js");
    expect(extractTotalTokens({ usage_metadata: { total_tokens: 99 } })).toBe(99);
  });

  it("X-03: legacy result.response_metadata.token_usage.total_tokens", async () => {
    const { extractTotalTokens } = await import("../src/extract.js");
    expect(
      extractTotalTokens({
        response_metadata: { token_usage: { total_tokens: 7 } },
      }),
    ).toBe(7);
  });

  it("X-04: returns 0 when none present", async () => {
    const { extractTotalTokens } = await import("../src/extract.js");
    expect(extractTotalTokens({})).toBe(0);
    expect(extractTotalTokens(undefined)).toBe(0);
    expect(extractTotalTokens(null)).toBe(0);
  });

  it("X-05: providerEventId reads result.id first", async () => {
    const { extractProviderEventId } = await import("../src/extract.js");
    expect(extractProviderEventId({ id: "evt-1" })).toBe("evt-1");
  });

  it("X-06: providerEventId falls back to response_metadata.id", async () => {
    const { extractProviderEventId } = await import("../src/extract.js");
    expect(extractProviderEventId({ response_metadata: { id: "evt-r" } })).toBe("evt-r");
  });

  it("X-07: providerEventId returns '' if absent", async () => {
    const { extractProviderEventId } = await import("../src/extract.js");
    expect(extractProviderEventId({})).toBe("");
    expect(extractProviderEventId(null)).toBe("");
  });

  it("X-08: robust to non-object usage field", async () => {
    const { extractTotalTokens } = await import("../src/extract.js");
    expect(extractTotalTokens({ usage: "garbage" })).toBe(0);
    expect(extractTotalTokens({ usage: 42 })).toBe(0);
  });
});

// ───────────────────────────────────────────────────────────────────────────
// Validation gates — defensive constructor checks.
// ───────────────────────────────────────────────────────────────────────────

describe("wrapWithSpendGuard — option validation", () => {
  it("rejects missing tenantId", () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    expect(() =>
      wrapWithSpendGuard(stepAi, mock.client, {
        // @ts-expect-error - intentional missing tenantId
        tenantId: undefined,
      }),
    ).toThrow(TypeError);
  });

  it("rejects empty tenantId", () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    expect(() => wrapWithSpendGuard(stepAi, mock.client, { tenantId: "" })).toThrow(TypeError);
  });
});
