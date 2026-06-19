// SpendGuard error -> Botpress `RuntimeError` translator.
//
//   - DecisionDenied        -> RuntimeError carrying spendguardCode=BUDGET_DENIED
//   - SidecarUnavailable    -> RuntimeError carrying spendguardCode=BUDGET_DEGRADED
//   - SpendGuardConfigError -> RuntimeError carrying spendguardCode=BUDGET_CONFIG
//
// IMPORTANT — the real `RuntimeError` (re-exported from `@botpress/client`,
// extending `BaseApiError`) has a READ-ONLY `code` field that is fixed to the
// HTTP status `400` for the Runtime error type. It is NOT a free-form string
// slot, so the stable SpendGuard wire identifier (BUDGET_DENIED / ... ) CANNOT
// be assigned to `rt.code`. Instead we thread it through the constructor's
// `metadata` bag (`{ spendguardCode }`) and embed it in the message, where
// Botpress surfaces it to operators and conversation-side error handlers.
//
// RuntimeError constructor signature (client 1.46.x):
//   new RuntimeError(message: string, error?: Error, id?: string,
//                    metadata?: Record<string, unknown>)
//
// Tests assert the SpendGuard code via `runtimeErrorCode(rt)` (reads the
// metadata bag) and `codeFor(err)` (reads the source error), so they never
// depend on the read-only numeric `code`.
import { RuntimeError } from "@botpress/sdk";
import { DecisionDenied, SidecarUnavailable, SpendGuardConfigError } from "../reservation.js";

export type SpendGuardCode = "BUDGET_DENIED" | "BUDGET_DEGRADED" | "BUDGET_CONFIG";

/** Build a Botpress RuntimeError carrying the stable SpendGuard code in its
 *  metadata bag (since the numeric `code` field is read-only). */
function runtimeError(
  spendguardCode: SpendGuardCode,
  message: string,
  cause?: Error,
): RuntimeError {
  return new RuntimeError(message, cause, undefined, { spendguardCode });
}

/**
 * Translate any SpendGuard-flavoured error to a Botpress `RuntimeError`.
 * Unrecognised inputs flow through as a `BUDGET_CONFIG` runtime error with the
 * original message preserved — this mirrors the Python LiteLLM callback's
 * "unknown-error-is-config" fallback
 * (sdk/python/src/spendguard/integrations/litellm.py:806-820).
 */
export function toRuntimeError(err: unknown): RuntimeError {
  if (err instanceof DecisionDenied) {
    return runtimeError("BUDGET_DENIED", `SpendGuard denied: ${err.message}`, err);
  }
  if (err instanceof SidecarUnavailable) {
    return runtimeError("BUDGET_DEGRADED", `SpendGuard degraded: ${err.message}`, err);
  }
  if (err instanceof SpendGuardConfigError) {
    return runtimeError("BUDGET_CONFIG", `SpendGuard config: ${err.message}`, err);
  }
  if (err instanceof RuntimeError) {
    return err;
  }
  // Unknown-error-is-config fallback (mirrors the Python LiteLLM callback).
  const cause = err instanceof Error ? err : undefined;
  return runtimeError(
    "BUDGET_CONFIG",
    `SpendGuard config: ${err instanceof Error ? err.message : String(err)}`,
    cause,
  );
}

/**
 * Read the SpendGuard code a translated RuntimeError carries (from its
 * metadata bag). Returns `undefined` if the RuntimeError was not minted by
 * `toRuntimeError`. Lets tests assert the translation without depending on
 * Botpress's internal numeric `code`.
 */
export function runtimeErrorCode(rt: RuntimeError): SpendGuardCode | undefined {
  const meta = (rt as { metadata?: Record<string, unknown> }).metadata;
  const code = meta?.spendguardCode;
  return code === "BUDGET_DENIED" || code === "BUDGET_DEGRADED" || code === "BUDGET_CONFIG"
    ? code
    : undefined;
}

/**
 * Inspect the `code` a SpendGuard-typed source error carries. Lets the unit
 * tests assert the error -> code mapping at the source.
 */
export function codeFor(
  err: DecisionDenied | SidecarUnavailable | SpendGuardConfigError,
): SpendGuardCode {
  return err.code;
}
