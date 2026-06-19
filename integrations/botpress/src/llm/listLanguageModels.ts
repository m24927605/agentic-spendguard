// `listLanguageModels` action implementation.
//
// Botpress calls this to populate the model picker. SpendGuard exposes the
// models for the configured upstream provider — it is a budget gate in front
// of the provider, not a model catalog, so the v1 list is the small set of
// default models per provider. The operator can still pass any model id to
// `generateContent`; this list only seeds the UI.
//
// The row shape matches the `llm` interface model schema (`modelRef`
// intersected with model metadata): `id`, `name`, `description`, `tags`,
// `input` { maxTokens, costPer1MTokens }, `output` { maxTokens,
// costPer1MTokens }. The pricing / context figures below are advisory display
// values for the picker — the SpendGuard sidecar remains the source of truth
// for the ledgered budget; Botpress billing is advisory.

import type { Configuration } from "../config.js";
import type { LanguageModel, ListLanguageModelsOutput } from "./schemas.js";

const MODELS_BY_PROVIDER: Record<
  Configuration["upstreamProvider"],
  ReadonlyArray<LanguageModel>
> = {
  openai: [
    {
      id: "gpt-4o",
      name: "OpenAI GPT-4o",
      description: "OpenAI flagship multimodal model.",
      tags: ["recommended", "general-purpose", "vision", "function-calling", "agents"],
      input: { maxTokens: 128_000, costPer1MTokens: 2.5 },
      output: { maxTokens: 16_384, costPer1MTokens: 10 },
    },
    {
      id: "gpt-4o-mini",
      name: "OpenAI GPT-4o mini",
      description: "Smaller, low-cost OpenAI multimodal model.",
      tags: ["low-cost", "general-purpose", "vision", "function-calling"],
      input: { maxTokens: 128_000, costPer1MTokens: 0.15 },
      output: { maxTokens: 16_384, costPer1MTokens: 0.6 },
    },
  ],
  anthropic: [
    {
      id: "claude-3-5-sonnet-latest",
      name: "Anthropic Claude 3.5 Sonnet",
      description: "Anthropic balanced model for general-purpose agent work.",
      tags: ["recommended", "general-purpose", "vision", "coding", "agents"],
      input: { maxTokens: 200_000, costPer1MTokens: 3 },
      output: { maxTokens: 8_192, costPer1MTokens: 15 },
    },
    {
      id: "claude-3-5-haiku-latest",
      name: "Anthropic Claude 3.5 Haiku",
      description: "Fast, low-cost Anthropic model.",
      tags: ["low-cost", "general-purpose", "function-calling"],
      input: { maxTokens: 200_000, costPer1MTokens: 0.8 },
      output: { maxTokens: 8_192, costPer1MTokens: 4 },
    },
  ],
  bedrock: [
    {
      id: "anthropic.claude-3-5-sonnet",
      name: "Bedrock Claude 3.5 Sonnet",
      description: "Anthropic Claude 3.5 Sonnet served via Amazon Bedrock.",
      tags: ["recommended", "general-purpose", "vision", "coding", "agents"],
      input: { maxTokens: 200_000, costPer1MTokens: 3 },
      output: { maxTokens: 8_192, costPer1MTokens: 15 },
    },
    {
      id: "anthropic.claude-3-5-haiku",
      name: "Bedrock Claude 3.5 Haiku",
      description: "Fast, low-cost Anthropic model served via Amazon Bedrock.",
      tags: ["low-cost", "general-purpose", "function-calling"],
      input: { maxTokens: 200_000, costPer1MTokens: 0.8 },
      output: { maxTokens: 8_192, costPer1MTokens: 4 },
    },
  ],
};

/** Return the models SpendGuard can route to for the configured provider. */
export function runListLanguageModels(configuration: Configuration): ListLanguageModelsOutput {
  return { models: [...MODELS_BY_PROVIDER[configuration.upstreamProvider]] };
}
