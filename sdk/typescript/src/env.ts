// SpendGuard SDK — environment-variable resolution.
//
// SLICE 3 wires the env-var lookup helpers used by the constructor + the
// `fromEnv()` factory. The full schema lives in design.md §5.1 (LOCKED public
// surface); the slice doc adds two compatibility aliases and an explicit default
// socket path:
//
//   - `SPENDGUARD_SOCKET_PATH` is accepted as a co-equal alias of
//     `SPENDGUARD_SIDECAR_UDS`. The slice doc explicitly names the new alias;
//     design.md §5.1 names the canonical var. We honor both for forward
//     compatibility — slice-doc alias wins when both are set so the slice
//     reviewer's literal intent is preserved.
//   - The slice doc nails the default to `/var/run/spendguard/adapter.sock`
//     when neither alias is set; design.md leaves the default to "—" (caller
//     must pass). The default applies to `fromEnv()` only — the bare
//     constructor still throws if no explicit `socketPath` and no env alias
//     are present, matching design.md §5.2.
//
// No `disabled` short-circuit logic ships here; SLICE 4 wires it once the RPC
// methods exist to short-circuit.

/**
 * Default UDS path used by `SpendGuardClient.fromEnv()` when neither
 * `SPENDGUARD_SOCKET_PATH` nor `SPENDGUARD_SIDECAR_UDS` is set. Matches the
 * sidecar Helm chart's default volume mount per slice spec.
 */
export const DEFAULT_SOCKET_PATH = "/var/run/spendguard/adapter.sock" as const;

/**
 * Resolved env-var snapshot. Every field is optional because the env may be
 * sparsely populated; the constructor combines this snapshot with the explicit
 * options and validates the union.
 *
 * `runProjectionDefault` is typed as the same `RunProjectionPolicy` literal
 * union surfaced by `SpendGuardClientConfig` so the env path produces the
 * same shape the explicit-options path produces (no `string` vs literal-union
 * drift between the two entry points).
 */
export interface ResolvedEnvConfig {
  socketPath?: string;
  tenantId?: string;
  workloadInstanceId?: string;
  decisionTimeoutMs?: number;
  handshakeTimeoutMs?: number;
  runProjectionDefault?: import("./config.js").RunProjectionPolicy;
  disabled?: boolean;
  /**
   * Deployment profile, read verbatim from `SPENDGUARD_PROFILE`. The
   * constructor uses this to gate the env-var disable path: a bare
   * `SPENDGUARD_DISABLE` ONLY takes effect when `SPENDGUARD_PROFILE=demo`,
   * mirroring the Rust signing service's `DisabledSigner::for_profile`
   * (`services/signing/src/lib.rs`) so a single mis-set env var cannot defeat
   * enforcement in production. Lowercased for case-insensitive comparison;
   * `undefined` when unset.
   */
  profile?: string;
}

/**
 * Reads env vars into a typed snapshot. Pure read — never mutates `process.env`
 * and never throws. Validation lives in `validateConfig` (config.ts) so the
 * error site has the merged context.
 *
 * @param env Optional env override (defaults to `process.env`); used by tests
 *   to avoid leaking real env into other test files.
 */
export function resolveEnvConfig(env: NodeJS.ProcessEnv = process.env): ResolvedEnvConfig {
  const out: ResolvedEnvConfig = {};

  // Socket path — slice-doc alias wins over canonical var (see comment at top).
  const socketPath = env.SPENDGUARD_SOCKET_PATH ?? env.SPENDGUARD_SIDECAR_UDS;
  if (socketPath !== undefined && socketPath.length > 0) {
    out.socketPath = socketPath;
  }

  const tenantId = env.SPENDGUARD_TENANT_ID;
  if (tenantId !== undefined && tenantId.length > 0) {
    out.tenantId = tenantId;
  }

  const workloadInstanceId = env.SPENDGUARD_WORKLOAD_INSTANCE_ID;
  if (workloadInstanceId !== undefined && workloadInstanceId.length > 0) {
    out.workloadInstanceId = workloadInstanceId;
  }

  const decisionTimeoutMs = parsePositiveIntegerEnv(
    env.SPENDGUARD_DECISION_TIMEOUT_MS,
    "SPENDGUARD_DECISION_TIMEOUT_MS",
  );
  if (decisionTimeoutMs !== undefined) {
    out.decisionTimeoutMs = decisionTimeoutMs;
  }

  const handshakeTimeoutMs = parsePositiveIntegerEnv(
    env.SPENDGUARD_HANDSHAKE_TIMEOUT_MS,
    "SPENDGUARD_HANDSHAKE_TIMEOUT_MS",
  );
  if (handshakeTimeoutMs !== undefined) {
    out.handshakeTimeoutMs = handshakeTimeoutMs;
  }

  const runProjectionDefault = env.SPENDGUARD_RUN_PROJECTION_DEFAULT;
  if (runProjectionDefault !== undefined && runProjectionDefault.length > 0) {
    out.runProjectionDefault = runProjectionDefault;
  }

  // Profile — read verbatim (lowercased) so the constructor can gate the
  // env-var disable path behind `SPENDGUARD_PROFILE=demo` (see ResolvedEnvConfig).
  const profile = env.SPENDGUARD_PROFILE;
  if (profile !== undefined && profile.length > 0) {
    out.profile = profile.toLowerCase();
  }

  // Disabled — accepted spellings per design §5.1 ("1" / "true"). Case
  // insensitive for ergonomics.
  //
  // NOTE: this only records the RAW env signal. The constructor REFUSES to
  // honor it unless `SPENDGUARD_PROFILE=demo`, so a single mis-set env var
  // cannot silently turn enforcement off in production. The explicit
  // `disabled: true` constructor option is unaffected (tests rely on it).
  const disabled = env.SPENDGUARD_DISABLE;
  if (disabled !== undefined) {
    const norm = disabled.toLowerCase();
    if (norm === "1" || norm === "true" || norm === "yes" || norm === "on") {
      out.disabled = true;
    }
  }

  return out;
}

/**
 * Parse a positive integer env var. Throws via the caller's validator if the
 * value is set but cannot be parsed — we surface a clear name here so the
 * downstream `SpendGuardConfigError` carries the offending var name verbatim.
 *
 * The parse itself returns `undefined` for "unset" and the integer for "set
 * and valid"; a thrown `EnvParseError` for "set and invalid".
 */
function parsePositiveIntegerEnv(raw: string | undefined, name: string): number | undefined {
  if (raw === undefined || raw.length === 0) return undefined;
  const n = Number(raw);
  if (!Number.isFinite(n) || !Number.isInteger(n) || n < 0) {
    throw new EnvParseError(
      `env var ${name}=${JSON.stringify(raw)} is not a finite non-negative integer`,
      name,
    );
  }
  return n;
}

/**
 * Internal — thrown by `parsePositiveIntegerEnv` for ill-formed env vars. The
 * constructor catches this and rewraps as `SpendGuardConfigError` so callers
 * only see SpendGuard-typed exceptions. Not part of the public surface.
 */
export class EnvParseError extends Error {
  override name = "EnvParseError";
  readonly varName: string;
  constructor(message: string, varName: string) {
    super(message);
    this.varName = varName;
  }
}
