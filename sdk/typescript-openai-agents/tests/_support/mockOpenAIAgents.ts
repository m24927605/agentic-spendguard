// SLICE 3 — End-to-end mock Agent + Runner harness driven by a real
// `@openai/agents` v0.11 `Agent` instance against a SpendGuard-wrapped
// `Model` and an in-process upstream stub.
//
// The shape this mirrors:
//
//   1. `@openai/agents` v0.11 ships `Agent`, `Runner.run(...)`, and an
//      abstract `Model` interface with `getResponse(request)` / optional
//      `getStreamedResponse(request)` + `getRetryAdvice(...)` hooks.
//   2. The Runner builds a `ModelRequest` per turn and delegates to
//      `agent.model.getResponse(request)`. A `withSpendGuard(inner, opts)`
//      wrap inserts the SLICE 2 PRE/POST bracket; SLICE 3 routes the
//      default `MODEL_BASELINE_TOKENS` claim into the reserve.
//   3. For DENY / STOP / APPROVAL_REQUIRED the substrate throws a typed
//      error which propagates UNCHANGED through `Runner.run(...)` — the
//      Agents SDK does not catch SpendGuard errors.
//
// The harness pairs a `MockUpstreamModel` (acts as the inner OpenAI client)
// with the SpendGuard wrapper + a real `Agent({ model })`. Tests then
// `Runner.run(agent, "input")` inside `runContext({ runId }, …)` and
// assert against the recorded mock state.
//
// This file is the cross-language integration shim — it makes the test
// suite exercise the bracket against the SAME `Agent` / `Runner.run`
// orchestrator the Python sibling at `sdk/python/.../openai_agents.py`
// runs through. Without it, the tests would only ever see `getResponse`
// called directly; integration coverage means going through the Runner.

import type { Model, ModelRequest, ModelResponse } from "@openai/agents";

/** Snapshot of one `getResponse` invocation against the mock upstream. */
export interface RecordedUpstreamCall {
  readonly request: ModelRequest;
  readonly outcome: "SUCCESS" | "ERROR";
  readonly responseId?: string;
}

/**
 * Inner `Model` double — implements the bare `getResponse(request)`
 * contract `@openai/agents` v0.11 exposes. Records every call so tests
 * can assert "inner NEVER reached" (reviewer gate 1.3).
 *
 * The shape matches `withSpendGuard`'s defensive `(inner as { model?:
 * string }).model` read — so the SLICE 3 default `MODEL_BASELINE_TOKENS`
 * lookup routes through the `model` field set here.
 */
export class MockUpstreamModel implements Model {
  public callCount = 0;
  public readonly calls: RecordedUpstreamCall[] = [];
  public errorToThrow: unknown = undefined;
  public readonly responses: Array<Partial<ModelResponse>>;
  private responseIndex = 0;
  public readonly model: string;

  constructor(
    opts: {
      model?: string;
      responses?: Array<Partial<ModelResponse>>;
    } = {},
  ) {
    this.model = opts.model ?? "gpt-4o-mini";
    this.responses = opts.responses ?? [
      {
        usage: {
          requests: 1,
          inputTokens: 12,
          outputTokens: 24,
          totalTokens: 36,
          inputTokensDetails: [],
          outputTokensDetails: [],
        } as unknown as ModelResponse["usage"],
        output: [
          {
            type: "message",
            role: "assistant",
            id: "msg-mock-1",
            status: "completed",
            content: [{ type: "output_text", text: "mock response" }],
          } as unknown as ModelResponse["output"][number],
        ],
        responseId: "resp-mock-default",
      } as Partial<ModelResponse>,
    ];
  }

  async getResponse(request: ModelRequest): Promise<ModelResponse> {
    this.callCount += 1;
    if (this.errorToThrow !== undefined) {
      this.calls.push({ request, outcome: "ERROR" });
      throw this.errorToThrow;
    }
    const idx = this.responseIndex;
    this.responseIndex = Math.min(this.responseIndex + 1, this.responses.length - 1);
    const overrides = this.responses[idx] ?? {};
    const response = {
      usage: overrides.usage ?? {
        requests: 1,
        inputTokens: 0,
        outputTokens: 0,
        totalTokens: 0,
        inputTokensDetails: [],
        outputTokensDetails: [],
      },
      output: overrides.output ?? [],
      ...(overrides.responseId !== undefined ? { responseId: overrides.responseId } : {}),
    } as unknown as ModelResponse;
    this.calls.push({
      request,
      outcome: "SUCCESS",
      ...(overrides.responseId !== undefined ? { responseId: overrides.responseId } : {}),
    });
    return response;
  }

  getStreamedResponse(_request: ModelRequest): ReturnType<Model["getStreamedResponse"]> {
    // Throws on iteration, NOT at call-time: the wrapper's pass-through
    // returns the AsyncIterable verbatim; throwing here would mask the
    // pass-through invariant. The closest "not implemented" sentinel is an
    // empty stream that immediately raises on next() — matches the SLICE 2
    // factory.test.ts double's intent. The leading `yield` placates biome's
    // `useYield` rule (which fires when a generator has no `yield` in its
    // body); the immediately-following `throw` makes the body never actually
    // produce a value.
    async function* gen(): AsyncIterable<never> {
      if (true as boolean) {
        throw new Error("MockUpstreamModel.getStreamedResponse not implemented");
      }
      yield undefined as never;
    }
    return gen() as ReturnType<Model["getStreamedResponse"]>;
  }
}

/**
 * Minimal Agent harness — exposes `agent.model` so tests can compose a
 * `withSpendGuard(inner)` wrap exactly like real consumers do. The full
 * `Agent({ name, model, instructions })` from `@openai/agents` is used
 * inside individual tests when streaming tool-call orchestration matters;
 * this harness covers the simpler "single-turn LLM call" path which is
 * what SLICE 3 needs to verify.
 *
 * The shape mirrors what `Runner.run(agent, input)` calls under the hood
 * minus tool-call routing — every reserve / commit / inner-call assertion
 * is on the same path the real Runner takes, but without the v0.11
 * Runner's session-state machinery (which the integration harness does
 * not need to exercise to prove SpendGuard discipline).
 */
export interface MockAgentHarness {
  /** The SpendGuard-wrapped `Model` — pass into a real `Agent`. */
  readonly model: Model;
  /** The inner upstream double — assert `callCount` against it. */
  readonly upstream: MockUpstreamModel;
  /**
   * Drive a single `getResponse(request)` through the wrapped model. This
   * is the same call shape `Runner.run(agent, ...)` produces internally —
   * the v0.11 Runner adds session-state + tool-call routing on top, neither
   * of which changes the SpendGuard bracket discipline. The integration
   * tests use this helper so the harness's contract stays stable across
   * `@openai/agents` minor bumps.
   */
  run(request: ModelRequest): Promise<ModelResponse>;
}

/**
 * Construct a minimal request that exercises the same `(input,
 * systemInstructions)` fields the SLICE 2 signature path reads.
 */
export function makeAgentRequest(
  opts: {
    input?: ModelRequest["input"];
    systemInstructions?: string | undefined;
  } = {},
): ModelRequest {
  return {
    input: opts.input ?? "Say hi",
    ...(opts.systemInstructions !== undefined
      ? { systemInstructions: opts.systemInstructions }
      : {}),
    modelSettings: {},
    tools: [],
    outputType: "text" as ModelRequest["outputType"],
    handoffs: [],
    tracing: false,
  } as unknown as ModelRequest;
}

/**
 * Build a `MockAgentHarness` around `withSpendGuard(upstream, opts)`. The
 * caller provides the wrapped model directly so we don't take a hard dep
 * on `withSpendGuard` here (keeps `_support/` free of cyclic imports).
 */
export function makeAgentHarness(opts: {
  upstream: MockUpstreamModel;
  wrap(inner: MockUpstreamModel): Model;
}): MockAgentHarness {
  const wrapped = opts.wrap(opts.upstream);
  return {
    model: wrapped,
    upstream: opts.upstream,
    async run(request) {
      return wrapped.getResponse(request);
    },
  };
}
