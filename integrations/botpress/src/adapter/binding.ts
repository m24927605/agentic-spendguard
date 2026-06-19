// `generateContent` action input -> SpendGuard `BotpressCallContext` adapter.
//
// Derives the SpendGuard call context from the Botpress `generateContent`
// action payload (`input` + `ctx`). The Botpress LLM router calls this action
// with the prompt messages, the chosen model, and an operator-declared
// `maxTokens` cap; SpendGuard reserves against that cap and commits real usage
// after the upstream call returns.
//
// `ctx` is the Botpress integration handler context — `ctx.botId` /
// `ctx.integrationId` identify the install. We map `botId` onto the SpendGuard
// `conversationId`/`botId` correlation fields since the action payload does not
// carry a Botpress conversation id (LLM calls are conversation-agnostic at the
// provider boundary).

import type { Configuration } from "../config.js";
import type { GenerateContentInput, ModelRef } from "../llm/schemas.js";
import type { BotpressCallContext } from "../reservation.js";

/** Minimal Botpress integration handler context the binding depends on. */
export interface BotpressActionCtx {
  readonly botId: string;
  readonly integrationId?: string;
}

/** Default per-provider model id used when the action input omits `model`. */
const DEFAULT_MODEL: Record<Configuration["upstreamProvider"], string> = {
  openai: "gpt-4o-mini",
  anthropic: "claude-3-5-haiku-latest",
  bedrock: "anthropic.claude-3-5-haiku",
};

/** Default reserve cap when the action input omits `maxTokens`. Matches the
 *  legacy estimator floor used elsewhere in SpendGuard. */
export const DEFAULT_MAX_TOKENS = 1024;

/** Resolve the model id to forward + reserve under: explicit input model id,
 *  else the provider default. */
export function resolveModel(input: GenerateContentInput, configuration: Configuration): string {
  const explicit: ModelRef | undefined = input.model;
  if (explicit !== undefined && explicit.id.length > 0) {
    return explicit.id;
  }
  return DEFAULT_MODEL[configuration.upstreamProvider];
}

/** Resolve the output-token cap that drives both the SpendGuard reserve
 *  estimate and the upstream `max_tokens` field. */
export function resolveMaxTokens(input: GenerateContentInput): number {
  return input.maxTokens !== undefined && input.maxTokens > 0
    ? input.maxTokens
    : DEFAULT_MAX_TOKENS;
}

/**
 * Build a `BotpressCallContext` from the `generateContent` action input + the
 * operator-supplied configuration + the handler `ctx`. The system prompt, when
 * present, is prepended to the message list so the SpendGuard prompt-hash and
 * token estimate cover it.
 */
export function toBindingFromActionInput(args: {
  readonly input: GenerateContentInput;
  readonly configuration: Configuration;
  readonly ctx: BotpressActionCtx;
}): BotpressCallContext {
  const { input, configuration, ctx } = args;
  const systemMessages =
    input.systemPrompt !== undefined && input.systemPrompt.length > 0
      ? [{ role: "system", content: input.systemPrompt }]
      : [];
  const messages: ReadonlyArray<{ role: string; content: string }> = [
    ...systemMessages,
    ...input.messages.map((m) => ({ role: m.role, content: m.content })),
  ];
  return {
    botId: ctx.botId,
    conversationId: `bot-${ctx.botId}`,
    userId: input.userId ?? "anonymous",
    model: resolveModel(input, configuration),
    messages,
    maxTokens: resolveMaxTokens(input),
  };
}

/**
 * Pick the binding tenant id. Configuration `tenantId` is Zod-validated as
 * non-empty in production; the binding stays tolerant of synthetic test
 * configurations and falls back to the bot id.
 */
export function pickTenantId(configuration: Configuration, botId: string): string {
  return configuration.tenantId.length > 0 ? configuration.tenantId : botId;
}
