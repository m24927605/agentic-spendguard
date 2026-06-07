// SLICE 4 — End-to-end integration tests against an enhanced in-memory
// `@inngest/agent-kit` step-harness double that simulates the Inngest
// runtime's deterministic-retry contract: a step body that throws is
// re-invoked by the harness with the SAME `step.id`, the SAME
// `step.idempotencyKey`, and an incremented `step.attempt`.
//
// Scope (review-standards.md §4 headline contract):
//   - Run-level vs step-level dedup distinction (R1 panel concern).
//   - End-to-end retry replay: cached `DecisionOutcome` flows through to
//     attempts 1..N when an in-process `IdempotencyCache` is supplied.
//   - End-to-end retry replay: byte-identical `idempotencyKey` arrives at
//     the sidecar across all N attempts when no in-process cache is
//     supplied (layered-defence path).
//   - Concurrent steps with distinct ids do not cross-correlate.
//   - The harness exposes the step-attempt timeline so the test asserts
//     reservation/commit cardinality at the granularity Inngest itself
//     would observe in production.
//
// All tests use the LOCKED public surface — `wrapWithSpendGuard(stepAi,
// client, options)`. The mock agent-kit step harness `runStepUntil(...)`
// mirrors Inngest's "same step.id, same idempotencyKey, attempt counter
// advances" replay loop semantics; tests assert against the resulting
// reserve/commit call counts on the mock client.

import {
  ApprovalRequired,
  DecisionDenied,
  DecisionStopped,
  InMemoryIdempotencyCache,
  type ReserveRequest,
  SidecarUnavailable,
  deriveIdempotencyKey as sdkDeriveIdempotencyKey,
  deriveUuidFromSignature as sdkDeriveUuidFromSignature,
} from "@spendguard/sdk";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClaimEstimator, WrapWithSpendGuardOptions } from "../src/options.js";
import { wrapWithSpendGuard } from "../src/wrapWithSpendGuard.js";
import { makeMockStepAi, makeRuntimeCtx, runStepUntil } from "./_support/mockAgentKit.js";
import { makeMockClient, makeOutcome } from "./_support/mockClient.js";

// ── Constants ──────────────────────────────────────────────────────────────

const TENANT_ID = "tenant-d29-int";
const RUN_ID = "01951f25-0000-7000-8000-d29000000001";
const STEP_ID = "step-d29-int-llm-call";
const BUDGET_ID = "44444444-4444-4444-8444-444444444444";

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
// IT-01..IT-08 — end-to-end ALLOW lifecycle via the harness.
// ───────────────────────────────────────────────────────────────────────────

describe("SLICE 4 integration — ALLOW lifecycle via mock agent-kit step harness", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("IT-01: single-attempt ALLOW step fires 1 reserve + 1 commit + 1 provider call", async () => {
    const { stepAi, inferBody } = makeMockStepAi();
    const mock = makeMockClient(TENANT_ID);
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    const { result, attempts } = await runStepUntil(sg, {
      maxAttempts: 4,
      runId: RUN_ID,
      stepId: STEP_ID,
      idempotencyKey: "K-it01",
      callName: "call-1",
      callOpts: { model: {}, body: {} },
    });

    expect(result).toBeDefined();
    expect(attempts).toBe(1);
    expect(mock.reserve).toHaveBeenCalledTimes(1);
    expect(mock.commitEstimated).toHaveBeenCalledTimes(1);
    expect(inferBody).toHaveBeenCalledTimes(1);
    expect(mock.commitEstimated.mock.calls[0]?.[0]?.outcome).toBe("SUCCESS");
  });

  it("IT-02: 2-attempt retry (transient provider error) WITH in-process cache → 1 reserve, 2 provider calls, 2 commits", async () => {
    const cache = new InMemoryIdempotencyCache();
    const { stepAi, inferBody } = makeMockStepAi({ throwOnAttempts: [0] });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ idempotencyCache: cache }));

    const { result, attempts } = await runStepUntil(sg, {
      maxAttempts: 4,
      runId: RUN_ID,
      stepId: STEP_ID,
      idempotencyKey: "K-it02",
      callName: "call-it02",
      callOpts: { model: {}, body: { msg: "retry me" } },
    });

    expect(result).toBeDefined();
    expect(attempts).toBe(2);
    // Reserve fires exactly ONCE — the cache absorbs the second attempt's
    // dedup probe. Layered-defence pillar review-standards §4.3.
    expect(mock.reserve).toHaveBeenCalledTimes(1);
    expect(inferBody).toHaveBeenCalledTimes(2);
    // Both attempts emit commits: attempt 0 → PROVIDER_ERROR, attempt 1 →
    // SUCCESS. Same decisionId across both — cached outcome flows through.
    expect(mock.commitEstimated).toHaveBeenCalledTimes(2);
    expect(mock.commitEstimated.mock.calls[0]?.[0]?.outcome).toBe("PROVIDER_ERROR");
    expect(mock.commitEstimated.mock.calls[1]?.[0]?.outcome).toBe("SUCCESS");
    const decisionA = mock.commitEstimated.mock.calls[0]?.[0]?.decisionId;
    const decisionB = mock.commitEstimated.mock.calls[1]?.[0]?.decisionId;
    expect(decisionA).toBe(decisionB);
  });

  it("IT-03: 3-attempt retry WITHOUT in-process cache → 3 reserves, all byte-identical idempotencyKey (layered-defence)", async () => {
    const { stepAi } = makeMockStepAi({ throwOnAttempts: [0, 1] });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    const { attempts } = await runStepUntil(sg, {
      maxAttempts: 5,
      runId: RUN_ID,
      stepId: STEP_ID,
      idempotencyKey: "K-it03",
      callName: "call-it03",
      callOpts: { model: {}, body: {} },
    });

    expect(attempts).toBe(3);
    expect(mock.reserve).toHaveBeenCalledTimes(3);
    const k0 = (mock.reserve.mock.calls[0]?.[0] as ReserveRequest).idempotencyKey;
    const k1 = (mock.reserve.mock.calls[1]?.[0] as ReserveRequest).idempotencyKey;
    const k2 = (mock.reserve.mock.calls[2]?.[0] as ReserveRequest).idempotencyKey;
    expect(k0).toBe(k1);
    expect(k1).toBe(k2);
    // Decision id is attempt-invariant too — sidecar dedup keys off it.
    const d0 = (mock.reserve.mock.calls[0]?.[0] as ReserveRequest).decisionId;
    const d1 = (mock.reserve.mock.calls[1]?.[0] as ReserveRequest).decisionId;
    const d2 = (mock.reserve.mock.calls[2]?.[0] as ReserveRequest).decisionId;
    expect(d0).toBe(d1);
    expect(d1).toBe(d2);
  });

  it("IT-04: harness-style runtime context surfaces stable runId / eventId / step.id across all attempts", async () => {
    const cache = new InMemoryIdempotencyCache();
    const seenRunIds = new Set<string>();
    const seenStepIds = new Set<string>();
    const seenEventIds = new Set<string>();
    const estimator = vi.fn<ClaimEstimator>((input) => {
      seenRunIds.add(input.runId);
      seenStepIds.add(input.stepId);
      if (input.eventId !== undefined) seenEventIds.add(input.eventId);
      return [
        {
          scopeId: BUDGET_ID,
          amountAtomic: "0",
          unit: { unit: "USD_MICROS", denomination: 1 },
        },
      ];
    });

    const { stepAi } = makeMockStepAi({ throwOnAttempts: [0, 1] });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(
      stepAi,
      mock.client,
      makeOptions({ idempotencyCache: cache, claimEstimator: estimator }),
    );

    const { attempts } = await runStepUntil(sg, {
      maxAttempts: 5,
      runId: RUN_ID,
      stepId: STEP_ID,
      eventId: "evt-it04",
      idempotencyKey: "K-it04",
      callName: "call-it04",
      callOpts: { model: {}, body: {} },
    });

    expect(attempts).toBe(3);
    expect(seenRunIds.size).toBe(1);
    expect(seenStepIds.size).toBe(1);
    expect(seenEventIds.size).toBe(1);
    expect([...seenRunIds][0]).toBe(RUN_ID);
    expect([...seenStepIds][0]).toBe(STEP_ID);
    expect([...seenEventIds][0]).toBe("evt-it04");
  });

  it("IT-05: claimEstimator input.attempt advances 0 → 1 → 2 across replays WITHOUT the in-process cache", async () => {
    const attemptsSeen: number[] = [];
    const estimator = vi.fn<ClaimEstimator>((input) => {
      attemptsSeen.push(input.attempt);
      return [];
    });
    const { stepAi } = makeMockStepAi({ throwOnAttempts: [0, 1] });
    const mock = makeMockClient();
    // No `idempotencyCache` — the wrap calls `reserve` (and therefore the
    // estimator) on every attempt; the substrate's own idempotency dedup
    // absorbs the duplicate keys. This is the layered-defence pillar.
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ claimEstimator: estimator }));

    await runStepUntil(sg, {
      maxAttempts: 5,
      runId: RUN_ID,
      stepId: STEP_ID,
      idempotencyKey: "K-it05",
      callName: "call-it05",
      callOpts: { model: {}, body: {} },
    });

    // attempts are 0, 1, 2 (3 invocations).
    expect(attemptsSeen).toEqual([0, 1, 2]);
  });

  it("IT-06: explicit `inngestIdempotencyKey` is preferred over `step.id` as the seed", async () => {
    const cache = new InMemoryIdempotencyCache();
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ idempotencyCache: cache }));

    await runStepUntil(sg, {
      maxAttempts: 2,
      runId: RUN_ID,
      stepId: STEP_ID,
      idempotencyKey: "I-key-explicit",
      callName: "call-it06",
      callOpts: { model: {}, body: {} },
    });

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    // The decisionId seed precedence is documented at design.md §6 — the
    // explicit `step.idempotencyKey` wins over `step.id`.
    expect(req.decisionId).toBe(
      sdkDeriveUuidFromSignature("I-key-explicit", { scope: "decision_id" }),
    );
  });

  it("IT-07: missing `inngestIdempotencyKey` falls back to `step.id` as the seed", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await runStepUntil(sg, {
      maxAttempts: 2,
      runId: RUN_ID,
      stepId: STEP_ID,
      // no idempotencyKey
      callName: "call-it07",
      callOpts: { model: {}, body: {} },
    });

    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.decisionId).toBe(sdkDeriveUuidFromSignature(STEP_ID, { scope: "decision_id" }));
  });

  it("IT-08: usage extraction from infer result flows through to commit", async () => {
    const { stepAi } = makeMockStepAi({
      inferReturns: () => ({
        id: "chatcmpl-it08",
        usage: { total_tokens: 1234 },
      }),
    });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await runStepUntil(sg, {
      maxAttempts: 2,
      runId: RUN_ID,
      stepId: STEP_ID,
      idempotencyKey: "K-it08",
      callName: "call-it08",
      callOpts: { model: {}, body: {} },
    });

    const commit = mock.commitEstimated.mock.calls[0]?.[0];
    expect(commit?.outcome).toBe("SUCCESS");
    expect(commit?.estimatedAmountAtomic).toBe("1234");
    expect(commit?.providerEventId).toBe("chatcmpl-it08");
  });
});

// ───────────────────────────────────────────────────────────────────────────
// IT-09..IT-14 — run-level vs step-level dedup distinction (HEADLINE).
// ───────────────────────────────────────────────────────────────────────────

describe("SLICE 4 integration — run-level vs step-level dedup", () => {
  it("IT-09: SAME step.id under DIFFERENT runIds → DIFFERENT idempotencyKey (run scope)", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await runStepUntil(sg, {
      maxAttempts: 1,
      runId: "run-IT09-A",
      stepId: STEP_ID,
      idempotencyKey: "shared-step-key",
      callName: "call",
      callOpts: { model: {}, body: {} },
    });
    await runStepUntil(sg, {
      maxAttempts: 1,
      runId: "run-IT09-B",
      stepId: STEP_ID,
      idempotencyKey: "shared-step-key",
      callName: "call",
      callOpts: { model: {}, body: {} },
    });

    expect(mock.reserve).toHaveBeenCalledTimes(2);
    const kA = (mock.reserve.mock.calls[0]?.[0] as ReserveRequest).idempotencyKey;
    const kB = (mock.reserve.mock.calls[1]?.[0] as ReserveRequest).idempotencyKey;
    expect(kA).not.toBe(kB);
  });

  it("IT-10: DIFFERENT step.ids in the SAME runId → DIFFERENT idempotencyKey (step scope)", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await runStepUntil(sg, {
      maxAttempts: 1,
      runId: RUN_ID,
      stepId: "step-IT10-A",
      idempotencyKey: "I-key-shared",
      callName: "A",
      callOpts: { model: {}, body: {} },
    });
    await runStepUntil(sg, {
      maxAttempts: 1,
      runId: RUN_ID,
      stepId: "step-IT10-B",
      idempotencyKey: "I-key-shared",
      callName: "B",
      callOpts: { model: {}, body: {} },
    });

    expect(mock.reserve).toHaveBeenCalledTimes(2);
    const kA = (mock.reserve.mock.calls[0]?.[0] as ReserveRequest).idempotencyKey;
    const kB = (mock.reserve.mock.calls[1]?.[0] as ReserveRequest).idempotencyKey;
    expect(kA).not.toBe(kB);
  });

  it("IT-11: SAME (runId, step.id) across retry replays → step-level dedup engages (single reserve with in-process cache)", async () => {
    const cache = new InMemoryIdempotencyCache();
    const { stepAi } = makeMockStepAi({ throwOnAttempts: [0, 1] });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ idempotencyCache: cache }));

    await runStepUntil(sg, {
      maxAttempts: 5,
      runId: RUN_ID,
      stepId: STEP_ID,
      idempotencyKey: "K-it11",
      callName: "call",
      callOpts: { model: {}, body: {} },
    });

    expect(mock.reserve).toHaveBeenCalledTimes(1);
  });

  it("IT-12: SAME (runId, step.id) across runs in different Inngest function invocations DO NOT collapse (run-scope sealing)", async () => {
    const cacheA = new InMemoryIdempotencyCache();
    const cacheB = new InMemoryIdempotencyCache();
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sgA = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ idempotencyCache: cacheA }));
    const sgB = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ idempotencyCache: cacheB }));

    await runStepUntil(sgA, {
      maxAttempts: 1,
      runId: "run-A",
      stepId: STEP_ID,
      idempotencyKey: "K-shared",
      callName: "call",
      callOpts: { model: {}, body: {} },
    });
    await runStepUntil(sgB, {
      maxAttempts: 1,
      runId: "run-B",
      stepId: STEP_ID,
      idempotencyKey: "K-shared",
      callName: "call",
      callOpts: { model: {}, body: {} },
    });

    // Reserve fires twice — different runIds → different keys → different
    // sidecar decision rows. Fresh run NOT deduped against prior run.
    expect(mock.reserve).toHaveBeenCalledTimes(2);
    const kA = (mock.reserve.mock.calls[0]?.[0] as ReserveRequest).idempotencyKey;
    const kB = (mock.reserve.mock.calls[1]?.[0] as ReserveRequest).idempotencyKey;
    expect(kA).not.toBe(kB);
  });

  it("IT-13: two distinct steps in the same retry sweep do NOT share state", async () => {
    const cache = new InMemoryIdempotencyCache();
    const { stepAi } = makeMockStepAi({ throwOnAttempts: [0] });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ idempotencyCache: cache }));

    await runStepUntil(sg, {
      maxAttempts: 5,
      runId: RUN_ID,
      stepId: "step-IT13-A",
      idempotencyKey: "K-IT13-A",
      callName: "A",
      callOpts: { model: {}, body: {} },
    });
    await runStepUntil(sg, {
      maxAttempts: 5,
      runId: RUN_ID,
      stepId: "step-IT13-B",
      idempotencyKey: "K-IT13-B",
      callName: "B",
      callOpts: { model: {}, body: {} },
    });

    // Each step contributes 1 reserve (the cache absorbs the second
    // attempt of each). Total 2 reserves across the two distinct steps.
    expect(mock.reserve).toHaveBeenCalledTimes(2);
    const stepIds = mock.reserve.mock.calls.map((c) => (c[0] as ReserveRequest).stepId);
    expect(new Set(stepIds)).toEqual(new Set(["step-IT13-A", "step-IT13-B"]));
  });

  it("IT-14: idempotencyKey derived in the wrap matches sdkDeriveIdempotencyKey for the canonical tuple", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await runStepUntil(sg, {
      maxAttempts: 1,
      runId: RUN_ID,
      stepId: STEP_ID,
      idempotencyKey: "K-it14",
      callName: "call",
      callOpts: { model: {}, body: {} },
    });

    const expected = sdkDeriveIdempotencyKey({
      tenantId: TENANT_ID,
      sessionId: RUN_ID,
      runId: RUN_ID,
      stepId: STEP_ID,
      llmCallId: STEP_ID,
      trigger: "LLM_CALL_PRE",
    });
    expect((mock.reserve.mock.calls[0]?.[0] as ReserveRequest).idempotencyKey).toBe(expected);
  });
});

// ───────────────────────────────────────────────────────────────────────────
// IT-15..IT-19 — failure-path replay matrices.
// ───────────────────────────────────────────────────────────────────────────

describe("SLICE 4 integration — failure-path replay matrices", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("IT-15: DecisionDenied at attempt 0 halts replay; inner body NEVER fires", async () => {
    const { stepAi, inferBody } = makeMockStepAi();
    const mock = makeMockClient();
    mock.reserve.mockRejectedValueOnce(
      new DecisionDenied("budget exhausted", {
        decisionId: "d-deny-it15",
        reasonCodes: ["BUDGET_EXCEEDED"],
      }),
    );
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    const outcome = await runStepUntil(sg, {
      maxAttempts: 3,
      runId: RUN_ID,
      stepId: STEP_ID,
      idempotencyKey: "K-it15",
      callName: "call",
      callOpts: { model: {}, body: {} },
    }).catch((err) => ({ thrown: err, attempts: 1 }));

    expect((outcome as { thrown?: unknown }).thrown).toBeInstanceOf(DecisionDenied);
    expect(inferBody).not.toHaveBeenCalled();
    expect(mock.commitEstimated).not.toHaveBeenCalled();
  });

  it("IT-16: DecisionStopped propagates and the harness does NOT retry (Inngest's NonRetriable semantics)", async () => {
    const { stepAi, inferBody } = makeMockStepAi({ throwOnAttempts: [0] });
    const mock = makeMockClient();
    mock.reserve.mockRejectedValueOnce(
      new DecisionStopped("hard stop", {
        decisionId: "d-stop-it16",
        reasonCodes: ["STOP_POLICY"],
      }),
    );
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await expect(
      runStepUntil(sg, {
        maxAttempts: 4,
        runId: RUN_ID,
        stepId: STEP_ID,
        idempotencyKey: "K-it16",
        callName: "call",
        callOpts: { model: {}, body: {} },
      }),
    ).rejects.toBeInstanceOf(DecisionStopped);
    expect(inferBody).not.toHaveBeenCalled();
  });

  it("IT-17: ApprovalRequired with onApprovalRequired resumer: replay completes via supplied outcome", async () => {
    const resumed = makeOutcome({
      decisionId: "d-resumed-it17",
      reservationIds: ["r-resumed-it17"],
    });
    const { stepAi, inferBody } = makeMockStepAi();
    const mock = makeMockClient();
    const apprErr = new ApprovalRequired("needs approval", {
      decisionId: "d-appr-it17",
      reasonCodes: ["APPROVAL_REQUIRED"],
      approvalRequestId: "appr-req-it17",
    });
    mock.reserve.mockRejectedValueOnce(apprErr);
    const onApprovalRequired = vi.fn(async () => resumed);
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ onApprovalRequired }));

    const { attempts } = await runStepUntil(sg, {
      maxAttempts: 2,
      runId: RUN_ID,
      stepId: STEP_ID,
      idempotencyKey: "K-it17",
      callName: "call",
      callOpts: { model: {}, body: {} },
    });

    expect(attempts).toBe(1);
    expect(onApprovalRequired).toHaveBeenCalledTimes(1);
    expect(inferBody).toHaveBeenCalledTimes(1);
    expect(mock.commitEstimated.mock.calls[0]?.[0]?.decisionId).toBe("d-resumed-it17");
  });

  it("IT-18: SidecarUnavailable at attempt 0 propagates; harness gets a typed error not a silent failure", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    mock.reserve.mockRejectedValueOnce(new SidecarUnavailable("UDS gone"));
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    await expect(
      runStepUntil(sg, {
        maxAttempts: 3,
        runId: RUN_ID,
        stepId: STEP_ID,
        idempotencyKey: "K-it18",
        callName: "call",
        callOpts: { model: {}, body: {} },
      }),
    ).rejects.toBeInstanceOf(SidecarUnavailable);
  });

  it("IT-19: provider-error replay records 1 PROVIDER_ERROR commit per attempt; cache absorbs the 2nd reserve", async () => {
    const cache = new InMemoryIdempotencyCache();
    const { stepAi } = makeMockStepAi({ throwOnAttempts: [0, 1, 2] });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ idempotencyCache: cache }));

    await expect(
      runStepUntil(sg, {
        maxAttempts: 3,
        runId: RUN_ID,
        stepId: STEP_ID,
        idempotencyKey: "K-it19",
        callName: "call",
        callOpts: { model: {}, body: {} },
      }),
    ).rejects.toThrow(/provider-error-attempt-2/);

    // 3 attempts, 1 reserve (cache hit for attempts 1 + 2), 3 commits.
    expect(mock.reserve).toHaveBeenCalledTimes(1);
    expect(mock.commitEstimated).toHaveBeenCalledTimes(3);
    for (const call of mock.commitEstimated.mock.calls) {
      expect(call[0]?.outcome).toBe("PROVIDER_ERROR");
    }
  });
});

// ───────────────────────────────────────────────────────────────────────────
// IT-20..IT-23 — concurrency + harness fidelity.
// ───────────────────────────────────────────────────────────────────────────

describe("SLICE 4 integration — concurrency + harness fidelity", () => {
  it("IT-20: two concurrent runs (different step.ids) do not cross-correlate", async () => {
    const { stepAi } = makeMockStepAi();
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    const [aOutcome, bOutcome] = await Promise.all([
      runStepUntil(sg, {
        maxAttempts: 1,
        runId: "run-IT20-A",
        stepId: "step-IT20-A",
        idempotencyKey: "K-A",
        callName: "A",
        callOpts: { model: {}, body: {} },
      }),
      runStepUntil(sg, {
        maxAttempts: 1,
        runId: "run-IT20-B",
        stepId: "step-IT20-B",
        idempotencyKey: "K-B",
        callName: "B",
        callOpts: { model: {}, body: {} },
      }),
    ]);

    expect(aOutcome.attempts).toBe(1);
    expect(bOutcome.attempts).toBe(1);
    expect(mock.reserve).toHaveBeenCalledTimes(2);
    const stepIds = mock.reserve.mock.calls.map((c) => (c[0] as ReserveRequest).stepId);
    expect(new Set(stepIds)).toEqual(new Set(["step-IT20-A", "step-IT20-B"]));
  });

  it("IT-21: max-attempts ceiling protects the harness from runaway retry storms", async () => {
    const cache = new InMemoryIdempotencyCache();
    const { stepAi } = makeMockStepAi({ throwOnAttempts: [0, 1, 2, 3, 4] });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions({ idempotencyCache: cache }));

    await expect(
      runStepUntil(sg, {
        maxAttempts: 3,
        runId: RUN_ID,
        stepId: STEP_ID,
        idempotencyKey: "K-it21",
        callName: "call",
        callOpts: { model: {}, body: {} },
      }),
    ).rejects.toThrow(/provider-error/);
    // The harness stops at maxAttempts even though the provider would
    // keep failing — proves the test framework matches Inngest's bounded
    // replay model.
    expect(mock.reserve).toHaveBeenCalledTimes(1);
  });

  it("IT-22: harness uses real-shape runtime ctx (`{runId, eventId, step}`)", async () => {
    const ctxBag = makeRuntimeCtx({
      runId: "run-it22",
      eventId: "evt-it22",
      step: { id: "step-it22", attempt: 3, idempotencyKey: "I-it22" },
    });
    expect(ctxBag.runId).toBe("run-it22");
    expect(ctxBag.eventId).toBe("evt-it22");
    expect(ctxBag.step.id).toBe("step-it22");
    expect(ctxBag.step.attempt).toBe(3);
    expect(ctxBag.step.idempotencyKey).toBe("I-it22");
  });

  it("IT-23: harness preserves the inner step.ai's return value through the wrap (no result mutation)", async () => {
    const { stepAi } = makeMockStepAi({
      inferReturns: () => ({
        id: "chatcmpl-it23",
        usage: { total_tokens: 5 },
        choices: [{ message: { content: "hi from it23" } }],
      }),
    });
    const mock = makeMockClient();
    const sg = wrapWithSpendGuard(stepAi, mock.client, makeOptions());

    const { result } = await runStepUntil(sg, {
      maxAttempts: 1,
      runId: RUN_ID,
      stepId: STEP_ID,
      idempotencyKey: "K-it23",
      callName: "call",
      callOpts: { model: {}, body: {} },
    });

    const typed = result as {
      id: string;
      usage: { total_tokens: number };
      choices: Array<{ message: { content: string } }>;
    };
    expect(typed.id).toBe("chatcmpl-it23");
    expect(typed.choices[0]?.message.content).toBe("hi from it23");
  });
});
