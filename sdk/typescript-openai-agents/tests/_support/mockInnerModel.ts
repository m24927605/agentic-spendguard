// In-process `@openai/agents` `Model` double. The wrapper interacts with
// the inner only through `getResponse(request)` + the optional
// `getStreamedResponse(request)` and `getRetryAdvice(...)` hooks.
//
// `callCount` is the workhorse counter every "inner NEVER reached" test
// asserts against — reviewer gate 1.3.

import type { Model, ModelRequest, ModelResponse } from "@openai/agents";

export interface MockInnerModel extends Model {
  callCount: number;
  lastRequest: ModelRequest | undefined;
  responseFactory: (request: ModelRequest) => ModelResponse;
  errorToThrow: unknown;
}

export function makeMockInnerModel(
  opts: {
    model?: string;
    response?: Partial<ModelResponse>;
    responseFactory?: (request: ModelRequest) => ModelResponse;
  } = {},
): MockInnerModel {
  const responseFactory =
    opts.responseFactory ??
    ((_req: ModelRequest): ModelResponse => makeMockResponse(opts.response ?? {}));
  const inner: MockInnerModel = {
    callCount: 0,
    lastRequest: undefined,
    responseFactory,
    errorToThrow: undefined,
    // Stamp a `.model` field — `withSpendGuard` reads it to populate the
    // SLICE 3 default-estimator routing. Useful even at SLICE 2 so tests
    // can verify the field flows into the bracket without breaking the
    // surface lock.
    ...(opts.model !== undefined ? { model: opts.model } : {}),
    async getResponse(request: ModelRequest): Promise<ModelResponse> {
      inner.callCount += 1;
      inner.lastRequest = request;
      if (inner.errorToThrow !== undefined) {
        throw inner.errorToThrow;
      }
      return inner.responseFactory(request);
    },
    getStreamedResponse(_request: ModelRequest) {
      throw new Error("MockInnerModel.getStreamedResponse not implemented");
    },
  };
  return inner;
}

export function makeMockResponse(overrides: Partial<ModelResponse> = {}): ModelResponse {
  return {
    usage: overrides.usage ?? makeMockUsage(),
    output: overrides.output ?? [],
    ...(overrides.responseId !== undefined ? { responseId: overrides.responseId } : {}),
    ...(overrides.requestId !== undefined ? { requestId: overrides.requestId } : {}),
    ...(overrides.providerData !== undefined ? { providerData: overrides.providerData } : {}),
  } as unknown as ModelResponse;
}

export function makeMockUsage(
  opts: {
    inputTokens?: number;
    outputTokens?: number;
    totalTokens?: number;
  } = {},
) {
  const inputTokens = opts.inputTokens ?? 12;
  const outputTokens = opts.outputTokens ?? 24;
  const totalTokens = opts.totalTokens ?? inputTokens + outputTokens;
  return {
    requests: 1,
    inputTokens,
    outputTokens,
    totalTokens,
    inputTokensDetails: [],
    outputTokensDetails: [],
    // Snake_case mirror is intentionally NOT populated here — the
    // extractor handles camelCase first; specific tests construct a
    // snake_case-only shape to exercise the fallback path.
  } as unknown as ModelResponse["usage"];
}

export function makeRequest(
  opts: {
    input?: ModelRequest["input"];
    systemInstructions?: string | undefined;
  } = {},
): ModelRequest {
  return {
    input: opts.input ?? "Say hello",
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
