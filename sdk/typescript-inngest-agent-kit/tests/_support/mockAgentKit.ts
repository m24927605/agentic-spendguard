// `mockStepAi` — a tiny `step.ai`-shaped shim that fires real-shape Inngest
// runtime context events. Lets D29 wrap.test.ts assert PRE/POST sequencing
// against a fully synchronous in-memory harness without spinning up the
// Inngest dev runtime.
//
// The mock simulates Inngest's retry semantics: the harness re-invokes
// `infer` with `ctx.step.attempt = n+1` and the SAME `step.id` + SAME
// `step.idempotencyKey` when the inner body throws — same as the real
// runtime's deterministic-retry contract.
//
// SLICE 4 extension — `runStepUntil(sg, opts)` simulates Inngest's
// deterministic-retry replay loop end-to-end against the LOCKED public
// `wrapWithSpendGuard(...)` factory: every retry replays the same
// `(runId, stepId, idempotencyKey)` and advances `step.attempt`, exactly
// like the real Inngest runtime.

import { vi } from "vitest";
import type { InngestRuntimeCtx, StepAi } from "../../src/wrapWithSpendGuard.js";

import {
  ApprovalRequired,
  DecisionDenied,
  DecisionSkipped,
  DecisionStopped,
  SidecarUnavailable,
} from "@spendguard/sdk";

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

// ── runStepUntil — Inngest deterministic-retry replay loop ────────────────
//
// SLICE 4 retry-dedup E2E gate. `runStepUntil(sg, opts)` simulates the
// Inngest runtime's retry semantics around a wrapped `step.ai`:
//
//   - Attempt 0 invokes `sg.infer(name, opts, ctx)` with `step.attempt = 0`.
//   - On provider-side throw (any non-typed-substrate Error), the harness
//     re-invokes `sg.infer(...)` with the SAME `(runId, stepId,
//     idempotencyKey)` and `step.attempt = n+1` — exactly mirroring
//     Inngest's deterministic-retry contract.
//   - On any typed substrate error (`DecisionDenied`, `DecisionStopped`,
//     `DecisionSkipped`, `ApprovalRequired`, `SidecarUnavailable`), the
//     harness STOPS retrying (mirrors Inngest's NonRetriable handling for
//     known-fatal errors).
//   - Otherwise replays until `maxAttempts` is reached; the last error
//     surfaces to the caller.
//
// Returns `{ result, attempts }` on success; throws the last attempt's
// error after `maxAttempts` are exhausted.

export interface RunStepUntilArgs {
  /** Max retry attempts the harness will simulate (1 = no retries). */
  maxAttempts: number;
  /** Inngest `ctx.runId` — stable across attempts. */
  runId: string;
  /** Inngest `step.id` — stable across attempts. */
  stepId: string;
  /** Optional `step.idempotencyKey` — stable across attempts when set. */
  idempotencyKey?: string;
  /** Optional Inngest `ctx.eventId`. */
  eventId?: string;
  /** First positional argument to `sg.infer(...)`. */
  callName: string;
  /** Second positional argument to `sg.infer(...)` (`{ model, body }`). */
  callOpts: { model: unknown; body: unknown };
}

export interface RunStepUntilResult<TOut = unknown> {
  result: TOut;
  attempts: number;
}

function isFatalSubstrateError(err: unknown): boolean {
  return (
    err instanceof DecisionDenied ||
    err instanceof DecisionStopped ||
    err instanceof DecisionSkipped ||
    err instanceof ApprovalRequired ||
    err instanceof SidecarUnavailable
  );
}

/**
 * Drive a `wrapWithSpendGuard(...)` result through an Inngest-like
 * deterministic-retry replay loop. Surfaces (a) the final result on
 * success and (b) the number of attempts the harness made — both useful
 * for assertion in the SLICE 4 integration tests.
 */
export async function runStepUntil<TOut = unknown>(
  sg: StepAi,
  args: RunStepUntilArgs,
): Promise<RunStepUntilResult<TOut>> {
  let attempts = 0;
  let lastErr: unknown;
  for (let attempt = 0; attempt < args.maxAttempts; attempt += 1) {
    attempts = attempt + 1;
    const step: InngestRuntimeCtx["step"] = {
      id: args.stepId,
      attempt,
    };
    if (args.idempotencyKey !== undefined) {
      step.idempotencyKey = args.idempotencyKey;
    }
    const ctx: InngestRuntimeCtx = { runId: args.runId, step };
    if (args.eventId !== undefined) {
      ctx.eventId = args.eventId;
    }
    try {
      const result = (await sg.infer(args.callName, args.callOpts, ctx)) as TOut;
      return { result, attempts };
    } catch (err) {
      lastErr = err;
      // Fatal substrate errors — stop replay. Mirrors Inngest's
      // NonRetriable semantics for typed control-flow errors so a
      // DecisionDenied at PRE doesn't keep firing reserves.
      if (isFatalSubstrateError(err)) {
        throw err;
      }
      // Otherwise loop into the next attempt with attempt = n+1.
    }
  }
  throw lastErr;
}
