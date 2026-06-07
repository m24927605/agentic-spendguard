// `mockStepAi` — a tiny `step.ai`-shaped shim that fires real-shape Inngest
// runtime context events. Lets D29 wrap.test.ts assert PRE/POST sequencing
// against a fully synchronous in-memory harness without spinning up the
// Inngest dev runtime.
//
// The mock simulates Inngest's retry semantics: the harness re-invokes
// `infer` with `ctx.step.attempt = n+1` and the SAME `step.id` + SAME
// `step.idempotencyKey` when the inner body throws — same as the real
// runtime's deterministic-retry contract.

import { vi } from "vitest";
import type { InngestRuntimeCtx, StepAi } from "../../src/wrapWithSpendGuard.js";

export interface MockStepAiResult {
  stepAi: StepAi;
  inferBody: ReturnType<typeof vi.fn>;
  wrapBody: ReturnType<typeof vi.fn>;
  /** Records the order operations fired across the mock — useful for
   *  assertion of `reserve < provider < commit` sequencing. */
  trace: Array<{ ts: number; op: string }>;
}

export interface MockStepAiOptions {
  /** Result body returned by `infer`. */
  inferReturns?: (name: string, opts: { model: unknown; body: unknown }) => unknown;
  /** When set + `attempt` matches, the inner body THROWS instead of returning. */
  throwOnAttempts?: number[];
  /** Custom error to throw. */
  throwError?: unknown;
}

/**
 * Construct a mock `StepAi` with deterministic-retry-aware behaviour.
 *
 * Tests use the `trace` array to assert ordering between PRE / provider /
 * POST events. Each operation appends a `{ ts, op }` record so the test
 * can assert `trace.find(op="reserve").ts < trace.find(op="provider").ts`.
 */
export function makeMockStepAi(options: MockStepAiOptions = {}): MockStepAiResult {
  const trace: Array<{ ts: number; op: string }> = [];
  let seq = 0;
  const tick = () => ++seq;

  const defaultResult = (_name: string, opts: { model: unknown; body: unknown }) => ({
    id: "chatcmpl-default",
    usage: { total_tokens: 42 },
    choices: [{ message: { content: "ok" } }],
    model: opts.model,
  });

  const inferReturns = options.inferReturns ?? defaultResult;
  const throwOnAttempts = new Set(options.throwOnAttempts ?? []);

  const inferBody = vi.fn(
    async (
      name: string,
      opts: { model: unknown; body: unknown },
      runtimeCtx?: Record<string, unknown>,
    ) => {
      trace.push({ ts: tick(), op: "provider" });
      const ctx = runtimeCtx as InngestRuntimeCtx | undefined;
      const attempt = ctx?.step.attempt ?? 0;
      if (throwOnAttempts.has(attempt)) {
        throw options.throwError ?? new Error(`provider-error-attempt-${attempt}`);
      }
      return inferReturns(name, opts);
    },
  );

  const wrapBody = vi.fn(
    async (_name: string, fn: (...args: never[]) => Promise<unknown>, ...args: unknown[]) => {
      trace.push({ ts: tick(), op: "provider" });
      return fn(...(args as never[]));
    },
  );

  const stepAi: StepAi = {
    infer: inferBody as unknown as StepAi["infer"],
    wrap: wrapBody as unknown as StepAi["wrap"],
  };
  return { stepAi, inferBody, wrapBody, trace };
}

/**
 * Build a synthetic Inngest runtime-context bag — useful for unit tests
 * that exercise the adapter without an Inngest function in scope.
 */
export function makeRuntimeCtx(overrides: Partial<InngestRuntimeCtx> = {}): InngestRuntimeCtx {
  return {
    runId: "run-id-default",
    eventId: "evt-id-default",
    step: {
      id: "step-id-default",
      attempt: 0,
      ...overrides.step,
    },
    ...(overrides.eventId !== undefined ? { eventId: overrides.eventId } : {}),
    ...(overrides.runId !== undefined ? { runId: overrides.runId } : {}),
  };
}
