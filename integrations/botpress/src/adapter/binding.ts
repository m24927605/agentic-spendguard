// Botpress hook input → SpendGuard `BotpressCallContext` adapter.
//
// review-standards.md §3.1 / §3.3 / AD01 / AD02 — derive
// `botId` / `conversationId` / `userId` / `model` / `messages` / `maxTokens`
// from the Botpress hook payload and override `tenantId` from the
// configuration when explicitly set (Slice 2 design.md §5 conversation
// mapping).
//
// We type-shape the Botpress hook input loosely so a future Botpress 0.7.x
// patch can add fields without invalidating our binding. The runtime
// signature is documented in `@botpress/sdk@^0.7`'s Integration type's
// `hooks.beforeAiGeneration` / `hooks.afterAiGeneration` arguments.

import type { Configuration } from "../config.js";
import type { BotpressCallContext } from "../reservation.js";

/**
 * Minimal Botpress hook input shape the adapter depends on.
 *
 * The real Botpress hook input is `{ ctx, client, data, configuration }`.
 * We only pull from `ctx` (botId) and `data` (everything else). The
 * configuration is threaded separately because it has already been Zod-
 * validated by Botpress before the hook fires.
 */
export interface BotpressHookInput {
  ctx: {
    readonly botId: string;
  };
  data: {
    readonly conversationId?: string;
    readonly userId?: string;
    readonly model?: string;
    readonly maxTokens?: number;
    readonly input?: {
      readonly messages?: ReadonlyArray<{ role?: string; content?: string }>;
    };
    /** Sometimes the messages live at the top of `data` rather than
     *  under `data.input`. Both shapes are observed across Botpress 0.7.x
     *  patch versions. The binding code prefers `data.input.messages` and
     *  falls back to `data.messages`. */
    readonly messages?: ReadonlyArray<{ role?: string; content?: string }>;
  };
}

/**
 * Build a `BotpressCallContext` from the Botpress hook input + the
 * operator-supplied configuration. The tenant id falls back to the bot id
 * when `configuration.tenantId` is empty (review-standards.md §3 AD01),
 * but Zod's `min(1)` on `tenantId` means the empty-string path is only
 * reachable via direct (test-only) construction.
 */
export function toBindingFromHookInput(args: {
  readonly input: BotpressHookInput;
  readonly configuration: Configuration;
}): BotpressCallContext {
  const { input, configuration } = args;
  const data = input.data;
  const messagesRaw = data.input?.messages ?? data.messages ?? [];
  const messages: ReadonlyArray<{ role: string; content: string }> = messagesRaw.map((m) => ({
    role: m.role ?? "user",
    content: m.content ?? "",
  }));
  return {
    botId: input.ctx.botId,
    conversationId: data.conversationId ?? `conv-${input.ctx.botId}`,
    userId: data.userId ?? "anonymous",
    model: data.model ?? "unknown",
    messages,
    maxTokens: data.maxTokens ?? 1024,
  };
}

/**
 * Convenience function — pick the binding tenant id. Promoted to a named
 * helper because AD01 / AD02 test it as a unit and the precedence rule is
 * load-bearing. (Configuration `tenantId` is Zod-validated as non-empty in
 * production, but the binding layer remains tolerant of synthetic test
 * configurations.)
 */
export function pickTenantId(configuration: Configuration, botId: string): string {
  return configuration.tenantId.length > 0 ? configuration.tenantId : botId;
}
