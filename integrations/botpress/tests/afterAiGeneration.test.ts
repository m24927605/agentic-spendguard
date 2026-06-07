// afterAiGeneration.test.ts — unit suite covering A01–A08 per tests.md §2.3.

import { RuntimeError } from "@botpress/sdk";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { BotpressHookInput } from "../src/adapter/binding.js";
import { runAfterAiGeneration } from "../src/hooks/afterAiGeneration.js";
import {
  type SpendGuardHandleStash,
  runBeforeAiGeneration,
} from "../src/hooks/beforeAiGeneration.js";
import type { ReservationHandle } from "../src/reservation.js";
import { makeConfig, makeHookInput } from "./_fixtures.js";
import { type MockSidecarHandle, setupMockSidecar } from "./_mockSidecar.js";

type AfterData = BotpressHookInput["data"] &
  SpendGuardHandleStash & {
    payload?: { usage?: { inputTokens?: number; outputTokens?: number } };
    usage?: { inputTokens?: number; outputTokens?: number };
    response?: { usage?: Record<string, number> };
    providerEventId?: string;
    _cancelled?: boolean;
  };

async function preBefore(mock: MockSidecarHandle): Promise<{
  input: BotpressHookInput & { data: AfterData };
  configuration: ReturnType<typeof makeConfig>;
  handle: ReservationHandle;
}> {
  const configuration = makeConfig({ sidecarUrl: mock.url });
  const inputBefore = makeHookInput();
  const out = await runBeforeAiGeneration({ input: inputBefore, configuration });
  const handle = (out.data as SpendGuardHandleStash)._spendguardHandle as ReservationHandle;
  // Build an after-hook input that re-uses the same data object so the
  // stashed handle flows through (same as the Botpress runtime would do).
  return {
    input: { ctx: inputBefore.ctx, data: out.data as AfterData },
    configuration,
    handle,
  };
}

describe("afterAiGeneration hook (A01–A08)", () => {
  let mock: MockSidecarHandle;
  let warnSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(async () => {
    mock = await setupMockSidecar();
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });

  afterEach(async () => {
    await mock.close();
    warnSpy.mockRestore();
  });

  test("A01 test_commit_uses_real_usage", async () => {
    const { input, configuration } = await preBefore(mock);
    input.data.payload = { usage: { inputTokens: 100, outputTokens: 42 } };
    await runAfterAiGeneration({ input, configuration });
    const traceEv = mock.events.find((e) => e.kind === "trace");
    expect(traceEv).toBeDefined();
    const body = traceEv?.body as unknown as Record<string, unknown>;
    expect(body.actual_amount_atomic).toBe("142");
    expect(body.input_tokens).toBe(100);
    expect(body.output_tokens).toBe(42);
    expect(body.outcome).toBe("ACCEPTED");
  });

  test("A02 test_no_usage_estimator_fallback_logs_warn", async () => {
    const { input, configuration } = await preBefore(mock);
    // No payload.usage / data.usage / response.usage. Estimator fallback.
    await runAfterAiGeneration({ input, configuration });
    const traceEv = mock.events.find((e) => e.kind === "trace");
    expect(traceEv).toBeDefined();
    const lines = warnSpy.mock.calls.map((c) => String(c[0]));
    expect(lines.some((l) => l.includes("falling back to estimator"))).toBe(true);
  });

  test("A03 test_after_without_before_is_noop", async () => {
    const configuration = makeConfig({ sidecarUrl: mock.url });
    const input = makeHookInput();
    // No _spendguardHandle stashed — afterAiGeneration must no-op.
    const out = await runAfterAiGeneration({ input, configuration });
    expect(mock.hits.trace).toBe(0);
    expect(out.data).toBeDefined();
  });

  test("A04 test_commit_failure_releases_then_throws", async () => {
    const { input, configuration } = await preBefore(mock);
    input.data.payload = { usage: { inputTokens: 1, outputTokens: 1 } };
    mock.setOptions({ failTraceWith: { status: 500 } });
    let caught: unknown;
    try {
      await runAfterAiGeneration({ input, configuration });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(RuntimeError);
    // Both the commit attempt + the release attempt hit the trace
    // endpoint, but mock counts each hit. Allow >=1.
    expect(mock.hits.trace).toBeGreaterThanOrEqual(1);
  });

  test("A05 test_cancel_releases", async () => {
    const { input, configuration } = await preBefore(mock);
    input.data._cancelled = true;
    await runAfterAiGeneration({ input, configuration });
    const traceEv = mock.events.find((e) => e.kind === "trace");
    expect(traceEv).toBeDefined();
    const body = traceEv?.body as unknown as Record<string, unknown>;
    expect(body.outcome).toBe("REJECTED");
    // CANCELLED classification is logged separately.
    const lines = warnSpy.mock.calls.map((c) => String(c[0]));
    expect(lines.some((l) => l.includes("CANCELLED"))).toBe(true);
  });

  test("A06 test_anthropic_usage_shape_normalised", async () => {
    const { input, configuration } = await preBefore(mock);
    // Anthropic emits raw `input_tokens` / `output_tokens` (snake case)
    // when Botpress hasn't normalised — exercise the response.usage path.
    input.data.response = { usage: { input_tokens: 21, output_tokens: 7 } };
    await runAfterAiGeneration({ input, configuration });
    const traceEv = mock.events.find((e) => e.kind === "trace");
    const body = traceEv?.body as unknown as Record<string, unknown>;
    expect(body.input_tokens).toBe(21);
    expect(body.output_tokens).toBe(7);
    expect(body.actual_amount_atomic).toBe("28");
  });

  test("A07 test_bedrock_usage_shape_normalised", async () => {
    const { input, configuration } = await preBefore(mock);
    // Bedrock InvokeModel via Botpress 0.7 normalises to inputTokens/outputTokens.
    input.data.payload = { usage: { inputTokens: 33, outputTokens: 9 } };
    await runAfterAiGeneration({ input, configuration });
    const traceEv = mock.events.find((e) => e.kind === "trace");
    const body = traceEv?.body as unknown as Record<string, unknown>;
    expect(body.input_tokens).toBe(33);
    expect(body.output_tokens).toBe(9);
  });

  test("A08 test_handle_cleared_from_data_after_commit", async () => {
    const { input, configuration } = await preBefore(mock);
    input.data.payload = { usage: { inputTokens: 5, outputTokens: 5 } };
    await runAfterAiGeneration({ input, configuration });
    // After successful commit, the handle stash is scrubbed.
    expect((input.data as SpendGuardHandleStash)._spendguardHandle).toBeUndefined();
  });
});
