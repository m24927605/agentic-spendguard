// Shared fixtures for the @spendguard/botpress-integration unit suite.
//
// Provides a canonical operator configuration + a minimal Botpress hook
// input the test cases can spread over to create variants.

import type { BotpressHookInput } from "../src/adapter/binding.js";
import type { Configuration } from "../src/config.js";

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

export function makeHookInput(
  overrides: {
    ctx?: Partial<BotpressHookInput["ctx"]>;
    data?: Partial<BotpressHookInput["data"]>;
  } = {},
): BotpressHookInput {
  return {
    ctx: { botId: "bot-test-1", ...overrides.ctx },
    data: {
      conversationId: "conv-test-1",
      userId: "user-test-1",
      model: "gpt-4o-mini",
      maxTokens: 100,
      input: {
        messages: [{ role: "user", content: "hello" }],
      },
      ...overrides.data,
    },
  };
}
