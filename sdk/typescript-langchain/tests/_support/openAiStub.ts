// SLICE 4 — Stubbed `fetch` implementation for `@langchain/openai`'s
// ChatOpenAI. Replaces the real OpenAI HTTP call with a deterministic
// canned response so the tests verify the SpendGuardCallbackHandler ↔
// `BaseChatModel` integration without ever touching the network.
//
// `ChatOpenAI` constructor accepts a `configuration: { fetch }` field
// (forwarded to the `openai` package's `OpenAI` client). The stub is plugged
// in there; it records every call so tests can assert against the exact
// request shape AND the order of operations (reserve → fetch → commit).
//
// The stub returns a chat-completions-shaped envelope by default, with
// `prompt_tokens` / `completion_tokens` / `total_tokens` populated so the
// handler's `extractTokenUsage` path is exercised end-to-end.

/** Recorded HTTP request shape — captured for assertion. */
export interface RecordedFetchCall {
  url: string;
  method: string;
  body?: unknown;
  callIndex: number;
}

/** Canned chat-completion response builder options. */
export interface OpenAiStubResponseOptions {
  /** Content of the single response choice. Defaults to "ok". */
  content?: string;
  /** Token counts surfaced on the response envelope. */
  promptTokens?: number;
  completionTokens?: number;
  /**
   * When set, the stub returns the HTTP status + JSON body — used to drive
   * the handler's `handleLLMError` PROVIDER_ERROR path (e.g. 429 / 503).
   */
  errorStatus?: number;
  errorMessage?: string;
}

/** Per-call queue of canned responses; falls back to a default success. */
export interface OpenAiStubOptions {
  responseQueue?: OpenAiStubResponseOptions[];
  defaultResponse?: OpenAiStubResponseOptions;
}

const DEFAULT_RESPONSE: Required<
  Omit<OpenAiStubResponseOptions, "errorStatus" | "errorMessage">
> & {
  errorStatus?: number;
  errorMessage?: string;
} = {
  content: "ok",
  promptTokens: 10,
  completionTokens: 5,
};

/** Mock implementation of the global `fetch` for OpenAI's chat-completions API. */
export class OpenAiStub {
  private readonly responseQueue: OpenAiStubResponseOptions[];
  private readonly defaultResponse: OpenAiStubResponseOptions;
  /** All `fetch` invocations in arrival order. */
  readonly fetchCalls: RecordedFetchCall[] = [];
  private callCounter = 0;

  constructor(options: OpenAiStubOptions = {}) {
    this.responseQueue = options.responseQueue ? [...options.responseQueue] : [];
    this.defaultResponse = options.defaultResponse ?? {};
  }

  /**
   * Returns the stub-bound `fetch` implementation suitable for
   * `ChatOpenAI({ configuration: { fetch } })`. The signature is widened to
   * `(input: unknown, init?) => Promise<Response>` so it satisfies both the
   * Node global fetch shape (`string | URL | Request`) AND the `openai`
   * package's `Fetch` type (which uses an `unknown`-degraded `RequestInfo`).
   * Callers cast to the host's preferred signature at the use site.
   */
  get fetch(): (input: unknown, init?: RequestInit) => Promise<Response> {
    return (input, init) => this.handleFetch(input, init);
  }

  /** Convenience: how many times the stubbed fetch has been called. */
  get callCount(): number {
    return this.fetchCalls.length;
  }

  /** Reset captured calls (useful between sequential test phases). */
  reset(): void {
    this.fetchCalls.length = 0;
    this.callCounter = 0;
  }

  private async handleFetch(input: unknown, init?: RequestInit): Promise<Response> {
    this.callCounter += 1;
    const callIndex = this.callCounter;
    const url = extractUrl(input);
    const method = init?.method ?? "POST";
    let body: unknown;
    if (typeof init?.body === "string") {
      try {
        body = JSON.parse(init.body);
      } catch {
        body = init.body;
      }
    }
    this.fetchCalls.push({ url, method, body, callIndex });

    const plan = this.responseQueue.shift() ?? this.defaultResponse;
    if (plan.errorStatus !== undefined) {
      // Simulate an OpenAI API error envelope (matches the shape the openai
      // SDK parses into an `APIError` so `ChatOpenAI` surfaces it as a thrown
      // error from `invoke()` — exercising the handler's `handleLLMError`
      // path).
      const errorBody = {
        error: {
          message: plan.errorMessage ?? "synthetic provider error",
          type: "server_error",
          code: "synthetic_error",
        },
      };
      return new Response(JSON.stringify(errorBody), {
        status: plan.errorStatus,
        statusText: plan.errorMessage ?? "Synthetic Error",
        headers: { "Content-Type": "application/json" },
      });
    }

    const responseBody = buildChatCompletionBody({
      content: plan.content ?? DEFAULT_RESPONSE.content,
      promptTokens: plan.promptTokens ?? DEFAULT_RESPONSE.promptTokens,
      completionTokens: plan.completionTokens ?? DEFAULT_RESPONSE.completionTokens,
      callIndex,
    });
    return new Response(JSON.stringify(responseBody), {
      status: 200,
      statusText: "OK",
      headers: { "Content-Type": "application/json" },
    });
  }
}

/**
 * Coerce any of the values the openai package may pass as `input` to a
 * loggable URL string. The package internally normalises to a string or a
 * `Request`-shaped object before invoking the fetch impl, but we accept any
 * shape (defensive: `unknown`) so the stub stays robust to upstream churn.
 */
function extractUrl(input: unknown): string {
  if (typeof input === "string") return input;
  if (input instanceof URL) return input.toString();
  if (typeof input === "object" && input !== null && "url" in input) {
    const url = (input as { url?: unknown }).url;
    if (typeof url === "string") return url;
  }
  return String(input);
}

/**
 * Build a canned OpenAI chat-completions JSON envelope. Mirrors the shape
 * `@langchain/openai` post-processes into `LLMResult.llmOutput.tokenUsage`.
 */
function buildChatCompletionBody(args: {
  content: string;
  promptTokens: number;
  completionTokens: number;
  callIndex: number;
}): Record<string, unknown> {
  return {
    id: `chatcmpl-mock-${args.callIndex}`,
    object: "chat.completion",
    created: 1_700_000_000 + args.callIndex,
    model: "gpt-4o-mini",
    choices: [
      {
        index: 0,
        message: {
          role: "assistant",
          content: args.content,
        },
        finish_reason: "stop",
      },
    ],
    usage: {
      prompt_tokens: args.promptTokens,
      completion_tokens: args.completionTokens,
      total_tokens: args.promptTokens + args.completionTokens,
    },
  };
}
