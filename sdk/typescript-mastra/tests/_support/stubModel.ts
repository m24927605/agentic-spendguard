// Recording stub language model for the real-`@mastra/core` Agent tests.
//
// TP-10's observable contract is "DENY ⇒ ZERO provider calls" — the stub
// counts every `doGenerate` / `doStream` invocation so tests assert the
// provider boundary was (or was not) crossed, without any network.
//
// Shape: AI SDK v5 `LanguageModelV2` (`specificationVersion: "v2"`) — one of
// the two model specs the installed `@mastra/core` 1.41.0 agent loop accepts
// (`supportedLanguageModelSpecifications = ["v2", "v3"]`). Mastra's loop
// drives models through `doStream`; `doGenerate` is implemented too so a
// future loop change cannot silently bypass the recorder.

interface StubUsage {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
}

export interface StubModelOptions {
  /** Reply text emitted by every call. Default "stub-reply". */
  replyText?: string;
  /** Usage reported on finish. Default 10/5/15. */
  usage?: StubUsage;
}

/**
 * Counting stub: a minimal `LanguageModelV2` whose only side effect is
 * incrementing `doGenerateCalls` / `doStreamCalls`.
 *
 * Typed structurally (not against the AI SDK package — `@mastra/core`
 * vendors its provider types) and handed to `new Agent({ model })` via the
 * `MastraLanguageModel` union, which accepts a `LanguageModelV2` instance.
 */
export class RecordingStubModel {
  readonly specificationVersion = "v2" as const;
  readonly provider = "spendguard-stub";
  readonly modelId = "stub-model";
  readonly supportedUrls: Record<string, RegExp[]> = {};

  doGenerateCalls = 0;
  doStreamCalls = 0;

  private readonly replyText: string;
  private readonly usage: StubUsage;

  constructor(options: StubModelOptions = {}) {
    this.replyText = options.replyText ?? "stub-reply";
    this.usage = options.usage ?? { inputTokens: 10, outputTokens: 5, totalTokens: 15 };
  }

  /** Total provider-boundary crossings — TP-10 asserts this stays 0 on DENY. */
  get totalCalls(): number {
    return this.doGenerateCalls + this.doStreamCalls;
  }

  async doGenerate(_options: unknown): Promise<unknown> {
    this.doGenerateCalls += 1;
    return {
      content: [{ type: "text", text: this.replyText }],
      finishReason: "stop" as const,
      usage: this.usage,
      warnings: [],
    };
  }

  async doStream(_options: unknown): Promise<unknown> {
    this.doStreamCalls += 1;
    const { replyText, usage } = this;
    const stream = new ReadableStream({
      start(controller) {
        controller.enqueue({ type: "stream-start", warnings: [] });
        controller.enqueue({ type: "text-start", id: "stub-text-1" });
        controller.enqueue({ type: "text-delta", id: "stub-text-1", delta: replyText });
        controller.enqueue({ type: "text-end", id: "stub-text-1" });
        controller.enqueue({ type: "finish", finishReason: "stop", usage });
        controller.close();
      },
    });
    return { stream, warnings: [] };
  }
}

/**
 * Tool-calling variant for TP-12 (1 tool call → 2 reserves): the FIRST step
 * emits a `tool-call` (forcing a tool-call continuation step), every later
 * step emits plain text. Counts crossings like `RecordingStubModel`.
 * Both `doGenerate` and `doStream` are step-aware — Mastra's loop drives
 * `doGenerate` for `agent.generate()` on v2 models.
 */
export class ToolCallingStubModel extends RecordingStubModel {
  private step = 0;
  readonly toolName: string;

  constructor(toolName: string, options: StubModelOptions = {}) {
    super(options);
    this.toolName = toolName;
  }

  override async doGenerate(options: unknown): Promise<unknown> {
    this.step += 1;
    if (this.step > 1) {
      return super.doGenerate(options);
    }
    this.doGenerateCalls += 1;
    return {
      content: [
        // Text alongside the tool call (typical provider behavior): the
        // continuation step's accumulated messages then differ in TEXT
        // PARTS, so §6.3 derives distinct per-step ids. A text-less
        // continuation flattens byte-identically and collapses onto the
        // same idempotency key — the documented §6.3 bullet-3 trade-off.
        { type: "text", text: "calling the tool" },
        {
          type: "tool-call",
          toolCallId: "stub-tool-call-1",
          toolName: this.toolName,
          input: "{}",
        },
      ],
      finishReason: "tool-calls" as const,
      usage: { inputTokens: 10, outputTokens: 5, totalTokens: 15 },
      warnings: [],
    };
  }

  override async doStream(options: unknown): Promise<unknown> {
    this.step += 1;
    if (this.step > 1) {
      return super.doStream(options);
    }
    this.doStreamCalls += 1;
    const toolName = this.toolName;
    const stream = new ReadableStream({
      start(controller) {
        controller.enqueue({ type: "stream-start", warnings: [] });
        controller.enqueue({ type: "text-start", id: "stub-tool-text-1" });
        controller.enqueue({
          type: "text-delta",
          id: "stub-tool-text-1",
          delta: "calling the tool",
        });
        controller.enqueue({ type: "text-end", id: "stub-tool-text-1" });
        controller.enqueue({
          type: "tool-call",
          toolCallId: "stub-tool-call-1",
          toolName,
          input: "{}",
        });
        controller.enqueue({
          type: "finish",
          finishReason: "tool-calls",
          usage: { inputTokens: 10, outputTokens: 5, totalTokens: 15 },
        });
        controller.close();
      },
    });
    return { stream, warnings: [] };
  }
}
