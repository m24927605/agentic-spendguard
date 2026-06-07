// @spendguard/botpress-integration — public barrel.
//
// Default export: a Botpress `Integration` instance wired with the
// SpendGuard hooks + lifecycle. The Botpress runtime imports the default
// export from this module via `botpress integrations push`'s build artefact.
//
// Named exports surface the typed helpers + error classes consumers may
// want to introspect (for testing, decoration, or custom error handling).
//
// Locked decisions (design.md §5 + review-standards.md §2):
//   - BOTH `beforeAiGeneration` AND `afterAiGeneration` are registered.
//   - `register` is wired to `validateConfiguration` — the install-time
//     reserve+release probe (INV-4).
//   - The Zod schema is the source of truth for operator-form validation.

import { Integration } from "@botpress/sdk";
import { type Configuration, ConfigurationSchema } from "./config.js";
import { runAfterAiGeneration } from "./hooks/afterAiGeneration.js";
import { type SpendGuardHandleStash, runBeforeAiGeneration } from "./hooks/beforeAiGeneration.js";
import { validateConfiguration } from "./lifecycle/validateConfiguration.js";

// ── Public surface — named exports ────────────────────────────────────

export { VERSION } from "./version.js";
export { ConfigurationSchema, SpendGuardConfigError } from "./config.js";
export type { Configuration } from "./config.js";
export {
  SpendGuardReservation,
  DecisionDenied,
  SidecarUnavailable,
} from "./reservation.js";
export type {
  BotpressCallContext,
  ReservationHandle,
} from "./reservation.js";
export {
  toBindingFromHookInput,
  pickTenantId,
} from "./adapter/binding.js";
export type { BotpressHookInput } from "./adapter/binding.js";
export {
  extractUsageFromBotpressEvent,
  pickProviderEventId,
  snapshotToUsage,
} from "./adapter/usage.js";
export { toRuntimeError, codeFor } from "./adapter/errors.js";
export {
  runBeforeAiGeneration,
  type SpendGuardHandleStash,
} from "./hooks/beforeAiGeneration.js";
export { runAfterAiGeneration } from "./hooks/afterAiGeneration.js";
export { validateConfiguration } from "./lifecycle/validateConfiguration.js";

// ── Default export — the Botpress Integration instance ────────────────
//
// review-standards.md §2.3 / §2.4: both hooks AND validateConfiguration
// MUST be wired by this constructor. The Botpress runtime treats the
// default export of this module as the integration registration; any
// hook or lifecycle function omitted here is invisible to Botpress.

export default new Integration<Configuration>({
  configuration: { schema: ConfigurationSchema },
  register: async ({ configuration }: { configuration: Configuration }) => {
    await validateConfiguration({ configuration });
  },
  unregister: async () => {
    // No durable state to tear down — the SpendGuardReservation lifecycle
    // is per-call. Future: emit a final unregister audit row.
  },
  channels: {},
  actions: {},
  hooks: {
    beforeAiGeneration: async ({
      ctx,
      data,
      configuration,
    }: {
      ctx: { botId: string };
      data: Record<string, unknown>;
      configuration: Configuration;
    }) => {
      const out = await runBeforeAiGeneration({
        input: {
          ctx,
          data: data as Parameters<typeof runBeforeAiGeneration>[0]["input"]["data"],
        },
        configuration,
      });
      return { data: out.data as Record<string, unknown> };
    },
    afterAiGeneration: async ({
      ctx,
      data,
      configuration,
    }: {
      ctx: { botId: string };
      data: Record<string, unknown>;
      configuration: Configuration;
    }) => {
      const out = await runAfterAiGeneration({
        input: {
          ctx,
          data: data as Parameters<typeof runAfterAiGeneration>[0]["input"]["data"] &
            SpendGuardHandleStash,
        },
        configuration,
      });
      return { data: out.data as Record<string, unknown> };
    },
  },
});
