// `listLanguageModels` action implementation.
//
// Botpress calls this to populate the model picker. SpendGuard exposes the
// models for the configured upstream provider — it is a budget gate in front
// of the provider, not a model catalog, so the v1 list is the small set of
// default models per provider. The operator can still pass any model id to
// `generateContent`; this list only seeds the UI.

import type { Configuration } from "../config.js";
import type { ListLanguageModelsOutput, ModelRef } from "./schemas.js";

const MODELS_BY_PROVIDER: Record<Configuration["upstreamProvider"], ReadonlyArray<ModelRef>> = {
  openai: [
    { id: "gpt-4o", name: "OpenAI GPT-4o" },
    { id: "gpt-4o-mini", name: "OpenAI GPT-4o mini" },
  ],
  anthropic: [
    { id: "claude-3-5-sonnet-latest", name: "Anthropic Claude 3.5 Sonnet" },
    { id: "claude-3-5-haiku-latest", name: "Anthropic Claude 3.5 Haiku" },
  ],
  bedrock: [
    { id: "anthropic.claude-3-5-sonnet", name: "Bedrock Claude 3.5 Sonnet" },
    { id: "anthropic.claude-3-5-haiku", name: "Bedrock Claude 3.5 Haiku" },
  ],
};

/** Return the models SpendGuard can route to for the configured provider. */
export function runListLanguageModels(configuration: Configuration): ListLanguageModelsOutput {
  return { models: [...MODELS_BY_PROVIDER[configuration.upstreamProvider]] };
}
