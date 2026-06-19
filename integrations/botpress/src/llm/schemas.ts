// Internal LLM action shapes for the SpendGuard Botpress integration.
//
// The integration now adopts the FORMAL Botpress `llm` interface via
// `.extend(llm, ...)` in `integration.definition.ts`. The authoritative
// `generateContent` / `listLanguageModels` input + output schemas therefore
// live in the vendored interface package (`bp_modules/llm/definition/...`) and
// the generated `.botpress` types (`import * as bp from '.botpress'`) are the
// contract the action handlers in `src/index.ts` must satisfy.
//
// This module keeps a SIMPLIFIED internal representation that the SpendGuard
// pipeline (`adapter/binding.ts`, `provider/forward.ts`, `reservation.ts`,
// `llm/generateContent.ts`) speaks. The action boundary in `src/index.ts`
// maps the rich interface input DOWN to this internal `GenerateContentInput`
// (flattening multipart/tool content to plain text, which is all the
// prompt-hash + token estimate need) and maps the internal
// `GenerateContentOutput` UP to the interface output (filling in the
// interface's `usage.inputCost` / `usage.outputCost` cost fields).
//
// Only `LanguageModelIdSchema` / `ModelRefSchema` are consumed by
// `integration.definition.ts` (to build the concrete `modelRef` entity the
// `llm` interface references via `z.ref("modelRef")`). The remaining shapes
// are internal and back the unit suite.
//
// `z` is the Botpress-flavoured zui re-export from `@botpress/sdk` (see the
// defensive import note in src/config.ts).
import { z } from "@botpress/sdk";

/** The `id` field of the `llm` interface `modelRef` entity. The interface
 *  declares `modelRef = z.object({ id: <string> }).catchall(z.never())` and
 *  references it from both action schemas via `z.ref("modelRef")`. We supply
 *  the concrete `id` schema here so `integration.definition.ts` can build the
 *  matching `modelRef` entity (mirroring the first-party OpenAI integration's
 *  `entities.modelRef.schema = z.object({ id: <languageModelId> })`). */
export const LanguageModelIdSchema = z
  .string()
  .title("LLM Model ID")
  .describe("Provider-qualified model id, e.g. gpt-4o-mini");

/** A reference to one upstream model the integration exposes — the runtime
 *  projection of the `llm` interface `modelRef` entity (`{ id }`). Note the
 *  interface `modelRef` carries ONLY `id` (no `name`); the human-facing name
 *  lives on the `listLanguageModels` model rows, not on the ref. */
export const ModelRefSchema = z.object({
  id: LanguageModelIdSchema,
});
export type ModelRef = z.infer<typeof ModelRefSchema>;

/** One message in the internal prompt representation. The `llm` interface
 *  restricts message roles to `user | assistant`, but the SpendGuard prompt
 *  hash + token estimate also fold in the `systemPrompt` (mapped to a
 *  synthetic `system` message by the binding), so the internal role set is
 *  wider. `content` is the flattened text the prompt-hash is computed over. */
export const MessageSchema = z.object({
  role: z.enum(["system", "user", "assistant", "tool"]).describe("Message role"),
  content: z.string().describe("Flattened message text content"),
});
export type Message = z.infer<typeof MessageSchema>;

/** Per-call token usage echoed back from the upstream provider (internal). The
 *  interface output additionally carries `inputCost` / `outputCost`; those are
 *  filled in by the action boundary in `src/index.ts`. */
export const UsageSchema = z.object({
  inputTokens: z.number().describe("Prompt / input token count"),
  outputTokens: z.number().describe("Completion / output token count"),
});
export type Usage = z.infer<typeof UsageSchema>;

/** One generated choice (internal). SpendGuard emits a single text choice; the
 *  interface output `stopReason` additionally allows `tool_calls`, which this
 *  text-only surface never produces. */
export const ChoiceSchema = z.object({
  role: z.literal("assistant").describe("Always assistant for generated content"),
  type: z.literal("text").describe("Content type — text only in v1"),
  content: z.string().describe("Generated text"),
  index: z.number().describe("Choice index"),
  stopReason: z
    .enum(["stop", "max_tokens", "content_filter", "other"])
    .describe("Why generation stopped"),
});
export type Choice = z.infer<typeof ChoiceSchema>;

/** Internal `generateContent` input — the simplified projection the SpendGuard
 *  pipeline consumes. `src/index.ts` builds this from the interface's richer
 *  action input. */
export const GenerateContentInputSchema = z.object({
  model: ModelRefSchema.optional().describe("Model to use; defaults to the provider default"),
  messages: z.array(MessageSchema).describe("Prompt messages (content flattened to text)"),
  systemPrompt: z.string().optional().describe("Optional system prompt"),
  maxTokens: z
    .number()
    .optional()
    .describe("Operator-declared output cap; drives the SpendGuard reserve estimate"),
  temperature: z.number().optional().describe("Sampling temperature"),
  topP: z.number().optional().describe("Nucleus sampling cutoff"),
  stopSequences: z.array(z.string()).optional().describe("Stop sequences"),
  userId: z.string().optional().describe("Opaque end-user id forwarded upstream"),
});
export type GenerateContentInput = z.infer<typeof GenerateContentInputSchema>;

/** Internal `generateContent` output. Structurally a subset of the interface
 *  output (single text choice, no cost fields); `src/index.ts` widens it to
 *  the interface output by adding `usage.inputCost` / `usage.outputCost`. */
export const GenerateContentOutputSchema = z.object({
  id: z.string().describe("Provider response id"),
  provider: z.string().describe("Upstream provider that served the call"),
  model: z.string().describe("Model id that served the call"),
  choices: z.array(ChoiceSchema).describe("Generated choices"),
  usage: UsageSchema.describe("Real token usage committed to SpendGuard"),
  botpress: z
    .object({ cost: z.number().describe("Cost in USD as reported to Botpress billing") })
    .describe("Botpress billing envelope"),
});
export type GenerateContentOutput = z.infer<typeof GenerateContentOutputSchema>;

/** One row of the `listLanguageModels` catalog — matches the `llm` interface
 *  model shape (`modelRef` intersected with the model metadata Botpress Studio
 *  renders in the model picker). */
export const LanguageModelSchema = z.object({
  id: LanguageModelIdSchema,
  name: z.string().describe("Human-facing model name"),
  description: z.string().describe("Short model description"),
  tags: z
    .array(
      z.enum([
        "recommended",
        "deprecated",
        "general-purpose",
        "low-cost",
        "vision",
        "coding",
        "agents",
        "function-calling",
        "roleplay",
        "storytelling",
        "reasoning",
        "preview",
        "speech-to-text",
        "image-generation",
        "text-to-speech",
      ]),
    )
    .describe("Capability / lifecycle tags rendered in the model picker"),
  input: z
    .object({
      maxTokens: z.number().describe("Max input tokens"),
      costPer1MTokens: z.number().describe("Input cost per 1M tokens, USD"),
    })
    .describe("Input limits + pricing"),
  output: z
    .object({
      maxTokens: z.number().describe("Max output tokens"),
      costPer1MTokens: z.number().describe("Output cost per 1M tokens, USD"),
    })
    .describe("Output limits + pricing"),
});
export type LanguageModel = z.infer<typeof LanguageModelSchema>;

export const ListLanguageModelsInputSchema = z.object({});
export type ListLanguageModelsInput = z.infer<typeof ListLanguageModelsInputSchema>;

export const ListLanguageModelsOutputSchema = z.object({
  models: z.array(LanguageModelSchema).describe("Models this integration can route to"),
});
export type ListLanguageModelsOutput = z.infer<typeof ListLanguageModelsOutputSchema>;
