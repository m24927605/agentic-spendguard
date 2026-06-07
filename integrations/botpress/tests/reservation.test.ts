// reservation.test.ts — unit suite covering R01–R11 per tests.md §2.1.

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  type BotpressCallContext,
  DecisionDenied,
  SidecarUnavailable,
  SpendGuardConfigError,
  SpendGuardReservation,
} from "../src/reservation.js";
import {
  FIXTURE_BUDGET_ID,
  FIXTURE_TENANT_ID,
  FIXTURE_WINDOW_INSTANCE_ID,
  makeConfig,
} from "./_fixtures.js";
import { type MockSidecarHandle, setupMockSidecar } from "./_mockSidecar.js";

const baseCtx: BotpressCallContext = {
  botId: "bot-test-1",
  conversationId: "conv-test-1",
  userId: "user-test-1",
  model: "gpt-4o-mini",
  messages: [{ role: "user", content: "hi" }],
  maxTokens: 100,
};

describe("SpendGuardReservation (R01–R11)", () => {
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

  test("R01 test_construct_requires_sidecar_url", () => {
    expect(
      () =>
        new SpendGuardReservation({
          sidecarUrl: "",
          spendguardBudgetId: FIXTURE_BUDGET_ID,
          spendguardWindowInstanceId: FIXTURE_WINDOW_INSTANCE_ID,
          upstreamProvider: "openai",
          tenantId: FIXTURE_TENANT_ID,
        }),
    ).toThrow(SpendGuardConfigError);
  });

  test("R02 test_construct_requires_budget_ids", () => {
    expect(
      () =>
        new SpendGuardReservation({
          sidecarUrl: mock.url,
          spendguardBudgetId: "",
          spendguardWindowInstanceId: "",
          upstreamProvider: "openai",
          tenantId: FIXTURE_TENANT_ID,
        }),
    ).toThrow(/spendguardBudgetId/);
  });

  test("R03 test_reserve_builds_binding_from_config_and_ctx", async () => {
    const reservation = new SpendGuardReservation(makeConfig({ sidecarUrl: mock.url }));
    const handle = await reservation.reserve(baseCtx);
    expect(handle.reservationId).toMatch(/^res-/);
    expect(handle.decisionId).toMatch(/^dec-/);
    expect(handle.runId.length).toBeGreaterThan(0);
    expect(handle.stepId.length).toBeGreaterThan(0);
    expect(handle.llmCallId.length).toBeGreaterThan(0);
    const ev = mock.events[0];
    expect(ev?.kind).toBe("decision");
    const body = ev?.body as unknown as Record<string, unknown>;
    expect(body.tenant_id).toBe(FIXTURE_TENANT_ID);
    expect(body.budget_id).toBe(FIXTURE_BUDGET_ID);
    expect(body.idempotency_key).toMatch(/^sg-/);
    const ctx = body.decision_context as Record<string, string>;
    expect(ctx.window_instance_id).toBe(FIXTURE_WINDOW_INSTANCE_ID);
    expect(ctx.bot_id).toBe("bot-test-1");
  });

  test("R04 test_reserve_request_decision_payload_shape", async () => {
    const reservation = new SpendGuardReservation(makeConfig({ sidecarUrl: mock.url }));
    await reservation.reserve(baseCtx);
    const ev = mock.events[0];
    const body = ev?.body as unknown as Record<string, unknown>;
    const ctx = body.decision_context as Record<string, string>;
    expect(ctx.integration).toBe("botpress");
    expect(ctx.mode).toBe("integration_sdk");
    expect(ctx.upstream_provider).toBe("openai");
    expect(ctx.bot_id).toBe("bot-test-1");
    expect(ctx.conversation_id).toBe("conv-test-1");
    expect(ctx.user_id).toBe("user-test-1");
    expect(ctx.model).toBe("gpt-4o-mini");
    expect(ctx.window_instance_id).toBe(FIXTURE_WINDOW_INSTANCE_ID);
    expect(ctx.prompt_hash).toMatch(/^[0-9a-f]{64}$/);
    expect(ctx.run_id?.length ?? 0).toBeGreaterThan(0);
    expect(ctx.step_id?.length ?? 0).toBeGreaterThan(0);
    expect(ctx.llm_call_id?.length ?? 0).toBeGreaterThan(0);
    expect(body.model_class).toBe("openai");
    expect(body.prompt_class).toMatch(/^[0-9a-f]{16}$/);
  });

  test("R05 test_reserve_propagates_decision_denied", async () => {
    mock.setOptions({ verdict: "DENY", denyReasonCodes: ["BUDGET_EXCEEDED"] });
    const reservation = new SpendGuardReservation(makeConfig({ sidecarUrl: mock.url }));
    await expect(reservation.reserve(baseCtx)).rejects.toBeInstanceOf(DecisionDenied);
    // No trace POST on DENY.
    expect(mock.hits.trace).toBe(0);
  });

  test("R06 test_reserve_degrade_fail_closed", async () => {
    mock.setOptions({ verdict: "DEGRADE" });
    const reservation = new SpendGuardReservation(makeConfig({ sidecarUrl: mock.url }));
    await expect(reservation.reserve(baseCtx)).rejects.toBeInstanceOf(SidecarUnavailable);
  });

  test("R07 test_reserve_degrade_fail_open_dev_allows", async () => {
    mock.setOptions({ verdict: "DEGRADE" });
    const reservation = new SpendGuardReservation(makeConfig({ sidecarUrl: mock.url }), {
      failOpenDevOverride: true,
    });
    const handle = await reservation.reserve(baseCtx);
    expect(handle.reservationId).toBe("");
    expect(warnSpy).toHaveBeenCalled();
    // commitSuccess no-ops with sentinel handle.
    await reservation.commitSuccess(handle, { inputTokens: 1, outputTokens: 1 }, "");
    expect(mock.hits.trace).toBe(0);
  });

  test("R08 test_commit_success_emits_real_usage", async () => {
    const reservation = new SpendGuardReservation(makeConfig({ sidecarUrl: mock.url }));
    const handle = await reservation.reserve(baseCtx);
    await reservation.commitSuccess(handle, { inputTokens: 100, outputTokens: 42 }, "evt-1");
    const traceEv = mock.events.find((e) => e.kind === "trace");
    expect(traceEv).toBeDefined();
    const body = traceEv?.body as unknown as Record<string, unknown>;
    expect(body.outcome).toBe("ACCEPTED");
    expect(body.input_tokens).toBe(100);
    expect(body.output_tokens).toBe(42);
    expect(body.actual_amount_atomic).toBe("142");
    expect(body.provider_event_id).toBe("evt-1");
  });

  test("R09 test_release_failure_swallows_release_rpc_errors", async () => {
    const reservation = new SpendGuardReservation(makeConfig({ sidecarUrl: mock.url }));
    const handle = await reservation.reserve(baseCtx);
    mock.setOptions({ failTraceWith: { status: 500, body: '{"error":"boom"}' } });
    // Must not re-throw.
    await reservation.releaseFailure(handle, new Error("upstream timeout"));
    expect(warnSpy).toHaveBeenCalled();
  });

  test("R10 test_release_failure_classifies_cancelled", async () => {
    const reservation = new SpendGuardReservation(makeConfig({ sidecarUrl: mock.url }));
    const handle = await reservation.reserve(baseCtx);
    const abortErr = Object.assign(new Error("aborted by user"), { name: "AbortError" });
    await reservation.releaseFailure(handle, abortErr);
    const calls = warnSpy.mock.calls.map((c) => String(c[0]));
    expect(calls.some((line) => line.includes("CANCELLED"))).toBe(true);
  });

  test("R11 test_idempotency_key_derivation_stable", async () => {
    // Same input identity (tenant, conversation, run, step, llmCall, trigger)
    // produces the same idempotency_key. The reservation generates fresh
    // runId/stepId/llmCallId per call by design (every Botpress hook fire
    // is a distinct call), so we verify stability via the underlying helper
    // not the reservation method.
    const { deriveIdempotencyKey } = await import("@spendguard/sdk");
    const args = {
      tenantId: FIXTURE_TENANT_ID,
      sessionId: "conv-test-1",
      runId: "run-fixed",
      stepId: "step-fixed",
      llmCallId: "call-fixed",
      trigger: "LLM_CALL_PRE",
    };
    expect(deriveIdempotencyKey(args)).toBe(deriveIdempotencyKey(args));
  });
});
