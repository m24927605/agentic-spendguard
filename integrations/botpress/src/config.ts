// Configuration schema for @spendguard/botpress-integration.
//
// Fields + upstreamProvider enum (openai | anthropic | bedrock). The
// `IntegrationDefinition` in integration.definition.ts wires
// `ConfigurationObjectSchema` (the un-refined base, below) as the install-time
// form schema: the operator fills the form, Botpress parses against it, and
// only well-formed configurations reach `register` -> `validateConfiguration`.
// The cross-field refinements live on `ConfigurationSchema` and are
// re-enforced fail-closed at runtime by `assertRequiredConfig`.
//
// Defensive note on the schema import: `@botpress/sdk` re-exports its zui
// `z` (a zod-compatible builder). Importing `z` from `@botpress/sdk` (instead
// of pulling a separate copy from the `zod` package) guarantees the schema
// instance the Botpress runtime validates against matches the instance we used
// to declare the schema — otherwise a cross-instance `instanceof` check on the
// Botpress side would fail and the definition would refuse to build.
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
/**
 * Base object schema (no cross-field refinements). Botpress's
 * `IntegrationDefinition` config + action schemas must be a plain
 * `ZodObject` / `ZodRecord` (`z.ZuiObjectSchema`); a `.refine()` produces a
 * `ZodEffects`, which the definition's `SchemaDefinition` constraint rejects.
 *
 * `integration.definition.ts` consumes THIS schema so `bp build` codegen sees
 * a well-typed config object. The cross-field invariants (transport-security +
 * all-or-none mTLS) are re-enforced fail-closed at runtime by
 * `assertRequiredConfig` inside `SpendGuardReservation`'s constructor, so the
 * deeper checks are never skipped on the live path.
 */
export const ConfigurationObjectSchema = z.object({
  sidecarUrl: z
    .string()
    .url()
    // The https-except-loopback rule is intentionally NOT a `.refine()` here:
    // a field-level refine is a ZodEffects, which Botpress's ZUI->JSON-schema
    // converter rejects at `bp deploy`. It is enforced fail-closed at runtime in
    // `assertRequiredConfig` (below) via `isSecureSidecarUrl`.
    .describe("HTTPS companion URL (plaintext http:// allowed only for loopback)"),
  spendguardBudgetId: z.string().min(1).describe("UUID of the SpendGuard budget to charge"),
  spendguardWindowInstanceId: z.string().min(1).describe("UUID of the SpendGuard window instance"),
  upstreamProvider: z
    .enum(["openai", "anthropic", "bedrock"])
    .describe("Upstream provider Botpress dispatches to"),
  tenantId: z.string().min(1).describe("Operator tenant identifier"),
  tlsCertPath: z.string().min(1).optional().describe("Path to SVID cert PEM"),
  tlsKeyPath: z.string().min(1).optional().describe("Path to SVID key PEM"),
  tlsRootCaPath: z.string().min(1).optional().describe("Path to sidecar CA PEM"),
});

export const ConfigurationSchema = z
  .object({
    sidecarUrl: z
      .string()
      .url()
      // Fail-closed transport gate: the decision/trace channel carries
      // DENY/ALLOW verdicts and prompts, so it MUST NOT be plaintext-capable
      // to a remote host. We require `https://` for any non-loopback host
      // (matching the Kong Go plugin which refuses plaintext sidecar URLs).
      // A plaintext `http://` URL is permitted ONLY for loopback hosts
      // (127.0.0.1 / ::1 / localhost), which never leave the pod and so are
      // not on a MITM-able network path (the standard "secure loopback"
      // exception used by browsers for the same reason).
      .refine(isSecureSidecarUrl, {
        message:
          "sidecarUrl must be https:// (plaintext http:// is allowed only for loopback hosts 127.0.0.1/::1/localhost)",
      })
      .describe("HTTPS companion URL (plaintext http:// allowed only for loopback)"),
    spendguardBudgetId: z.string().min(1).describe("UUID of the SpendGuard budget to charge"),
    spendguardWindowInstanceId: z
      .string()
      .min(1)
      .describe("UUID of the SpendGuard window instance"),
    upstreamProvider: z
      .enum(["openai", "anthropic", "bedrock"])
      .describe("Upstream provider Botpress dispatches to"),
    tenantId: z.string().min(1).describe("Operator tenant identifier"),
    tlsCertPath: z.string().min(1).optional().describe("Path to SVID cert PEM"),
    tlsKeyPath: z.string().min(1).optional().describe("Path to SVID key PEM"),
    tlsRootCaPath: z.string().min(1).optional().describe("Path to sidecar CA PEM"),
  })
  // mTLS material is all-or-none: a half-configured trio (cert without key, or
  // CA without cert) cannot produce a valid client identity and would silently
  // fall back to no client auth. Reject it at register time (fail closed).
  .refine(
    (cfg) => {
      const present = [cfg.tlsCertPath, cfg.tlsKeyPath, cfg.tlsRootCaPath].filter(
        (p) => p !== undefined,
      ).length;
      return present === 0 || present === 3;
    },
    {
      message:
        "tlsCertPath, tlsKeyPath and tlsRootCaPath must be supplied together (all three) or not at all",
      path: ["tlsCertPath"],
    },
  );

export type Configuration = z.infer<typeof ConfigurationSchema>;

/** Loopback hosts that never leave the pod, so a plaintext companion on one
 *  is not on a MITM-able network path. Lowercased for case-insensitive match;
 *  IPv6 loopback is compared after stripping surrounding brackets. */
const LOOPBACK_HOSTS = new Set(["127.0.0.1", "::1", "localhost"]);

/**
 * True when `sidecarUrl` is safe to dial: any `https://` URL, or an `http://`
 * URL whose host is loopback. Unparseable URLs are rejected (fail closed).
 *
 * Shared by the Zod schema (register-time form validation) and the
 * `SpendGuardReservation` constructor (synthetic-object path) so both gates
 * agree on the transport-security rule.
 */
export function isSecureSidecarUrl(raw: string): boolean {
  let u: URL;
  try {
    u = new URL(raw);
  } catch {
    return false;
  }
  if (u.protocol === "https:") return true;
  if (u.protocol !== "http:") return false;
  // hostname strips IPv6 brackets already; lowercase for localhost casing.
  return LOOPBACK_HOSTS.has(u.hostname.toLowerCase());
}

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
  // Transport-security gate (fail closed). Mirrors the Zod refinement so the
  // synthetic-object path (constructed without Botpress's form parse) cannot
  // dial a plaintext remote companion. `cfg.sidecarUrl` is guaranteed present
  // by the missing-field check above.
  if (!isSecureSidecarUrl(cfg.sidecarUrl as string)) {
    throw new SpendGuardConfigError(
      `spendguard:botpress: sidecarUrl must be https:// (plaintext http:// is allowed only for loopback hosts); got ${redactUrlForError(
        cfg.sidecarUrl as string,
      )}`,
    );
  }
  // mTLS material must be all-or-none (see ConfigurationSchema refinement).
  const tlsPresent = [cfg.tlsCertPath, cfg.tlsKeyPath, cfg.tlsRootCaPath].filter(
    (p) => p !== undefined && p !== "",
  ).length;
  if (tlsPresent !== 0 && tlsPresent !== 3) {
    throw new SpendGuardConfigError(
      "spendguard:botpress: tlsCertPath, tlsKeyPath and tlsRootCaPath must be supplied together (all three) or not at all",
    );
  }
}

/** Scheme + host breadcrumb for error messages; never leaks port or any
 *  embedded credential material (INV-6). */
function redactUrlForError(raw: string): string {
  try {
    const u = new URL(raw);
    return `${u.protocol}//${u.hostname}`;
  } catch {
    return "(invalid sidecarUrl)";
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
