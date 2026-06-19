// Upstream provider forward for the SpendGuard `generateContent` action.
//
// After SpendGuard reserves budget, the integration forwards the prompt to the
// configured upstream provider and returns the real completion + usage. The
// first-party Botpress OpenAI integration delegates this to
// `@botpress/common`'s `llm.openai.generateContent(...)` helper, but
// `@botpress/common` is an internal Botpress workspace package that is not
// published to npm. We therefore implement a small, provider-agnostic forward
// that speaks each provider's chat-completions wire shape directly.
//
// The forward is INJECTABLE (`ForwardFn`): the runtime accepts an optional
// override so the unit tier drives the reserve -> forward -> commit ordering
// without a live provider socket, exactly as the SpendGuard sidecar mock does
// for the budget RPCs. Production uses `defaultForward`, which dials the
// provider over HTTPS using the API key from the integration's environment.
//
// Anti-scope (v1): one text choice, no streaming, no tool calls. Botpress's
// LLM router falls back to non-streaming generateContent when streaming is
// unavailable, so this is a correct (if minimal) provider surface.

import type { Configuration } from "../config.js";
import type { GenerateContentInput, GenerateContentOutput, Message } from "../llm/schemas.js";

/** Resolved per-call forward request — the normalised inputs every provider
 *  branch consumes. */
export interface ForwardRequest {
  readonly provider: Configuration["upstreamProvider"];
  readonly model: string;
  readonly messages: ReadonlyArray<Message>;
  readonly systemPrompt: string | undefined;
  readonly maxTokens: number;
  readonly temperature: number | undefined;
  readonly topP: number | undefined;
  readonly stopSequences: ReadonlyArray<string> | undefined;
  readonly userId: string | undefined;
}

/** Normalised forward result — the provider's completion + real usage. */
export interface ForwardResult {
  readonly id: string;
  readonly model: string;
  readonly content: string;
  readonly stopReason: "stop" | "max_tokens" | "content_filter" | "other";
  readonly inputTokens: number;
  readonly outputTokens: number;
}

/** Pluggable forward seam. Injected by tests; defaults to `defaultForward`. */
export type ForwardFn = (req: ForwardRequest) => Promise<ForwardResult>;

/** Raised when the upstream provider call itself fails (network / 5xx / auth).
 *  Distinct from the SpendGuard budget errors so the runtime can release the
 *  reservation and surface a provider-flavoured RuntimeError. */
export class ProviderForwardError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ProviderForwardError";
  }
}

/** Build the normalised `ForwardRequest` from the action input + config. The
 *  resolved `model` prefers the explicit `input.model.id`, then the
 *  provider-default model. `maxTokens` mirrors the SpendGuard reserve estimate
 *  so the upstream cap and the reserved budget agree. */
export function toForwardRequest(
  input: GenerateContentInput,
  config: Configuration,
  resolvedModel: string,
  resolvedMaxTokens: number,
): ForwardRequest {
  return {
    provider: config.upstreamProvider,
    model: resolvedModel,
    messages: input.messages,
    systemPrompt: input.systemPrompt,
    maxTokens: resolvedMaxTokens,
    temperature: input.temperature,
    topP: input.topP,
    stopSequences: input.stopSequences,
    userId: input.userId,
  };
}

/** Map a `ForwardResult` into the action's `GenerateContentOutput`. `cost` is
 *  left to the caller (it depends on the SpendGuard pricing freeze), so this
 *  takes the resolved USD cost as an argument. */
export function toGenerateContentOutput(
  result: ForwardResult,
  provider: string,
  cost: number,
): GenerateContentOutput {
  return {
    id: result.id,
    provider,
    model: result.model,
    choices: [
      {
        role: "assistant",
        type: "text",
        content: result.content,
        index: 0,
        stopReason: result.stopReason,
      },
    ],
    usage: {
      inputTokens: result.inputTokens,
      outputTokens: result.outputTokens,
    },
    botpress: { cost },
  };
}

/** Provider HTTPS endpoints + the env var holding each provider's API key.
 *  Bedrock is reached via its OpenAI-compatible gateway shape in v1; the
 *  region/credential wiring lives in the deployment environment. */
const PROVIDER_ENDPOINTS: Record<
  Configuration["upstreamProvider"],
  { url: string; apiKeyEnv: string }
> = {
  openai: { url: "https://api.openai.com/v1/chat/completions", apiKeyEnv: "OPENAI_API_KEY" },
  anthropic: { url: "https://api.anthropic.com/v1/messages", apiKeyEnv: "ANTHROPIC_API_KEY" },
  bedrock: {
    url: process.env.BEDROCK_OPENAI_GATEWAY_URL ?? "",
    apiKeyEnv: "BEDROCK_API_KEY",
  },
};

/** Default OpenAI-compatible forward. Reads the provider API key from the
 *  environment and POSTs the chat-completions request. Used in production; the
 *  unit tier injects a stub `ForwardFn` instead. Anthropic uses its `messages`
 *  wire shape; openai/bedrock use the chat-completions shape. */
export const defaultForward: ForwardFn = async (req) => {
  const endpoint = PROVIDER_ENDPOINTS[req.provider];
  const apiKey = (process.env[endpoint.apiKeyEnv] ?? "").trim();
  if (apiKey.length === 0) {
    throw new ProviderForwardError(
      `spendguard:botpress: ${endpoint.apiKeyEnv} is not set; cannot forward to ${req.provider}`,
    );
  }
  if (endpoint.url.length === 0) {
    throw new ProviderForwardError(
      `spendguard:botpress: no endpoint configured for provider ${req.provider}`,
    );
  }
  if (req.provider === "anthropic") {
    return forwardAnthropic(req, endpoint.url, apiKey);
  }
  return forwardOpenAiCompatible(req, endpoint.url, apiKey);
};

interface OpenAiChatResponse {
  id?: string;
  model?: string;
  choices?: Array<{
    message?: { content?: string | null };
    finish_reason?: string | null;
  }>;
  usage?: { prompt_tokens?: number; completion_tokens?: number };
}

async function forwardOpenAiCompatible(
  req: ForwardRequest,
  url: string,
  apiKey: string,
): Promise<ForwardResult> {
  const messages = req.systemPrompt
    ? [{ role: "system", content: req.systemPrompt }, ...req.messages]
    : [...req.messages];
  const body = {
    model: req.model,
    messages,
    max_tokens: req.maxTokens,
    ...(req.temperature !== undefined ? { temperature: req.temperature } : {}),
    ...(req.topP !== undefined ? { top_p: req.topP } : {}),
    ...(req.stopSequences !== undefined ? { stop: req.stopSequences } : {}),
    ...(req.userId !== undefined ? { user: req.userId } : {}),
  };
  const resp = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${apiKey}` },
    body: JSON.stringify(body),
  });
  if (!resp.ok) {
    throw new ProviderForwardError(`upstream ${req.provider} returned HTTP ${resp.status}`);
  }
  const json = (await resp.json()) as OpenAiChatResponse;
  const choice = json.choices?.[0];
  return {
    id: json.id ?? "",
    model: json.model ?? req.model,
    content: choice?.message?.content ?? "",
    stopReason: mapStopReason(choice?.finish_reason),
    inputTokens: json.usage?.prompt_tokens ?? 0,
    outputTokens: json.usage?.completion_tokens ?? 0,
  };
}

interface AnthropicResponse {
  id?: string;
  model?: string;
  content?: Array<{ type?: string; text?: string }>;
  stop_reason?: string | null;
  usage?: { input_tokens?: number; output_tokens?: number };
}

async function forwardAnthropic(
  req: ForwardRequest,
  url: string,
  apiKey: string,
): Promise<ForwardResult> {
  const body = {
    model: req.model,
    max_tokens: req.maxTokens,
    ...(req.systemPrompt !== undefined ? { system: req.systemPrompt } : {}),
    ...(req.temperature !== undefined ? { temperature: req.temperature } : {}),
    ...(req.topP !== undefined ? { top_p: req.topP } : {}),
    ...(req.stopSequences !== undefined ? { stop_sequences: req.stopSequences } : {}),
    messages: req.messages.map((m) => ({ role: m.role, content: m.content })),
  };
  const resp = await fetch(url, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-api-key": apiKey,
      "anthropic-version": "2023-06-01",
    },
    body: JSON.stringify(body),
  });
  if (!resp.ok) {
    throw new ProviderForwardError(`upstream anthropic returned HTTP ${resp.status}`);
  }
  const json = (await resp.json()) as AnthropicResponse;
  const text = (json.content ?? [])
    .filter((b) => b.type === "text")
    .map((b) => b.text ?? "")
    .join("");
  return {
    id: json.id ?? "",
    model: json.model ?? req.model,
    content: text,
    stopReason: mapStopReason(json.stop_reason),
    inputTokens: json.usage?.input_tokens ?? 0,
    outputTokens: json.usage?.output_tokens ?? 0,
  };
}

function mapStopReason(
  raw: string | null | undefined,
): "stop" | "max_tokens" | "content_filter" | "other" {
  switch (raw) {
    case "stop":
    case "end_turn":
    case "stop_sequence":
      return "stop";
    case "length":
    case "max_tokens":
      return "max_tokens";
    case "content_filter":
      return "content_filter";
    default:
      return "other";
  }
}
