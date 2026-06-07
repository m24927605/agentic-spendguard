// Zod configuration schema for @spendguard/botpress-integration.
//
// review-standards.md §2.1 / §2.2 LOCKED — fields and upstreamProvider enum.
// Botpress's `new Integration({ configuration: { schema } })` wires this Zod
// schema as the source of truth for register-time validation: the operator
// fills the form, Botpress parses against this schema, and only well-formed
// configurations reach `validateConfiguration`.
//
// Defensive note on the Zod import: Botpress 0.7 re-exports zod as a named
// export from `@botpress/sdk`. Importing from `@botpress/sdk` (instead of
// pulling a second copy from `zod`) guarantees the Zod instance the Botpress
// runtime uses to validate the form matches the instance we used to declare
// the schema — without this, an `instanceof ZodObject` check on the
// Botpress side would silently fail and the form would refuse to save.
//
// LOCKED upstreamProvider enum (review-standards.md §2.2):
//   - `openai` / `anthropic` / `bedrock` only.
//   - NO `cohere`, NO `gemini` in v1 — D32 design.md §3 anti-scope.
import { z } from "@botpress/sdk";

/**
 * LOCKED v1 configuration schema. Adding a field is a v0.2 minor; removing
 * one is a v1.0 major.
 *
 * design.md §5 + review-standards.md §2.1:
 *   - `sidecarUrl`               — D09 HTTP companion endpoint URL.
 *   - `spendguardBudgetId`       — UUID of the SpendGuard budget to charge.
 *   - `spendguardWindowInstanceId` — UUID of the SpendGuard window instance.
 *   - `upstreamProvider`         — enum LOCKED at `openai | anthropic | bedrock`.
 *   - `tenantId`                 — operator tenant identifier (overrides
 *                                  the per-bot default; see §5 conversation
 *                                  mapping).
 *   - `tlsCertPath` / `tlsKeyPath` / `tlsRootCaPath` — optional mTLS
 *                                  material paths. Resolved at runtime by
 *                                  `src/reservation.ts` (see §3.3 D09 mTLS
 *                                  contract).
 */
export const ConfigurationSchema = z.object({
  sidecarUrl: z.string().url().describe("HTTP companion URL (loopback or sidecar-pod port)"),
  spendguardBudgetId: z.string().min(1).describe("UUID of the SpendGuard budget to charge"),
  spendguardWindowInstanceId: z.string().min(1).describe("UUID of the SpendGuard window instance"),
  upstreamProvider: z
    .enum(["openai", "anthropic", "bedrock"])
    .describe("Upstream provider Botpress dispatches to"),
  tenantId: z.string().min(1).describe("Operator tenant identifier"),
  tlsCertPath: z.string().optional().describe("Path to SVID cert PEM"),
  tlsKeyPath: z.string().optional().describe("Path to SVID key PEM"),
  tlsRootCaPath: z.string().optional().describe("Path to sidecar CA PEM"),
});

export type Configuration = z.infer<typeof ConfigurationSchema>;

/**
 * Lightweight runtime check used by `SpendGuardReservation` constructor —
 * complements the Zod parse Botpress runs on the operator form. Catches the
 * "constructed from synthetic test object" path where Botpress's parse never
 * ran and the SpendGuardReservation receives a half-built object.
 *
 * review-standards.md §2.6 — missing required fields raise a
 * `SpendGuardConfigError` naming the offending field (see
 * `src/adapter/errors.ts`).
 */
export function assertRequiredConfig(cfg: Partial<Configuration>): asserts cfg is Configuration {
  const missing: string[] = [];
  if (!cfg.sidecarUrl) missing.push("sidecarUrl");
  if (!cfg.spendguardBudgetId) missing.push("spendguardBudgetId");
  if (!cfg.spendguardWindowInstanceId) missing.push("spendguardWindowInstanceId");
  if (!cfg.upstreamProvider) missing.push("upstreamProvider");
  if (!cfg.tenantId) missing.push("tenantId");
  if (missing.length > 0) {
    throw new SpendGuardConfigError(
      `spendguard:botpress: missing required configuration field(s): ${missing.join(", ")}`,
    );
  }
}

/**
 * Local config error — kept local rather than re-exported from
 * `@spendguard/sdk` to avoid an unnecessary peer-dep surface widening for a
 * pure-validation concern.
 *
 * The error's `code` mirrors the Botpress `RuntimeError` code that
 * `src/adapter/errors.ts` will translate it to (`BUDGET_CONFIG`), so a
 * defensive consumer catching this directly sees the same code in both
 * directions.
 */
export class SpendGuardConfigError extends Error {
  readonly code = "BUDGET_CONFIG" as const;
  constructor(message: string) {
    super(message);
    this.name = "SpendGuardConfigError";
  }
}
