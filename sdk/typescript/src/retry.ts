// SpendGuard SDK — bounded retry helper.
//
// Tiny retry surface for the sidecar-side `UNAVAILABLE` / `DEADLINE_EXCEEDED`
// / `CANCELLED` cluster, mirroring Python `_classify_rpc_error` at
// `sdk/python/src/spendguard/client.py:929-941`. The substrate ships:
//
//   - `classifyRpcError(err)` — returns `"transient"` for the three-status
//     cluster, `"permanent"` for everything else. Identical bucketing to the
//     Python helper.
//   - `runWithRetry(fn, opts)` — invokes `fn`; on transient + idempotencyKey
//     present, retries once with a small constant backoff (25 ms + ≤25 ms
//     jitter). Permanent errors NEVER retry. Without `idempotencyKey`,
//     the helper bails immediately with `SidecarUnavailable(cause: err)`
//     so adapters can route on the typed exception.
//
// ── Spec lineage (LOCKED) ──────────────────────────────────────────────────
//
//   - design.md §6.5 lines 424-430 (retry classifier + idempotency guard).
//   - design.md §3 module layout pins `retry.ts` (line 33).
//   - implementation.md §11 (one-line skeleton).
//   - review-standards.md §1.5 P0 alias identity (we never wrap `reserve` in
//     a way that changes the reserve===requestDecision identity — the retry
//     wrapper is internal to `reserve`'s body, not a substitution).
//   - sdk/python/src/spendguard/client.py:929-941 (Python parity reference).
//
// ── Why max attempts = 2 and constant backoff ──────────────────────────────
//
// The sidecar-side `requestDecision` timeout is 250 ms p99 (design.md §4.2
// `DEFAULT_DECISION_TIMEOUT_MS`). A retry budget of 2 attempts × ~50 ms
// (25 ms base + ≤25 ms jitter) keeps the worst-case p99 round-trip under
// 600 ms even on full retry — well inside the operator-facing p99 budget
// for adapter call sites. Exponential backoff would either undershoot the
// jitter spread or push past the 600 ms ceiling; constant + jitter hits
// the right point in the design space.
//
// Per design.md §6.5 line 430, retry only runs when the caller passed a
// stable `idempotencyKey`. Without one, a retry would create a fresh
// decision on the second attempt (the sidecar's idempotency-cache key
// derived from the request body would differ), and the ledger would
// double-reserve — which is exactly the failure mode the substrate exists
// to prevent. The pointed `SidecarUnavailable` throw with `cause` set lets
// adapters route to their own retry budget (e.g. exponential at the agent
// runtime level) without losing the underlying error context.

import { SidecarUnavailable, SpendGuardError } from "./errors.js";

/**
 * Classification bucket for an RPC error per Python `_classify_rpc_error`.
 *
 * - `"transient"` → the sidecar / network is temporarily unavailable. Safe to
 *   retry IF the caller has a stable idempotency key.
 * - `"permanent"` → all other failures (invalid argument, decision denied,
 *   precondition failed, etc.). MUST NOT retry — retrying a permanent error
 *   either does nothing (waste of latency budget) or worse, re-asserts a
 *   broken request shape.
 */
export type RpcErrorClassification = "transient" | "permanent";

/**
 * The gRPC status codes that mirror Python's transient-error bucket.
 * Listed here as a frozen set so tests can assert the bucket is the locked
 * three (review-standards.md §1.2 verbatim-signature gate).
 *
 * The string values are the canonical `@grpc/grpc-js` status names; same
 * spelling as `grpc.StatusCode.<NAME>` and `protobuf-ts`'s `RpcError.code`.
 */
export const TRANSIENT_STATUS_CODES: ReadonlySet<string> = new Set([
  "UNAVAILABLE",
  "DEADLINE_EXCEEDED",
  "CANCELLED",
]);

/**
 * Classify an error as transient or permanent.
 *
 * Inputs accepted (in order of dispatch):
 *   1. `SidecarUnavailable` instance — `"transient"` (we already classified it
 *      upstream via `mapGrpcStatusToError`).
 *   2. Object with `.code` matching one of `TRANSIENT_STATUS_CODES` — typed
 *      `"transient"`. This is the `protobuf-ts` `RpcError` shape AND the
 *      `@grpc/grpc-js` ServiceError shape AND a duck-typed mock for tests.
 *   3. Everything else — `"permanent"`.
 *
 * Matches Python `_classify_rpc_error` line-for-line: same three statuses,
 * same fallthrough, same conservative default ("if we don't know, don't retry").
 *
 * @param err The error to classify. Accepts any thrown value (RpcError,
 *   SpendGuardError subclasses, plain Error, string, undefined, …).
 * @returns `"transient"` or `"permanent"`.
 *
 * @example
 *   import { classifyRpcError } from "@spendguard/sdk/retry";
 *
 *   try {
 *     await client.reserve(req);
 *   } catch (err) {
 *     if (classifyRpcError(err) === "transient") {
 *       // queue for retry
 *     } else {
 *       throw err;
 *     }
 *   }
 */
export function classifyRpcError(err: unknown): RpcErrorClassification {
  if (err instanceof SidecarUnavailable) return "transient";
  if (err !== null && typeof err === "object" && "code" in err) {
    const code = (err as { code: unknown }).code;
    if (typeof code === "string" && TRANSIENT_STATUS_CODES.has(code)) {
      return "transient";
    }
  }
  return "permanent";
}

/**
 * Options for `runWithRetry`.
 *
 * - `idempotencyKey` — REQUIRED for any retry to actually occur. Without one,
 *   `runWithRetry` runs `fn` exactly once and rethrows a pointed
 *   `SidecarUnavailable(cause: err)` on a transient error so the adapter can
 *   route. With one, `fn` runs up to `maxAttempts` times on transient errors.
 * - `maxAttempts` — total attempts (initial + retries). Default 2; legal
 *   range [1, 5]. The default 2 matches design.md §6.5 line 428 ("initial +
 *   1 retry"); values ≥ 5 require an explicit opt-in to discourage runaway
 *   retry loops in the substrate.
 * - `baseBackoffMs` — fixed delay between attempts. Default 25 ms per
 *   design.md §6.5 line 429.
 * - `jitterMs` — random delay added on top of `baseBackoffMs`. Default 25 ms;
 *   sampled uniformly from `[0, jitterMs]`. Avoids retry-storm pile-ups when
 *   N adapters share a single sidecar.
 * - `sleep` — pluggable sleeper for tests. Defaults to `setTimeout`.
 */
export interface RunWithRetryOptions {
  idempotencyKey?: string;
  maxAttempts?: number;
  baseBackoffMs?: number;
  jitterMs?: number;
  sleep?: (ms: number) => Promise<void>;
}

/**
 * Run `fn()` with bounded retry per design.md §6.5 + Python
 * `_classify_rpc_error` parity.
 *
 * Algorithm:
 *   1. Run `fn()`. If it succeeds, return the value.
 *   2. If it throws, classify the error via `classifyRpcError(err)`.
 *      - PERMANENT → rethrow as-is (NEVER retry).
 *      - TRANSIENT without `idempotencyKey` → throw
 *        `SidecarUnavailable(cause: err)` immediately. Caller's adapter is
 *        responsible for routing; the substrate refuses to retry without a
 *        stable key because a fresh decision on retry would double-reserve.
 *      - TRANSIENT with `idempotencyKey` AND attempts remaining → sleep
 *        `baseBackoffMs + uniform(0, jitterMs)`, then go to step 1.
 *      - TRANSIENT with `idempotencyKey` AND attempts exhausted → throw the
 *        LAST error (preserves the original `cause` if it's already a
 *        `SidecarUnavailable`).
 *
 * Permanent errors NEVER retry — design.md §6.5 line 426 spells out the
 * three-status transient cluster as the only retry-eligible surface.
 *
 * The `sleep` function in tests defaults to `setTimeout` — for unit tests,
 * pass a no-op (`async () => {}`) so the retry loop runs synchronously.
 *
 * @param fn The function to run. Awaited; thrown errors are inspected.
 * @param opts Retry options. `idempotencyKey` MUST be set for retries to
 *   actually occur.
 *
 * @returns The result of `fn()` on success.
 *
 * @throws SidecarUnavailable when `fn` fails transiently without an
 *   idempotency key.
 * @throws The last error from `fn` when attempts are exhausted (or the
 *   error type is permanent).
 *
 * @example
 *   await runWithRetry(
 *     () => client.requestDecision(grpcReq, { timeout: 250 }),
 *     { idempotencyKey: req.idempotencyKey, maxAttempts: 2 },
 *   );
 */
export async function runWithRetry<T>(
  fn: () => Promise<T>,
  opts: RunWithRetryOptions = {},
): Promise<T> {
  const idempotencyKey = opts.idempotencyKey;
  const maxAttempts = clampMaxAttempts(opts.maxAttempts ?? 2);
  const baseBackoffMs = opts.baseBackoffMs ?? 25;
  const jitterMs = opts.jitterMs ?? 25;
  const sleep = opts.sleep ?? defaultSleep;

  let lastErr: unknown;
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    try {
      return await fn();
    } catch (err) {
      const classification = classifyRpcError(err);
      if (classification === "permanent") {
        // Permanent errors NEVER retry — surface the original throw verbatim
        // so the caller's typed-exception routing works (DecisionDenied /
        // ApprovalRequired / MutationApplyFailed / etc).
        throw err;
      }
      // TRANSIENT — gate retry on idempotency key
      if (idempotencyKey === undefined || idempotencyKey.length === 0) {
        // No stable key → DO NOT retry. Throw a pointed SidecarUnavailable
        // with the original error as cause so the adapter sees the
        // underlying gRPC status.
        if (err instanceof SidecarUnavailable) {
          // Already pointed — preserve the cause chain (avoid double-wrap).
          throw err;
        }
        throw new SidecarUnavailable(
          `transient RPC failure with no idempotency key; refusing to retry: ${errorMessage(err)}`,
          { cause: err },
        );
      }
      lastErr = err;
      if (attempt < maxAttempts) {
        const delay = baseBackoffMs + Math.floor(Math.random() * (jitterMs + 1));
        await sleep(delay);
        continue;
      }
      // Attempts exhausted; throw the last error (already a
      // SidecarUnavailable in production; the cause chain is preserved).
      throw err;
    }
  }
  // Unreachable — the loop either returns or throws on every iteration.
  // Included only for TypeScript's control-flow analysis.
  throw lastErr ?? new SpendGuardError("runWithRetry: unreachable");
}

/**
 * Clamp `maxAttempts` to the legal range [1, 5] per `RunWithRetryOptions`
 * JSDoc. Values outside the range fall back to the nearest bound rather than
 * throwing so a misconfigured adapter doesn't crash on the first RPC.
 */
function clampMaxAttempts(value: number): number {
  if (!Number.isFinite(value) || !Number.isInteger(value)) return 2;
  if (value < 1) return 1;
  if (value > 5) return 5;
  return value;
}

/** Default sleeper — `setTimeout` wrapped in a Promise. */
function defaultSleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

/** Best-effort string extraction for non-Error throws. */
function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  return String(err);
}
