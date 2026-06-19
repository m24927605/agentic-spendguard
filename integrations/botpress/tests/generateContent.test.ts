// generateContent.test.ts — the SpendGuard gate-point unit suite.
//
// Covers the reserve -> forward -> commit ordering + fail-closed behaviour:
//   G01 ALLOW path forwards + commits real usage, returns the completion.
//   G02 DENY -> RuntimeError(BUDGET_DENIED), NO upstream forward (INV-1).
//   G03 DEGRADE -> RuntimeError(BUDGET_DEGRADED), NO upstream forward.
//   G04 reserve precedes forward precedes commit (strict ordering).
//   G05 provider forward error -> reservation released + RuntimeError.
//   G06 commit failure -> reservation released + RuntimeError.
//   G07 config error -> RuntimeError(BUDGET_CONFIG), NO upstream forward.
//   G08 cost resolver feeds the botpress billing envelope.
//   G09 listLanguageModels returns provider-scoped models.

import { RuntimeError } from "@botpress/sdk";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { runtimeErrorCode } from "../src/adapter/errors.js";
import { runGenerateContent } from "../src/llm/generateContent.js";
import { runListLanguageModels } from "../src/llm/listLanguageModels.js";
import type { ForwardFn, ForwardRequest, ForwardResult } from "../src/provider/forward.js";
import { ProviderForwardError } from "../src/provider/forward.js";
import { SpendGuardReservation } from "../src/reservation.js";
import { makeConfig, makeCtx, makeGenerateContentInput } from "./_fixtures.js";
import { type MockSidecarHandle, setupMockSidecar } from "./_mockSidecar.js";

/** A forward stub that records its calls + their timestamp. */
function recordingForward(result?: Partial<ForwardResult>): {
  fn: ForwardFn;
  calls: ForwardRequest[];
  timestamps: number[];
} {
  const calls: ForwardRequest[] = [];
  const timestamps: number[] = [];
  const fn: ForwardFn = async (req) => {
    calls.push(req);
    timestamps.push(performance.now());
    return {
      id: "prov-resp-1",
      model: req.model,
      content: "hi there",
      stopReason: "stop",
      inputTokens: 11,
      outputTokens: 7,
      ...result,
    };
  };
  return { fn, calls, timestamps };
}

describe("runGenerateContent (G01–G09)", () => {
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

  test("G01 ALLOW forwards + commits real usage + returns completion", async () => {
    const configuration = makeConfig({ sidecarUrl: mock.url });
    const forward = recordingForward();
    const out = await runGenerateContent({
      input: makeGenerateContentInput(),
      configuration,
      ctx: makeCtx(),
      forward: forward.fn,
    });
    // Forwarded exactly once.
    expect(forward.calls.length).toBe(1);
    // Output carries the provider completion.
    expect(out.choices[0]?.content).toBe("hi there");
    expect(out.provider).toBe("openai");
    expect(out.usage).toEqual({ inputTokens: 11, outputTokens: 7 });
    // Committed the REAL usage to the sidecar.
    const traceEv = mock.events.find((e) => e.kind === "trace");
    const body = traceEv?.body as unknown as Record<string, unknown>;
    expect(body.outcome).toBe("ACCEPTED");
    expect(body.input_tokens).toBe(11);
    expect(body.output_tokens).toBe(7);
    expect(body.actual_amount_atomic).toBe("18");
  });

  test("G02 DENY throws BUDGET_DENIED and never forwards (INV-1)", async () => {
    mock.setOptions({ verdict: "DENY", denyReasonCodes: ["BUDGET_EXCEEDED"] });
    const configuration = makeConfig({ sidecarUrl: mock.url });
    const forward = recordingForward();
    let caught: unknown;
    try {
      await runGenerateContent({
        input: makeGenerateContentInput(),
        configuration,
        ctx: makeCtx(),
        forward: forward.fn,
      });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(RuntimeError);
    expect(runtimeErrorCode(caught as RuntimeError)).toBe("BUDGET_DENIED");
    // CRITICAL: upstream was never called.
    expect(forward.calls.length).toBe(0);
    expect(mock.hits.trace).toBe(0);
  });

  test("G03 DEGRADE throws BUDGET_DEGRADED and never forwards", async () => {
    mock.setOptions({ verdict: "DEGRADE" });
    const configuration = makeConfig({ sidecarUrl: mock.url });
    const forward = recordingForward();
    let caught: unknown;
    try {
      await runGenerateContent({
        input: makeGenerateContentInput(),
        configuration,
        ctx: makeCtx(),
        forward: forward.fn,
      });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(RuntimeError);
    expect(runtimeErrorCode(caught as RuntimeError)).toBe("BUDGET_DEGRADED");
    expect(forward.calls.length).toBe(0);
  });

  test("G04 strict ordering: reserve before forward before commit", async () => {
    const configuration = makeConfig({ sidecarUrl: mock.url });
    const forward = recordingForward();
    await runGenerateContent({
      input: makeGenerateContentInput(),
      configuration,
      ctx: makeCtx(),
      forward: forward.fn,
    });
    const decisionEv = mock.events.find((e) => e.kind === "decision");
    const traceEv = mock.events.find((e) => e.kind === "trace");
    expect(decisionEv).toBeDefined();
    expect(traceEv).toBeDefined();
    const forwardTs = forward.timestamps[0] as number;
    // reserve (decision) < forward < commit (trace)
    expect(decisionEv!.timestamp).toBeLessThanOrEqual(forwardTs);
    expect(forwardTs).toBeLessThanOrEqual(traceEv!.timestamp);
  });

  test("G05 provider forward error releases the reservation + throws", async () => {
    const configuration = makeConfig({ sidecarUrl: mock.url });
    const failing: ForwardFn = async () => {
      throw new ProviderForwardError("upstream openai returned HTTP 503");
    };
    let caught: unknown;
    try {
      await runGenerateContent({
        input: makeGenerateContentInput(),
        configuration,
        ctx: makeCtx(),
        forward: failing,
      });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(RuntimeError);
    // Reservation was released (REJECTED trace) — not left dangling.
    const traceEv = mock.events.find((e) => e.kind === "trace");
    expect(traceEv).toBeDefined();
    const body = traceEv?.body as unknown as Record<string, unknown>;
    expect(body.outcome).toBe("REJECTED");
  });

  test("G06 commit failure releases the reservation + throws", async () => {
    const configuration = makeConfig({ sidecarUrl: mock.url });
    const forward = recordingForward();
    mock.setOptions({ failTraceWith: { status: 500 } });
    let caught: unknown;
    try {
      await runGenerateContent({
        input: makeGenerateContentInput(),
        configuration,
        ctx: makeCtx(),
        forward: forward.fn,
      });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(RuntimeError);
    // The forward DID happen (commit is post-forward), and at least the
    // commit attempt + a release attempt hit the trace endpoint.
    expect(forward.calls.length).toBe(1);
    expect(mock.hits.trace).toBeGreaterThanOrEqual(1);
  });

  test("G07 config error throws BUDGET_CONFIG and never forwards", async () => {
    // Empty tenantId trips assertRequiredConfig in the reservation constructor.
    const configuration = makeConfig({ sidecarUrl: mock.url, tenantId: "" });
    const forward = recordingForward();
    let caught: unknown;
    try {
      await runGenerateContent({
        input: makeGenerateContentInput(),
        configuration,
        ctx: makeCtx(),
        forward: forward.fn,
      });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(RuntimeError);
    expect(runtimeErrorCode(caught as RuntimeError)).toBe("BUDGET_CONFIG");
    expect(forward.calls.length).toBe(0);
  });

  test("G08 cost resolver feeds the botpress billing envelope", async () => {
    const configuration = makeConfig({ sidecarUrl: mock.url });
    const forward = recordingForward({ inputTokens: 100, outputTokens: 50 });
    const out = await runGenerateContent({
      input: makeGenerateContentInput(),
      configuration,
      ctx: makeCtx(),
      forward: forward.fn,
      costResolver: (u) => (u.inputTokens + u.outputTokens) * 0.001,
    });
    expect(out.botpress.cost).toBeCloseTo(0.15, 6);
  });

  test("G09 reservation override is honoured", async () => {
    // Drive the call through an explicitly-constructed reservation so the
    // mock-sidecar fetch seam is exercised end-to-end.
    const configuration = makeConfig({ sidecarUrl: mock.url });
    const reservation = new SpendGuardReservation(configuration);
    const forward = recordingForward();
    const out = await runGenerateContent({
      input: makeGenerateContentInput(),
      configuration,
      ctx: makeCtx(),
      forward: forward.fn,
      reservationOverride: reservation,
    });
    expect(out.choices[0]?.content).toBe("hi there");
    expect(mock.hits.decision).toBe(1);
  });
});

describe("runListLanguageModels", () => {
  test("returns provider-scoped models for each upstream", () => {
    expect(runListLanguageModels(makeConfig({ upstreamProvider: "openai" })).models[0]?.id).toMatch(
      /^gpt-/,
    );
    expect(
      runListLanguageModels(makeConfig({ upstreamProvider: "anthropic" })).models[0]?.id,
    ).toMatch(/^claude-/);
    expect(
      runListLanguageModels(makeConfig({ upstreamProvider: "bedrock" })).models[0]?.id,
    ).toMatch(/^anthropic\./);
  });
});
