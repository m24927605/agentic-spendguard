// Shared LLM action schemas for the SpendGuard Botpress integration.
//
// These mirror the public Botpress `llm` interface contract
// (interfaces/llm/interface.definition.ts -> @botpress/common llm schemas)
// closely enough to be a drop-in LLM-provider surface, but are declared
// inline here because `@botpress/common` is an internal Botpress workspace
// package that is NOT published to the public npm registry, and `bp add llm`
// (which would fetch the resolved interface) requires Botpress Cloud auth.
//
// Declaring the `generateContent` / `listLanguageModels` actions natively on
// the IntegrationDefinition (rather than via `.extend(llm)`) keeps the build
// fully offline + auth-free while still producing a correct, SDK-compiled
// LLM-provider integration whose actions Botpress can invoke for LLM spend.
//
// The field shapes are the load-bearing subset Botpress's LLM router speaks:
//   - generateContent input:  { model, messages[], systemPrompt?, maxTokens?,
//                               temperature?, topP?, stopSequences?,
//                               userId?, responseFormat? }
//   - generateContent output: { id, provider, model, choices[], usage,
//                               botpress: { cost } }
//   - listLanguageModels output: { models: ModelRef[] }
//
// `z` is the Botpress-flavoured zui re-export from `@botpress/sdk` (see the
// defensive import note in src/config.ts).
import { z } from "@botpress/sdk";

/** A reference to one upstream model the integration exposes. Mirrors the
 *  llm interface `modelRef` entity (id + human-facing name). */
export const ModelRefSchema = z.object({
  id: z.string().describe("Provider-qualified model id, e.g. openai:gpt-4o-mini"),
  name: z.string().describe("Human-facing model name"),
});
export type ModelRef = z.infer<typeof ModelRefSchema>;

/** One message in the prompt. Botpress normalises tool/assistant/user roles
 *  into this shape before invoking generateContent. `content` is the text
 *  the SpendGuard prompt-hash is computed over. */
export const MessageSchema = z.object({
  role: z.enum(["system", "user", "assistant", "tool"]).describe("Message role"),
  content: z.string().describe("Message text content"),
});
export type Message = z.infer<typeof MessageSchema>;

/** Per-call token usage echoed back from the upstream provider. */
export const UsageSchema = z.object({
  inputTokens: z.number().describe("Prompt / input token count"),
  outputTokens: z.number().describe("Completion / output token count"),
});
export type Usage = z.infer<typeof UsageSchema>;

/** One generated choice. */
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

export const GenerateContentInputSchema = z.object({
  model: ModelRefSchema.optional().describe("Model to use; defaults to the first listed model"),
  messages: z.array(MessageSchema).describe("Prompt messages"),
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

export const ListLanguageModelsInputSchema = z.object({});
export type ListLanguageModelsInput = z.infer<typeof ListLanguageModelsInputSchema>;

export const ListLanguageModelsOutputSchema = z.object({
  models: z.array(ModelRefSchema).describe("Models this integration can route to"),
});
export type ListLanguageModelsOutput = z.infer<typeof ListLanguageModelsOutputSchema>;
