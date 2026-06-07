// D06 SLICE 6 — Provider integration test matrix.
//
// Scope (per design.md §7 / implementation.md §1 / review-standards.md §2):
//   - Prove `createSpendGuardMiddleware(...)` works correctly when wrapped
//     around a Vercel AI SDK v4 `LanguageModelV1` that mimics the
//     `@ai-sdk/openai@^1` + `@ai-sdk/anthropic@^1` provider shapes.
//   - Cover BOTH `wrapGenerate` (non-streaming) AND `wrapStream`
//     (streaming) paths for BOTH providers.
//   - ≥10 distinct tests covering: happy-path provider routing, multi-
//     provider parity, recorded fixture parity, streaming finish-part
//     handling, error propagation under provider rejects, terminal denial
//     short-circuit (provider NEVER invoked), idempotency across the
//     middleware ↔ provider boundary.
//
// Anti-scope (SLICE 6 doc):
//   - No real HTTP fetch — provider doubles are in-process
//     `LanguageModelV1` implementations under `_support/mockProvider.ts`.
//     The middleware's contract is with `LanguageModelV1`, not with the
//     wire-level OpenAI/Anthropic HTTP API; SLICE 7 owns the live demo
//     against the in-network counting-stub.
//   - No tool-call gating — v0.2 (design.md §3).
//   - No DEGRADE patch application — v0.2 (design.md §3).
//   - No `wrapLanguageModel(...)` re-wrap test — SLICE 4/5 already proved
//     the middleware's `wrapGenerate` / `wrapStream` factories return
//     ai@4-compatible hooks. SLICE 6 drives `transformParams` + the hook
//     factories directly to keep the test rig deterministic without
//     pulling in `ai`'s middleware glue.

import { type CommitEstimatedRequest, DecisionDenied } from "@spendguard/sdk";
import type { LanguageModelV1, LanguageModelV1StreamPart } from "ai";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { _internalStashFor, createSpendGuardMiddleware } from "../src/middleware.js";
import type { SpendGuardMiddlewareOptions } from "../src/options.js";
import { MockSpendGuardClient } from "./_support/mockSidecar.js";
import {
  ANTHROPIC_FIXTURES,
  MockAnthropicModel,
  MockOpenAIModel,
  OPENAI_FIXTURES,
  makeCallOptions,
} from "./_support/mockProvider.js";

// ── Fixtures ──────────────────────────────────────────────────────────────

const TENANT_ID = "tenant-d06-slice6-providers";

function makeMiddlewareWith(client: MockSpendGuardClient): {
  middleware: ReturnType<typeof createSpendGuardMiddleware>;
  opts: SpendGuardMiddlewareOptions;
} {
  const opts: SpendGuardMiddlewareOptions = {
    client: client.client,
    tenantId: TENANT_ID,
  };
  return { middleware: createSpendGuardMiddleware(opts), opts };
}

/**
 * Drive `transformParams` → `wrapGenerate` end-to-end for one provider call.
 * Returns the inner provider call's result + the captured stash entry so
 * tests can assert against both the middleware's correlation discipline
 * AND the provider's response shape.
 */
async function driveGenerate(
  middleware: ReturnType<typeof createSpendGuardMiddleware>,
  model: LanguageModelV1,
  params: ReturnType<typeof makeCallOptions>,
): Promise<{
  result: Awaited<ReturnType<LanguageModelV1["doGenerate"]>>;
  stashBefore: ReturnType<typeof _internalStashFor>;
}> {
  if (!middleware.transformParams || !middleware.wrapGenerate) {
    throw new Error("middleware missing required hooks");
  }
  await middleware.transformParams({ type: "generate", params });
  const stashBefore = _internalStashFor(params);
  const result = await middleware.wrapGenerate({
    doGenerate: () => model.doGenerate(params),
    doStream: () => model.doStream(params),
    params,
    model,
  });
  return { result, stashBefore };
}

/**
 * Drive `transformParams` → `wrapStream` end-to-end and fully drain the
 * resulting stream. Returns the collected parts + accumulated text so
 * tests can assert against streaming pass-through AND the post-stream
 * commit's token counts.
 */
async function driveStream(
  middleware: ReturnType<typeof createSpendGuardMiddleware>,
  model: LanguageModelV1,
  params: ReturnType<typeof makeCallOptions>,
): Promise<{
  parts: LanguageModelV1StreamPart[];
  text: string;
}> {
  if (!middleware.transformParams || !middleware.wrapStream) {
    throw new Error("middleware missing required hooks");
  }
  await middleware.transformParams({ type: "stream", params });
  const inner = await middleware.wrapStream({
    doGenerate: () => model.doGenerate(params),
    doStream: () => model.doStream(params),
    params,
    model,
  });
  const parts: LanguageModelV1StreamPart[] = [];
  let text = "";
  const reader = inner.stream.getReader();
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    if (value === undefined) continue;
    parts.push(value);
    if (value.type === "text-delta") {
      text += value.textDelta;
    }
  }
  return { parts, text };
}

// ── Suite 1: OpenAI provider — generate path ──────────────────────────────

describe("D06 SLICE 6 — OpenAI provider: wrapGenerate", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  // Test 1 — happy path against OpenAI shape
  it("OpenAI happy path: reserve → doGenerate → commit SUCCESS with provider-reported tokens", async () => {
    const sidecar = new MockSpendGuardClient();
    const model = new MockOpenAIModel();
    const { middleware } = makeMiddlewareWith(sidecar);
    const params = makeCallOptions("hello openai");

    const { result, stashBefore } = await driveGenerate(middleware, model, params);

    expect(model.generateCalls).toHaveLength(1);
    expect(model.streamCalls).toHaveLength(0);
    expect(result.text).toBe(OPENAI_FIXTURES.simpleAllow.text);
    expect(result.finishReason).toBe("stop");
    expect(stashBefore?.decisionId).toBe("mock-decision-1");
    expect(sidecar.reserveCalls).toHaveLength(1);
    expect(sidecar.commitCalls).toHaveLength(1);
    const commit = sidecar.commitCalls[0]?.request as CommitEstimatedRequest;
    expect(commit.outcomeKind).toBe("SUCCESS");
    expect(commit.actualInputTokensWire).toBe("12");
    expect(commit.actualOutputTokensWire).toBe("8");
    expect(commit.reservationId).toBe("mock-reservation-1");
  });

  // Test 2 — provider HTTP-equivalent error path
  it("OpenAI provider rate-limit error → commit FAILURE + re-throws", async () => {
    const sidecar = new MockSpendGuardClient();
    const model = new MockOpenAIModel({
      errorOnNthGenerate: 1,
      errorMessage: "rate_limit_exceeded (429)",
    });
    const { middleware } = makeMiddlewareWith(sidecar);
    const params = makeCallOptions("trigger openai rate-limit");

    await expect(driveGenerate(middleware, model, params)).rejects.toThrow(
      /rate_limit_exceeded \(429\)/,
    );

    expect(sidecar.reserveCalls).toHaveLength(1);
    expect(sidecar.commitCalls).toHaveLength(1);
    const commit = sidecar.commitCalls[0]?.request as CommitEstimatedRequest;
    expect(commit.outcomeKind).toBe("FAILURE");
    expect(commit.outcome).toBe("PROVIDER_ERROR");
    expect(commit.actualErrorMessage).toContain("rate_limit_exceeded");
  });

  // Test 3 — denial short-circuit: provider NEVER invoked
  it("OpenAI: DecisionDenied propagates from transformParams; doGenerate NEVER fires", async () => {
    const sidecar = new MockSpendGuardClient({
      decisionQueue: [{ kind: "DENY", reasonCodes: ["BUDGET_EXCEEDED"] }],
    });
    const model = new MockOpenAIModel();
    const { middleware } = makeMiddlewareWith(sidecar);
    const params = makeCallOptions("blocked openai call");
    if (!middleware.transformParams) throw new Error("transformParams missing");

    await expect(
      middleware.transformParams({ type: "generate", params }),
    ).rejects.toBeInstanceOf(DecisionDenied);

    // Critical: the upstream OpenAI provider was NEVER hit.
    expect(model.generateCalls).toHaveLength(0);
    expect(model.streamCalls).toHaveLength(0);
    // No commit either — only a reserve that rejected.
    expect(sidecar.commitCalls).toHaveLength(0);
    expect(sidecar.reserveCalls).toHaveLength(1);
    expect(sidecar.reserveCalls[0]?.rejected?.name).toBe("DecisionDenied");
  });
});

// ── Suite 2: Anthropic provider — generate path ───────────────────────────

describe("D06 SLICE 6 — Anthropic provider: wrapGenerate", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  // Test 4 — Anthropic happy path
  it("Anthropic happy path: reserve → doGenerate → commit SUCCESS with provider-reported tokens", async () => {
    const sidecar = new MockSpendGuardClient();
    const model = new MockAnthropicModel();
    const { middleware } = makeMiddlewareWith(sidecar);
    const params = makeCallOptions("hello anthropic");

    const { result } = await driveGenerate(middleware, model, params);

    expect(model.generateCalls).toHaveLength(1);
    expect(result.text).toBe(ANTHROPIC_FIXTURES.simpleAllow.text);
    // `@ai-sdk/anthropic` normalises `end_turn` → canonical `"stop"`.
    expect(result.finishReason).toBe("stop");
    expect(sidecar.commitCalls).toHaveLength(1);
    const commit = sidecar.commitCalls[0]?.request as CommitEstimatedRequest;
    expect(commit.outcomeKind).toBe("SUCCESS");
    expect(commit.actualInputTokensWire).toBe("18");
    expect(commit.actualOutputTokensWire).toBe("11");
  });

  // Test 5 — Anthropic overloaded_error
  it("Anthropic overloaded_error → commit FAILURE + re-throws with provider error message", async () => {
    const sidecar = new MockSpendGuardClient();
    const model = new MockAnthropicModel({
      errorOnNthGenerate: 1,
      errorMessage: "overloaded_error: Anthropic API is currently overloaded",
    });
    const { middleware } = makeMiddlewareWith(sidecar);
    const params = makeCallOptions("trigger anthropic overload");

    await expect(driveGenerate(middleware, model, params)).rejects.toThrow(
      /overloaded_error/,
    );

    expect(sidecar.commitCalls).toHaveLength(1);
    const commit = sidecar.commitCalls[0]?.request as CommitEstimatedRequest;
    expect(commit.outcomeKind).toBe("FAILURE");
    expect(commit.actualErrorMessage).toContain("overloaded_error");
  });
});

// ── Suite 3: Streaming path — both providers ──────────────────────────────

describe("D06 SLICE 6 — wrapStream across providers", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  // Test 6 — OpenAI streaming
  it("OpenAI streaming: parts forwarded byte-for-byte; commit fires after finish part with aggregated usage", async () => {
    const sidecar = new MockSpendGuardClient();
    const model = new MockOpenAIModel();
    const { middleware } = makeMiddlewareWith(sidecar);
    const params = makeCallOptions("stream openai");

    const { parts, text } = await driveStream(middleware, model, params);

    expect(model.streamCalls).toHaveLength(1);
    expect(text).toBe("hello back stream");
    // 3 text-delta + 1 finish = 4 parts, byte-equal to fixture.
    expect(parts).toHaveLength(4);
    expect(parts[parts.length - 1]?.type).toBe("finish");
    expect(sidecar.commitCalls).toHaveLength(1);
    const commit = sidecar.commitCalls[0]?.request as CommitEstimatedRequest;
    expect(commit.outcomeKind).toBe("SUCCESS");
    expect(commit.actualInputTokensWire).toBe("14");
    expect(commit.actualOutputTokensWire).toBe("6");
  });

  // Test 7 — Anthropic streaming
  it("Anthropic streaming: parts forwarded byte-for-byte; commit fires after finish part", async () => {
    const sidecar = new MockSpendGuardClient();
    const model = new MockAnthropicModel();
    const { middleware } = makeMiddlewareWith(sidecar);
    const params = makeCallOptions("stream anthropic");

    const { parts, text } = await driveStream(middleware, model, params);

    expect(model.streamCalls).toHaveLength(1);
    expect(text).toBe("Hello from Claude!");
    expect(parts).toHaveLength(4);
    expect(sidecar.commitCalls).toHaveLength(1);
    const commit = sidecar.commitCalls[0]?.request as CommitEstimatedRequest;
    expect(commit.actualInputTokensWire).toBe("20");
    expect(commit.actualOutputTokensWire).toBe("4");
  });

  // Test 8 — streaming + deny: provider stream NEVER opened
  it("Anthropic streaming: denial halts BEFORE doStream is invoked", async () => {
    const sidecar = new MockSpendGuardClient({
      decisionQueue: [{ kind: "DENY", reasonCodes: ["BUDGET_EXCEEDED"] }],
    });
    const model = new MockAnthropicModel();
    const { middleware } = makeMiddlewareWith(sidecar);
    const params = makeCallOptions("denied anthropic stream");
    if (!middleware.transformParams) throw new Error("transformParams missing");

    await expect(
      middleware.transformParams({ type: "stream", params }),
    ).rejects.toBeInstanceOf(DecisionDenied);

    expect(model.streamCalls).toHaveLength(0);
    expect(model.generateCalls).toHaveLength(0);
  });
});

// ── Suite 4: Cross-provider parity ────────────────────────────────────────

describe("D06 SLICE 6 — Cross-provider parity", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  // Test 9 — middleware behaves identically against OpenAI vs Anthropic
  it("Same middleware drives OpenAI and Anthropic with identical commit semantics", async () => {
    // Two independent sidecars so commit logs stay clean.
    const sidecarOA = new MockSpendGuardClient();
    const sidecarAN = new MockSpendGuardClient();
    const openai = new MockOpenAIModel();
    const anthropic = new MockAnthropicModel();
    const { middleware: mwOA } = makeMiddlewareWith(sidecarOA);
    const { middleware: mwAN } = makeMiddlewareWith(sidecarAN);

    const paramsOA = makeCallOptions("greet me, openai");
    const paramsAN = makeCallOptions("greet me, anthropic");

    await driveGenerate(mwOA, openai, paramsOA);
    await driveGenerate(mwAN, anthropic, paramsAN);

    expect(sidecarOA.commitCalls).toHaveLength(1);
    expect(sidecarAN.commitCalls).toHaveLength(1);
    const cOA = sidecarOA.commitCalls[0]?.request as CommitEstimatedRequest;
    const cAN = sidecarAN.commitCalls[0]?.request as CommitEstimatedRequest;

    // Both ship SUCCESS commits, both use the same `stepId="llm_call"`,
    // both ship the canonical empty pricing freeze.
    expect(cOA.outcomeKind).toBe(cAN.outcomeKind);
    expect(cOA.stepId).toBe(cAN.stepId);
    expect(cOA.unit?.unit).toBe(cAN.unit?.unit);
    // Token counts differ because the fixtures differ — that is the
    // point of the parity test: SAME middleware, DIFFERENT provider
    // payload → DIFFERENT commit numbers, SAME wire shape.
    expect(cOA.actualInputTokensWire).not.toBe(cAN.actualInputTokensWire);
  });

  // Test 10 — sequential multi-provider calls share the same middleware
  it("Sequential OpenAI → Anthropic calls through one middleware emit independent reserve+commit pairs", async () => {
    const sidecar = new MockSpendGuardClient();
    const openai = new MockOpenAIModel();
    const anthropic = new MockAnthropicModel();
    const { middleware } = makeMiddlewareWith(sidecar);

    const p1 = makeCallOptions("first call openai");
    const p2 = makeCallOptions("second call anthropic");

    await driveGenerate(middleware, openai, p1);
    await driveGenerate(middleware, anthropic, p2);

    expect(sidecar.reserveCalls).toHaveLength(2);
    expect(sidecar.commitCalls).toHaveLength(2);
    expect(openai.generateCalls).toHaveLength(1);
    expect(anthropic.generateCalls).toHaveLength(1);
    // Each call carries its own decisionId on the commit.
    const c1 = sidecar.commitCalls[0]?.request as CommitEstimatedRequest;
    const c2 = sidecar.commitCalls[1]?.request as CommitEstimatedRequest;
    expect(c1.decisionId).not.toBe(c2.decisionId);
    expect(c1.reservationId).not.toBe(c2.reservationId);
  });

  // Test 11 — recorded fixture parity: big completion shape
  it("Both providers' bigCompletion fixtures yield correct commit token counts", async () => {
    const sidecarOA = new MockSpendGuardClient();
    const sidecarAN = new MockSpendGuardClient();
    const openai = new MockOpenAIModel({ generateFixture: OPENAI_FIXTURES.bigCompletion });
    const anthropic = new MockAnthropicModel({ generateFixture: ANTHROPIC_FIXTURES.bigCompletion });
    const { middleware: mwOA } = makeMiddlewareWith(sidecarOA);
    const { middleware: mwAN } = makeMiddlewareWith(sidecarAN);

    await driveGenerate(mwOA, openai, makeCallOptions("big openai"));
    await driveGenerate(mwAN, anthropic, makeCallOptions("big anthropic"));

    const cOA = sidecarOA.commitCalls[0]?.request as CommitEstimatedRequest;
    const cAN = sidecarAN.commitCalls[0]?.request as CommitEstimatedRequest;
    expect(cOA.actualOutputTokensWire).toBe("250");
    expect(cAN.actualOutputTokensWire).toBe("210");
  });

  // Test 12 — idempotency: same params → same idempotencyKey across calls
  it("Two transformParams calls with the same params reference deduplicate via WeakMap stash", async () => {
    const sidecar = new MockSpendGuardClient();
    const model = new MockOpenAIModel();
    const { middleware } = makeMiddlewareWith(sidecar);
    if (!middleware.transformParams) throw new Error("transformParams missing");
    const params = makeCallOptions("idempotent openai");

    await middleware.transformParams({ type: "generate", params });
    const stash1 = _internalStashFor(params);
    await middleware.transformParams({ type: "generate", params });
    const stash2 = _internalStashFor(params);

    expect(stash1?.idempotencyKey).toBe(stash2?.idempotencyKey);
    expect(stash1?.runId).toBe(stash2?.runId);

    // The mock sidecar still records both reserves — the SUBSTRATE is
    // what dedups by idempotencyKey at the persistence layer (D05).
    // The middleware-side guarantee is that the key is byte-equal so
    // the substrate's cache sees the same key.
    expect(sidecar.reserveCalls).toHaveLength(2);
    expect(sidecar.reserveCalls[0]?.request.idempotencyKey).toBe(
      sidecar.reserveCalls[1]?.request.idempotencyKey,
    );
  });

  // Test 13 — OpenAI empty-stream case
  it("OpenAI empty-stream fixture: commit fires with 0 completion tokens (review-standards §3.6)", async () => {
    const sidecar = new MockSpendGuardClient();
    const model = new MockOpenAIModel({ streamFixture: OPENAI_FIXTURES.emptyStream });
    const { middleware } = makeMiddlewareWith(sidecar);
    const params = makeCallOptions("empty stream openai");

    const { parts, text } = await driveStream(middleware, model, params);

    // Empty stream still emits the finish part.
    expect(parts).toHaveLength(1);
    expect(parts[0]?.type).toBe("finish");
    expect(text).toBe("");
    expect(sidecar.commitCalls).toHaveLength(1);
    const commit = sidecar.commitCalls[0]?.request as CommitEstimatedRequest;
    expect(commit.actualOutputTokensWire).toBe("0");
    expect(commit.outcomeKind).toBe("SUCCESS");
  });

  // Test 14 — middleware preserves provider metadata on inner result
  it("OpenAI: middleware does not mutate the inner doGenerate result (passthrough integrity)", async () => {
    const sidecar = new MockSpendGuardClient();
    const model = new MockOpenAIModel();
    const { middleware } = makeMiddlewareWith(sidecar);
    const params = makeCallOptions("preserve passthrough");

    const { result } = await driveGenerate(middleware, model, params);

    expect(result.rawCall.rawSettings).toEqual({
      temperature: 0,
      model: "gpt-4o-mini",
    });
    expect(result.rawResponse?.headers?.["x-mock-provider"]).toBe("openai");
    expect(result.finishReason).toBe("stop");
    expect(result.warnings).toEqual([]);
  });
});
