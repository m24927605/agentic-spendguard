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
  // The Botpress runtime routes conversation-side handlers off the `code`
  // field on the RuntimeError, so the stable BUDGET_DENIED / BUDGET_DEGRADED /
  // BUDGET_CONFIG identifier MUST be carried through. @botpress/sdk's
  // RuntimeError constructor is message-only with a separate mutable `code`
  // field (index.d.ts: `constructor(message: string)` + `code?: string`), so
  // we assign the code after construction rather than passing an options bag —
  // a two-arg form would fail to type-check against the SDK and the extra arg
  // would be ignored at runtime.
  if (err instanceof DecisionDenied) {
    const rt = new RuntimeError(`SpendGuard denied: ${err.message}`);
    rt.code = err.code; // "BUDGET_DENIED"
    return rt;
  }
  if (err instanceof SidecarUnavailable) {
    const rt = new RuntimeError(`SpendGuard degraded: ${err.message}`);
    rt.code = err.code; // "BUDGET_DEGRADED"
    return rt;
  }
  if (err instanceof SpendGuardConfigError) {
    const rt = new RuntimeError(`SpendGuard config: ${err.message}`);
    rt.code = err.code; // "BUDGET_CONFIG"
    return rt;
  }
  if (err instanceof RuntimeError) {
    return err;
  }
  // Unknown-error-is-config fallback (mirrors the Python LiteLLM callback).
  const rt = new RuntimeError(
    `SpendGuard config: ${err instanceof Error ? err.message : String(err)}`,
  );
  rt.code = "BUDGET_CONFIG";
  return rt;
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
