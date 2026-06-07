// SpendGuard error → Botpress `RuntimeError` translator.
//
// review-standards.md §3.7 / §3.8 / AD04 / AD05 / AD06:
//   - DecisionDenied             → RuntimeError(code="BUDGET_DENIED")
//   - SidecarUnavailable         → RuntimeError(code="BUDGET_DEGRADED")
//   - SpendGuardConfigError      → RuntimeError(code="BUDGET_CONFIG")
//
// The Botpress runtime uses the `code` field on the RuntimeError to route
// the error into deterministic conversation-side handlers (per
// @botpress/sdk@0.7 docs); the codes above are stable wire identifiers the
// docs page advertises (docs/site-v2/.../botpress.mdx).
//
// We import RuntimeError from @botpress/sdk; the unit tests use the same
// import so the constructor identity matches what the Botpress runtime
// instantiates at hook-dispatch time.

import { RuntimeError } from "@botpress/sdk";
import { DecisionDenied, SidecarUnavailable, SpendGuardConfigError } from "../reservation.js";

/**
 * Translate any SpendGuard-flavoured error to a Botpress `RuntimeError`
 * carrying a stable `code` field. Unrecognised inputs flow through as a
 * `BUDGET_CONFIG` runtime error with the original message preserved — this
 * mirrors the Python LiteLLM callback's "unknown-error-is-config" fallback
 * (sdk/python/src/spendguard/integrations/litellm.py:806-820).
 */
export function toRuntimeError(err: unknown): RuntimeError {
  if (err instanceof DecisionDenied) {
    return new RuntimeError(`SpendGuard denied: ${err.message}`);
  }
  if (err instanceof SidecarUnavailable) {
    return new RuntimeError(`SpendGuard degraded: ${err.message}`);
  }
  if (err instanceof SpendGuardConfigError) {
    return new RuntimeError(`SpendGuard config: ${err.message}`);
  }
  if (err instanceof RuntimeError) {
    return err;
  }
  return new RuntimeError(`SpendGuard config: ${err instanceof Error ? err.message : String(err)}`);
}

/**
 * Inspect the `code` a runtime error carries, given a SpendGuard-typed
 * input. Lets the unit tests assert AD04-AD06 without depending on Botpress's
 * internal RuntimeError shape.
 */
export function codeFor(
  err: DecisionDenied | SidecarUnavailable | SpendGuardConfigError,
): "BUDGET_DENIED" | "BUDGET_DEGRADED" | "BUDGET_CONFIG" {
  return err.code;
}
