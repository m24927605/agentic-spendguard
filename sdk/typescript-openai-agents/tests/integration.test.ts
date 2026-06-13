// SLICE 3 — End-to-end integration tests.
//
// Drives `withSpendGuard(...)` through a mock `Agent`-style harness against
// an in-process `MockSpendGuardClient` and an upstream `MockUpstreamModel`.
// Exercises the full ALLOW / DENY / STOP / APPROVAL_REQUIRED /
// SIDECAR_UNAVAILABLE / PROVIDER_ERROR matrix end-to-end + cross-language
// determinism gates against fixture vectors in
// `sdk/fixtures/cross-language/v1.json#openai_agents`.
//
// Scope (review-standards.md):
//   - §1 (Behaviour invariant — P0): every non-CONTINUE outcome asserts
//     `mock.upstream.callCount === 0` (reviewer gate 1.3).
//   - §2 (Cross-language determinism — P0): every fixture vector under
//     the `openai_agents` section asserts byte-equality on
//     deriveAgentSignature / derive UUID / derive idempotency key.
//   - §3 (Public-surface lock — P0): factory + class call shape matches
//     design.md §4 verbatim.
//   - §9 (Default estimator parity — P1): MODEL_BASELINE_TOKENS literal
//     matches design.md §11 table byte-for-byte; unknown model → 800.
//   - §10 (Error semantics — P2): substrate-typed errors propagate
//     unchanged; commit-side failure does not corrupt inner response.

import type { ModelResponse } from "@openai/agents";
import {
  ApprovalRequired,
  DecisionDenied,
  DecisionStopped,
  type ReserveRequest,
  SidecarUnavailable,
  deriveIdempotencyKey,
  deriveUuidFromSignature,
} from "@spendguard/sdk";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  DEFAULT_BASELINE_TOKENS,
  MODEL_BASELINE_TOKENS,
  defaultClaimEstimator,
  resolveBaselineTokens,
} from "../src/defaultEstimator.js";
import { SpendGuardAgentsModel } from "../src/model.js";
import { runContext } from "../src/runContext.js";
import { deriveAgentSignature } from "../src/signature.js";
import { withSpendGuard } from "../src/withSpendGuard.js";
import { makeMockClient, makeOutcome } from "./_support/mockClient.js";
import { MockUpstreamModel, makeAgentRequest } from "./_support/mockOpenAIAgents.js";

// ── Test helper ────────────────────────────────────────────────────────────

/** Build a `Partial<ModelResponse>` override that supplies usage + responseId. */
function makeResponseOverride(opts: {
  inputTokens: number;
  outputTokens: number;
  totalTokens?: number;
  responseId?: string;
}): Partial<ModelResponse> {
  return {
    usage: {
      requests: 1,
      inputTokens: opts.inputTokens,
      outputTokens: opts.outputTokens,
      totalTokens: opts.totalTokens ?? opts.inputTokens + opts.outputTokens,
      inputTokensDetails: [],
      outputTokensDetails: [],
    } as unknown as ModelResponse["usage"],
    output: [],
    ...(opts.responseId !== undefined ? { responseId: opts.responseId } : {}),
  } as Partial<ModelResponse>;
}

const TENANT_ID = "tenant-d08-s3";
const BUDGET_ID = "44444444-4444-4444-8444-444444444444";
const RUN_ID = "01951f25-0000-7000-8000-d08000000001";

// ── Suite: ALLOW lifecycle through the harness ─────────────────────────────

describe("SLICE 3 integration — ALLOW lifecycle", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  // Test 1
  it("Runner-style ALLOW path: reserve → inner once → commit SUCCESS", async () => {
    const mock = makeMockClient();
    mock.reserve.mockResolvedValueOnce(
      makeOutcome({ decisionId: "dec-allow", reservationIds: ["res-allow"] }),
    );
    const upstream = new MockUpstreamModel({ model: "gpt-4o-mini" });
    const guarded = withSpendGuard(upstream, {
      client: mock.client,
      tenantId: TENANT_ID,
      budgetId: BUDGET_ID,
    });

    const response = await runContext({ runId: RUN_ID }, () =>
      guarded.getResponse(makeAgentRequest({ input: "hello agent" })),
    );

    expect(upstream.callCount).toBe(1);
    expect(mock.reserve).toHaveBeenCalledTimes(1);
    expect(mock.commitEstimated).toHaveBeenCalledTimes(1);
    expect((response as { responseId?: string }).responseId).toBe("resp-mock-default");
    const reserveReq = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(reserveReq.projectedClaims[0]?.scopeId).toBe(BUDGET_ID);
    // gpt-4o-mini baseline = 500 per design.md §11 table.
    expect(reserveReq.projectedClaims[0]?.amountAtomic).toBe("500");
  });

  // Test 2
  it("default baseline routes via inner.model (gpt-4o → 1500)", async () => {
    const mock = makeMockClient();
    const upstream = new MockUpstreamModel({ model: "gpt-4o" });
    const guarded = withSpendGuard(upstream, {
      client: mock.client,
      tenantId: TENANT_ID,
    });
    await runContext({ runId: RUN_ID }, () => guarded.getResponse(makeAgentRequest()));
    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims[0]?.amountAtomic).toBe("1500");
  });

  // Test 3
  it("unknown model name falls back to DEFAULT_BASELINE_TOKENS (800)", async () => {
    const mock = makeMockClient();
    const upstream = new MockUpstreamModel({ model: "gpt-4o-novel-2030" });
    const guarded = withSpendGuard(upstream, {
      client: mock.client,
      tenantId: TENANT_ID,
    });
    await runContext({ runId: RUN_ID }, () => guarded.getResponse(makeAgentRequest()));
    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims[0]?.amountAtomic).toBe(String(DEFAULT_BASELINE_TOKENS));
  });

  // Test 4
  it("subclass form parity — Runner-style ALLOW path", async () => {
    const mock = makeMockClient();
    mock.reserve.mockResolvedValueOnce(
      makeOutcome({ decisionId: "dec-allow-cls", reservationIds: ["res-allow-cls"] }),
    );
    const upstream = new MockUpstreamModel({ model: "o1" });
    const guarded = new SpendGuardAgentsModel({
      inner: upstream,
      client: mock.client,
      tenantId: TENANT_ID,
    });
    await runContext({ runId: RUN_ID }, () =>
      guarded.getResponse(makeAgentRequest({ input: "subclass" })),
    );
    expect(upstream.callCount).toBe(1);
    expect(mock.commitEstimated).toHaveBeenCalledTimes(1);
    // o1 baseline = 3000.
    const req = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    expect(req.projectedClaims[0]?.amountAtomic).toBe("3000");
  });

  // Test 5
  it("commit carries the inner usage.totalTokens (real token aggregation)", async () => {
    const mock = makeMockClient();
    const upstream = new MockUpstreamModel({
      model: "gpt-4o-mini",
      responses: [
        makeResponseOverride({ inputTokens: 42, outputTokens: 58, responseId: "resp-A" }),
      ],
    });
    const guarded = withSpendGuard(upstream, { client: mock.client, tenantId: TENANT_ID });
    await runContext({ runId: RUN_ID }, () =>
      guarded.getResponse(makeAgentRequest({ input: "tokens-test" })),
    );
    expect(mock.commitEstimated.mock.calls[0]?.[0]?.estimatedAmountAtomic).toBe("100");
    expect(mock.commitEstimated.mock.calls[0]?.[0]?.providerEventId).toBe("resp-A");
  });
});

// ── Suite: DENY / STOP / APPROVAL_REQUIRED — inner NEVER reached ──────────

describe("SLICE 3 integration — non-CONTINUE outcomes", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  // Test 6
  it("DENY: DecisionDenied throws, upstream stays at 0 calls", async () => {
    const mock = makeMockClient();
    mock.reserve.mockRejectedValueOnce(
      new DecisionDenied("budget exceeded", {
        decisionId: "dec-deny",
        reasonCodes: ["BUDGET_EXCEEDED"],
      }),
    );
    const upstream = new MockUpstreamModel();
    const guarded = withSpendGuard(upstream, { client: mock.client, tenantId: TENANT_ID });
    await expect(
      runContext({ runId: RUN_ID }, () => guarded.getResponse(makeAgentRequest())),
    ).rejects.toBeInstanceOf(DecisionDenied);
    expect(upstream.callCount).toBe(0);
    expect(mock.commitEstimated).not.toHaveBeenCalled();
  });

  // Test 7
  it("STOP: DecisionStopped throws, upstream stays at 0 calls", async () => {
    const mock = makeMockClient();
    mock.reserve.mockRejectedValueOnce(
      new DecisionStopped("projection over threshold", {
        decisionId: "dec-stop",
        reasonCodes: ["projection.run.over_threshold"],
      }),
    );
    const upstream = new MockUpstreamModel();
    const guarded = withSpendGuard(upstream, { client: mock.client, tenantId: TENANT_ID });
    await expect(
      runContext({ runId: RUN_ID }, () => guarded.getResponse(makeAgentRequest())),
    ).rejects.toBeInstanceOf(DecisionStopped);
    expect(upstream.callCount).toBe(0);
    expect(mock.commitEstimated).not.toHaveBeenCalled();
  });

  // Test 8
  it("APPROVAL_REQUIRED: ApprovalRequired propagates, upstream stays at 0", async () => {
    const mock = makeMockClient();
    mock.reserve.mockRejectedValueOnce(
      new ApprovalRequired("needs operator approval", {
        decisionId: "dec-approval",
        approvalRequestId: "appr-1",
      }),
    );
    const upstream = new MockUpstreamModel();
    const guarded = withSpendGuard(upstream, { client: mock.client, tenantId: TENANT_ID });
    const err = await runContext({ runId: RUN_ID }, () =>
      guarded.getResponse(makeAgentRequest()),
    ).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApprovalRequired);
    // ApprovalRequired is a DecisionDenied subclass — caller can pattern-match
    // on either.
    expect(err).toBeInstanceOf(DecisionDenied);
    expect(upstream.callCount).toBe(0);
    expect(mock.commitEstimated).not.toHaveBeenCalled();
  });

  // Test 9
  it("SidecarUnavailable propagates UNCHANGED (no degrade in v0.1.x)", async () => {
    const mock = makeMockClient();
    mock.reserve.mockRejectedValueOnce(new SidecarUnavailable("UDS gone"));
    const upstream = new MockUpstreamModel();
    const guarded = withSpendGuard(upstream, { client: mock.client, tenantId: TENANT_ID });
    await expect(
      runContext({ runId: RUN_ID }, () => guarded.getResponse(makeAgentRequest())),
    ).rejects.toBeInstanceOf(SidecarUnavailable);
    expect(upstream.callCount).toBe(0);
  });

  // Test 10
  it("subclass form parity — DENY blocks before inner.getResponse", async () => {
    const mock = makeMockClient();
    mock.reserve.mockRejectedValueOnce(
      new DecisionDenied("nope", { decisionId: "d", reasonCodes: ["X"] }),
    );
    const upstream = new MockUpstreamModel();
    const guarded = new SpendGuardAgentsModel({
      inner: upstream,
      client: mock.client,
      tenantId: TENANT_ID,
    });
    await expect(
      runContext({ runId: RUN_ID }, () => guarded.getResponse(makeAgentRequest())),
    ).rejects.toBeInstanceOf(DecisionDenied);
    expect(upstream.callCount).toBe(0);
  });
});

// ── Suite: provider error — POST FAILURE path ──────────────────────────────

describe("SLICE 3 integration — provider error path", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  // Test 11
  it("inner.getResponse throws → commit fires PROVIDER_ERROR with amount=0", async () => {
    const mock = makeMockClient();
    mock.reserve.mockResolvedValueOnce(makeOutcome({ decisionId: "d", reservationIds: ["r"] }));
    const upstream = new MockUpstreamModel();
    upstream.errorToThrow = new Error("upstream 503 OutOfCapacity");
    const guarded = withSpendGuard(upstream, { client: mock.client, tenantId: TENANT_ID });
    await expect(
      runContext({ runId: RUN_ID }, () => guarded.getResponse(makeAgentRequest())),
    ).rejects.toThrow(/upstream 503/);
    expect(upstream.callCount).toBe(1);
    expect(mock.commitEstimated).toHaveBeenCalledTimes(1);
    const commit = mock.commitEstimated.mock.calls[0]?.[0];
    expect(commit?.outcome).toBe("PROVIDER_ERROR");
    expect(commit?.estimatedAmountAtomic).toBe("0");
  });

  // Test 12
  it("commit-side failure does NOT corrupt inner response (warns instead)", async () => {
    const mock = makeMockClient();
    mock.commitEstimated.mockRejectedValueOnce(new SidecarUnavailable("commit gone"));
    const upstream = new MockUpstreamModel({
      model: "gpt-4o-mini",
      responses: [
        makeResponseOverride({ inputTokens: 3, outputTokens: 5, responseId: "resp-warn" }),
      ],
    });
    const guarded = withSpendGuard(upstream, { client: mock.client, tenantId: TENANT_ID });
    const response = await runContext({ runId: RUN_ID }, () =>
      guarded.getResponse(makeAgentRequest()),
    );
    expect((response as { responseId?: string }).responseId).toBe("resp-warn");
  });
});

// ── Suite: cross-language fixture parity ───────────────────────────────────

describe("SLICE 3 integration — cross-language fixture parity", async () => {
  // Lazy-load the fixture file at test time so a missing v1.json fails
  // with a focused error rather than a top-level import crash.
  const fs = await import("node:fs/promises");
  const path = await import("node:path");
  const url = await import("node:url");
  const here = path.dirname(url.fileURLToPath(import.meta.url));
  const fixturePath = path.resolve(here, "../../../sdk/fixtures/cross-language/v1.json");
  const rawJson = await fs.readFile(fixturePath, "utf-8");
  const fixtures = JSON.parse(rawJson) as {
    fixtures: Array<{
      id: string;
      fn: string;
      description: string;
      inputs: Record<string, unknown>;
      expected_output: string;
    }>;
  };

  const openaiAgentsFixtures = fixtures.fixtures.filter((f) => f.id.startsWith("FXOA"));

  // Test 13 — at least 4 fixture rows committed (review-standards §2.5
  // requires ≥ 32 across the whole openai_agents section once Python sibling
  // ports; for v0.1.x we ship the seed ≥ 4 vectors design.md §11 baselines).
  it("fixtures/cross-language/v1.json carries the openai_agents section (≥ 4 rows)", () => {
    expect(openaiAgentsFixtures.length).toBeGreaterThanOrEqual(4);
  });

  // Test 14 — every signatureOf fixture matches byte-for-byte.
  it("deriveAgentSignature matches each `derive_agent_signature` fixture row", () => {
    const sigRows = openaiAgentsFixtures.filter((f) => f.fn === "derive_agent_signature");
    expect(sigRows.length).toBeGreaterThanOrEqual(1);
    for (const row of sigRows) {
      const input = row.inputs.input as unknown;
      const sysInst = row.inputs.system_instructions as string | null | undefined;
      const actual = deriveAgentSignature(input, sysInst ?? null);
      expect(actual).toBe(row.expected_output);
    }
  });

  it("deriveAgentSignature mirrors Python repr quote/control edges", () => {
    expect(deriveAgentSignature("can't", null)).toBe("c60f3565dd179cae8973a0e1b500a64d");
    expect(deriveAgentSignature("line\nbreak", "sys\tinst")).toBe(
      "d8fa680b4135e57412c6b21d2189a8a3",
    );
    expect(deriveAgentSignature("mix\u2028世界", null)).toBe("c6cfaaaf3447a3baa6cc6ce5b03b95af");
  });

  // Test 15 — every deriveIdempotencyKey row using the LLM_CALL_PRE trigger
  // shape matches the OpenAI-agents-style (runId, stepId, llmCallId) tuple.
  it("deriveIdempotencyKey matches each `derive_idempotency_key` openai_agents row", () => {
    const rows = openaiAgentsFixtures.filter((f) => f.fn === "derive_idempotency_key");
    expect(rows.length).toBeGreaterThanOrEqual(1);
    for (const row of rows) {
      const actual = deriveIdempotencyKey({
        tenantId: row.inputs.tenant_id as string,
        sessionId: row.inputs.session_id as string,
        runId: row.inputs.run_id as string,
        stepId: row.inputs.step_id as string,
        llmCallId: row.inputs.llm_call_id as string,
        trigger: row.inputs.trigger as string,
      });
      expect(actual).toBe(row.expected_output);
    }
  });

  // Test 16 — UUID derivation for openai_agents (decision_id / llm_call_id scopes).
  it("deriveUuidFromSignature matches each openai_agents UUID row", () => {
    const rows = openaiAgentsFixtures.filter((f) => f.fn === "derive_uuid_from_signature");
    expect(rows.length).toBeGreaterThanOrEqual(1);
    for (const row of rows) {
      const actual = deriveUuidFromSignature(row.inputs.signature as string, {
        scope: row.inputs.scope as string,
      });
      expect(actual).toBe(row.expected_output);
    }
  });
});

// ── Suite: default estimator parity (MODEL_BASELINE_TOKENS) ────────────────

describe("SLICE 3 integration — MODEL_BASELINE_TOKENS parity", () => {
  // Test 17 — table values byte-identical to design.md §11.
  it("MODEL_BASELINE_TOKENS matches design.md §11 literal table", () => {
    expect(MODEL_BASELINE_TOKENS["gpt-4o-mini"]).toBe(500);
    expect(MODEL_BASELINE_TOKENS["gpt-4o"]).toBe(1500);
    expect(MODEL_BASELINE_TOKENS["gpt-4.1-mini"]).toBe(500);
    expect(MODEL_BASELINE_TOKENS["gpt-4.1"]).toBe(1500);
    expect(MODEL_BASELINE_TOKENS.o1).toBe(3000);
    expect(MODEL_BASELINE_TOKENS["o3-mini"]).toBe(1500);
    expect(MODEL_BASELINE_TOKENS.o3).toBe(3000);
  });

  // Test 18 — resolveBaselineTokens dispatch.
  it("resolveBaselineTokens returns table value when known, 800 fallback otherwise", () => {
    expect(resolveBaselineTokens("gpt-4o")).toBe(1500);
    expect(resolveBaselineTokens("not-a-model")).toBe(800);
  });

  // Test 19 — defaultClaimEstimator output shape.
  it("defaultClaimEstimator emits a single BudgetClaim with the right scope + unit", () => {
    const est = defaultClaimEstimator({
      scopeId: BUDGET_ID,
      unit: { unit: "USD_MICROS", denomination: 1 },
      modelName: "gpt-4o-mini",
    });
    const claims = est("hello");
    expect(claims).toHaveLength(1);
    expect(claims[0]?.scopeId).toBe(BUDGET_ID);
    expect(claims[0]?.amountAtomic).toBe("500");
    expect(claims[0]?.unit).toEqual({ unit: "USD_MICROS", denomination: 1 });
  });
});

// ── Suite: tenant-scoped budget propagation ────────────────────────────────

describe("SLICE 3 integration — tenant + budget propagation", () => {
  // Test 20 — budgetId overrides scopeId, tenantId fallback otherwise.
  it("opts.budgetId routes the projected claim scopeId; falls back to tenantId", async () => {
    const mockA = makeMockClient();
    const mockB = makeMockClient();
    const upstream = new MockUpstreamModel({ model: "gpt-4o-mini" });
    const guardedA = withSpendGuard(upstream, {
      client: mockA.client,
      tenantId: TENANT_ID,
      budgetId: BUDGET_ID,
    });
    const guardedB = withSpendGuard(upstream, {
      client: mockB.client,
      tenantId: TENANT_ID,
    });
    await runContext({ runId: RUN_ID }, async () => {
      await guardedA.getResponse(makeAgentRequest({ input: "A" }));
      await guardedB.getResponse(makeAgentRequest({ input: "B" }));
    });
    expect((mockA.reserve.mock.calls[0]?.[0] as ReserveRequest).projectedClaims[0]?.scopeId).toBe(
      BUDGET_ID,
    );
    expect((mockB.reserve.mock.calls[0]?.[0] as ReserveRequest).projectedClaims[0]?.scopeId).toBe(
      TENANT_ID,
    );
  });

  // Test 21 — idempotency key derivation is deterministic across identical
  // (runId, input, sysInst) triples. Reuses the SLICE 2 cross-call
  // determinism assertion at the integration layer.
  it("identical (runId, input) produces identical idempotencyKey + decisionId", async () => {
    const mock = makeMockClient();
    const upstream = new MockUpstreamModel({ model: "gpt-4o-mini" });
    const guarded = withSpendGuard(upstream, { client: mock.client, tenantId: TENANT_ID });
    await runContext({ runId: RUN_ID }, async () => {
      await guarded.getResponse(makeAgentRequest({ input: "same" }));
      await guarded.getResponse(makeAgentRequest({ input: "same" }));
    });
    const reqA = mock.reserve.mock.calls[0]?.[0] as ReserveRequest;
    const reqB = mock.reserve.mock.calls[1]?.[0] as ReserveRequest;
    expect(reqA.idempotencyKey).toBe(reqB.idempotencyKey);
    expect(reqA.decisionId).toBe(reqB.decisionId);
    expect(reqA.llmCallId).toBe(reqB.llmCallId);
  });
});
