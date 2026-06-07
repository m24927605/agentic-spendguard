// D06 SLICE 6 — In-process `LanguageModelV1` doubles that mimic the
// `@ai-sdk/openai` + `@ai-sdk/anthropic` provider shapes.
//
// **Why hand-rolled doubles vs the real `@ai-sdk/openai`/`@ai-sdk/anthropic`
// packages?**: those packages pull HTTP fetch + auth + retry surface
// transitively; the SLICE 6 acceptance criterion is "the middleware works
// with provider-shaped `LanguageModelV1` implementations" — wire-level HTTP
// recording is anti-scope per design.md §3 (provider matrix testing is a
// future hardening). The hand-rolled doubles match the LanguageModelV1
// surface that the real OpenAI / Anthropic providers expose, so the
// middleware exercises the same `doGenerate` / `doStream` result-shape
// contract end-to-end without the heavyweight package dependency tree.
//
// Recorded fixtures live as TypeScript literal objects (`OPENAI_FIXTURES`
// / `ANTHROPIC_FIXTURES`) at the bottom of this file. They mirror the
// `usage` shape the real providers report:
//
//   - OpenAI:   `usage: { promptTokens: N, completionTokens: M }`
//               (Vercel AI SDK v4 canonical camelCase form returned by
//               `@ai-sdk/openai@^1`)
//   - Anthropic: `usage: { promptTokens: N, completionTokens: M }`
//               (Vercel AI SDK v4 canonical camelCase form returned by
//               `@ai-sdk/anthropic@^1`; the underlying Anthropic API
//               uses `input_tokens`/`output_tokens`, the AI SDK adapter
//               normalises to camelCase)
//
// The provider-specific surface (`provider` string + `modelId`) is also
// preserved so middleware tests can assert per-provider routing if needed.

import type {
  LanguageModelV1,
  LanguageModelV1CallOptions,
  LanguageModelV1StreamPart,
} from "ai";

/**
 * `LanguageModelV1FinishReason` mirror — the canonical type lives in
 * `@ai-sdk/provider` but is NOT re-exported from `ai@4`'s public surface.
 * Inlined here so the fixture types stay public + the support module
 * does not take a direct dev-dep on `@ai-sdk/provider`. Values mirror
 * the canonical enum at `@ai-sdk/provider@1.x` verbatim.
 *
 * The real `@ai-sdk/anthropic` adapter normalises Anthropic's
 * `end_turn` / `max_tokens` / `tool_use` finish reasons to this
 * canonical enum on the way out; the SLICE 6 Anthropic fixture
 * therefore reports `"stop"` to match what an end-to-end caller would
 * actually see.
 */
type LanguageModelV1FinishReason =
  | "stop"
  | "length"
  | "content-filter"
  | "tool-calls"
  | "error"
  | "other"
  | "unknown";

// ── Fixture types ─────────────────────────────────────────────────────────

/**
 * Recorded `doGenerate` response shape — the minimal subset of the
 * `LanguageModelV1.doGenerate` return type the middleware actually
 * consumes (`text` + `usage` + `finishReason` + `rawCall`). Other
 * fields (raw response headers, sources, logprobs, …) are ignored by
 * the middleware so the fixture stays compact.
 */
export interface RecordedGenerateResponse {
  text: string;
  usage: { promptTokens: number; completionTokens: number };
  finishReason: LanguageModelV1FinishReason;
}

/**
 * Recorded `doStream` part sequence — array of stream parts the mock
 * emits in order. The `finish` part carries the same `usage` shape the
 * `RecordedGenerateResponse` does so streaming tests share the fixture
 * declaration site.
 */
export interface RecordedStreamSequence {
  parts: ReadonlyArray<LanguageModelV1StreamPart>;
}

// ── Provider-shaped LanguageModelV1 implementations ───────────────────────

/**
 * Mock OpenAI provider. Returns the configured fixture for `doGenerate`
 * + `doStream`. Mirrors the surface `@ai-sdk/openai@^1`'s `chatModel`
 * exposes — `provider: "openai.chat"`, `modelId: "gpt-4o-mini"` by
 * default. Each call records the `LanguageModelV1CallOptions` it
 * received so tests can assert request shape parity with what the real
 * `@ai-sdk/openai` would have built before its HTTP send.
 *
 * Fields the real provider sets (`defaultObjectGenerationMode: "tool"`,
 * `supportsImageUrls: true`, `supportsStructuredOutputs: true`) are
 * mirrored verbatim so type assertions against `LanguageModelV1` hold.
 */
export class MockOpenAIModel implements LanguageModelV1 {
  readonly specificationVersion = "v1" as const;
  readonly provider = "openai.chat";
  readonly defaultObjectGenerationMode = "tool" as const;
  readonly supportsImageUrls = true;
  readonly supportsStructuredOutputs = true;

  readonly modelId: string;

  readonly generateCalls: LanguageModelV1CallOptions[] = [];
  readonly streamCalls: LanguageModelV1CallOptions[] = [];

  private readonly generateFixture: RecordedGenerateResponse;
  private readonly streamFixture: RecordedStreamSequence;
  private readonly errorOnNthGenerate: number | undefined;
  private readonly errorMessage: string;

  constructor(opts: {
    modelId?: string;
    generateFixture?: RecordedGenerateResponse;
    streamFixture?: RecordedStreamSequence;
    /** Throw on the Nth `doGenerate` (1-indexed). */
    errorOnNthGenerate?: number;
    errorMessage?: string;
  } = {}) {
    this.modelId = opts.modelId ?? "gpt-4o-mini";
    this.generateFixture = opts.generateFixture ?? OPENAI_FIXTURES.simpleAllow;
    this.streamFixture = opts.streamFixture ?? OPENAI_FIXTURES.simpleStream;
    this.errorOnNthGenerate = opts.errorOnNthGenerate;
    this.errorMessage = opts.errorMessage ?? "synthetic openai provider error (429 rate-limited)";
  }

  // The official `LanguageModelV1` shape declares `doGenerate` /
  // `doStream` with a `PromiseLike<...>` return type. TypeScript's
  // `async` keyword always lowers to `Promise<T>` which is structurally
  // assignable to `PromiseLike<T>` but the compiler short-circuits the
  // check at the method-declaration site. We sidestep the warning by
  // declaring the methods as ordinary functions that explicitly return
  // a Promise, which IS a PromiseLike. The wire-level behaviour is
  // identical.
  doGenerate(
    options: LanguageModelV1CallOptions,
  ): ReturnType<LanguageModelV1["doGenerate"]> {
    this.generateCalls.push(options);
    if (this.errorOnNthGenerate !== undefined &&
        this.generateCalls.length === this.errorOnNthGenerate) {
      return Promise.reject(new Error(this.errorMessage));
    }
    const fx = this.generateFixture;
    return Promise.resolve({
      text: fx.text,
      usage: fx.usage,
      finishReason: fx.finishReason,
      rawCall: {
        rawPrompt: options.prompt,
        rawSettings: {
          temperature: options.temperature ?? 0,
          model: this.modelId,
        },
      },
      rawResponse: { headers: { "x-mock-provider": "openai" } },
      warnings: [],
    });
  }

  doStream(
    options: LanguageModelV1CallOptions,
  ): ReturnType<LanguageModelV1["doStream"]> {
    this.streamCalls.push(options);
    const fx = this.streamFixture;
    const stream = new ReadableStream<LanguageModelV1StreamPart>({
      start(controller) {
        for (const p of fx.parts) {
          controller.enqueue(p);
        }
        controller.close();
      },
    });
    return Promise.resolve({
      stream,
      rawCall: {
        rawPrompt: options.prompt,
        rawSettings: {
          temperature: options.temperature ?? 0,
          model: this.modelId,
        },
      },
      rawResponse: { headers: { "x-mock-provider": "openai" } },
      warnings: [],
    });
  }

  reset(): void {
    this.generateCalls.length = 0;
    this.streamCalls.length = 0;
  }
}

/**
 * Mock Anthropic provider. Same shape as `MockOpenAIModel` — the real
 * `@ai-sdk/anthropic` adapter also implements `LanguageModelV1` and
 * already normalises Anthropic's `input_tokens`/`output_tokens` shape
 * to the Vercel AI SDK canonical `{promptTokens, completionTokens}`.
 *
 * Surface differences mirrored verbatim:
 *   - `provider: "anthropic.messages"`
 *   - `modelId: "claude-3-5-sonnet-20241022"` (default)
 *   - `defaultObjectGenerationMode: "tool"` (same as OpenAI)
 *   - `supportsImageUrls: true`
 *   - `supportsStructuredOutputs: false` (Anthropic does not gate
 *     structured outputs the same way)
 */
export class MockAnthropicModel implements LanguageModelV1 {
  readonly specificationVersion = "v1" as const;
  readonly provider = "anthropic.messages";
  readonly defaultObjectGenerationMode = "tool" as const;
  readonly supportsImageUrls = true;
  readonly supportsStructuredOutputs = false;

  readonly modelId: string;

  readonly generateCalls: LanguageModelV1CallOptions[] = [];
  readonly streamCalls: LanguageModelV1CallOptions[] = [];

  private readonly generateFixture: RecordedGenerateResponse;
  private readonly streamFixture: RecordedStreamSequence;
  private readonly errorOnNthGenerate: number | undefined;
  private readonly errorMessage: string;

  constructor(opts: {
    modelId?: string;
    generateFixture?: RecordedGenerateResponse;
    streamFixture?: RecordedStreamSequence;
    errorOnNthGenerate?: number;
    errorMessage?: string;
  } = {}) {
    this.modelId = opts.modelId ?? "claude-3-5-sonnet-20241022";
    this.generateFixture = opts.generateFixture ?? ANTHROPIC_FIXTURES.simpleAllow;
    this.streamFixture = opts.streamFixture ?? ANTHROPIC_FIXTURES.simpleStream;
    this.errorOnNthGenerate = opts.errorOnNthGenerate;
    this.errorMessage = opts.errorMessage ?? "synthetic anthropic provider error (overloaded_error)";
  }

  // See `MockOpenAIModel.doGenerate` comment for the
  // PromiseLike vs Promise return-type rationale.
  doGenerate(
    options: LanguageModelV1CallOptions,
  ): ReturnType<LanguageModelV1["doGenerate"]> {
    this.generateCalls.push(options);
    if (this.errorOnNthGenerate !== undefined &&
        this.generateCalls.length === this.errorOnNthGenerate) {
      return Promise.reject(new Error(this.errorMessage));
    }
    const fx = this.generateFixture;
    return Promise.resolve({
      text: fx.text,
      usage: fx.usage,
      finishReason: fx.finishReason,
      rawCall: {
        rawPrompt: options.prompt,
        rawSettings: {
          temperature: options.temperature ?? 0,
          model: this.modelId,
        },
      },
      rawResponse: { headers: { "x-mock-provider": "anthropic" } },
      warnings: [],
    });
  }

  doStream(
    options: LanguageModelV1CallOptions,
  ): ReturnType<LanguageModelV1["doStream"]> {
    this.streamCalls.push(options);
    const fx = this.streamFixture;
    const stream = new ReadableStream<LanguageModelV1StreamPart>({
      start(controller) {
        for (const p of fx.parts) {
          controller.enqueue(p);
        }
        controller.close();
      },
    });
    return Promise.resolve({
      stream,
      rawCall: {
        rawPrompt: options.prompt,
        rawSettings: {
          temperature: options.temperature ?? 0,
          model: this.modelId,
        },
      },
      rawResponse: { headers: { "x-mock-provider": "anthropic" } },
      warnings: [],
    });
  }

  reset(): void {
    this.generateCalls.length = 0;
    this.streamCalls.length = 0;
  }
}

// ── Recorded fixtures ─────────────────────────────────────────────────────

/**
 * OpenAI-shaped recorded responses. Token counts mirror what real
 * `gpt-4o-mini` returns for short prompts; the `text-delta` part
 * sequence in `simpleStream` mirrors the chunked SSE stream the AI
 * SDK's OpenAI provider transforms into LanguageModelV1StreamPart.
 */
export const OPENAI_FIXTURES: {
  readonly simpleAllow: RecordedGenerateResponse;
  readonly simpleStream: RecordedStreamSequence;
  readonly bigCompletion: RecordedGenerateResponse;
  readonly emptyStream: RecordedStreamSequence;
} = {
  simpleAllow: {
    text: "hello back from openai gpt-4o-mini",
    usage: { promptTokens: 12, completionTokens: 8 },
    finishReason: "stop",
  },
  simpleStream: {
    parts: [
      { type: "text-delta", textDelta: "hello " },
      { type: "text-delta", textDelta: "back " },
      { type: "text-delta", textDelta: "stream" },
      {
        type: "finish",
        finishReason: "stop",
        usage: { promptTokens: 14, completionTokens: 6 },
      },
    ],
  },
  bigCompletion: {
    text: "this is a longer openai completion " + "x".repeat(200),
    usage: { promptTokens: 50, completionTokens: 250 },
    finishReason: "stop",
  },
  emptyStream: {
    parts: [
      {
        type: "finish",
        finishReason: "stop",
        usage: { promptTokens: 5, completionTokens: 0 },
      },
    ],
  },
};

/**
 * Anthropic-shaped recorded responses. The token counts here are
 * what `@ai-sdk/anthropic` would report after normalising the
 * Anthropic API's `input_tokens` / `output_tokens` fields to the AI
 * SDK canonical `{promptTokens, completionTokens}` shape.
 */
export const ANTHROPIC_FIXTURES: {
  readonly simpleAllow: RecordedGenerateResponse;
  readonly simpleStream: RecordedStreamSequence;
  readonly bigCompletion: RecordedGenerateResponse;
} = {
  simpleAllow: {
    text: "hello back from anthropic claude-3-5-sonnet",
    usage: { promptTokens: 18, completionTokens: 11 },
    // `@ai-sdk/anthropic` normalises Anthropic's `end_turn` to canonical
    // `"stop"` on the way out — see @ai-sdk/anthropic@^1
    // `mapAnthropicFinishReason`. Test 4 asserts against this normalised
    // value, not the raw Anthropic API value.
    finishReason: "stop",
  },
  simpleStream: {
    parts: [
      { type: "text-delta", textDelta: "Hello " },
      { type: "text-delta", textDelta: "from " },
      { type: "text-delta", textDelta: "Claude!" },
      {
        type: "finish",
        finishReason: "stop",
        usage: { promptTokens: 20, completionTokens: 4 },
      },
    ],
  },
  bigCompletion: {
    text: "this is a longer anthropic completion " + "y".repeat(180),
    usage: { promptTokens: 40, completionTokens: 210 },
    finishReason: "stop",
  },
};

/**
 * Helper: build a `LanguageModelV1CallOptions` reference suitable for
 * driving `wrapLanguageModel(...).doGenerate(...)` or `.doStream(...)`
 * directly. Each call returns a fresh object reference so WeakMap key
 * identity tests exercise distinct keys.
 */
export function makeCallOptions(promptText: string): LanguageModelV1CallOptions {
  return {
    inputFormat: "messages",
    mode: { type: "regular" },
    prompt: [
      {
        role: "user",
        content: [{ type: "text", text: promptText }],
      },
    ],
  };
}
