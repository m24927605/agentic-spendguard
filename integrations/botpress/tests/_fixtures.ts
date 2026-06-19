// Shared fixtures for the @spendguard/botpress-integration unit suite.
//
// Provides a canonical operator configuration + a minimal `generateContent`
// action input + handler ctx the test cases spread over to create variants.

import type { BotpressActionCtx } from "../src/adapter/binding.js";
import type { Configuration } from "../src/config.js";
import type { GenerateContentInput } from "../src/llm/schemas.js";

export const FIXTURE_TENANT_ID = "00000000-0000-4000-8000-000000000001";
export const FIXTURE_BUDGET_ID = "44444444-4444-4444-8444-444444444444";
export const FIXTURE_WINDOW_INSTANCE_ID = "55555555-5555-4555-8555-555555555555";

export function makeConfig(overrides: Partial<Configuration> = {}): Configuration {
  return {
    sidecarUrl: "http://127.0.0.1:0",
    spendguardBudgetId: FIXTURE_BUDGET_ID,
    spendguardWindowInstanceId: FIXTURE_WINDOW_INSTANCE_ID,
    upstreamProvider: "openai",
    tenantId: FIXTURE_TENANT_ID,
    ...overrides,
  };
}

export function makeCtx(overrides: Partial<BotpressActionCtx> = {}): BotpressActionCtx {
  return { botId: "bot-test-1", integrationId: "int-test-1", ...overrides };
}

export function makeGenerateContentInput(
  overrides: Partial<GenerateContentInput> = {},
): GenerateContentInput {
  return {
    model: { id: "gpt-4o-mini", name: "OpenAI GPT-4o mini" },
    messages: [{ role: "user", content: "hello" }],
    maxTokens: 100,
    ...overrides,
  };
}
