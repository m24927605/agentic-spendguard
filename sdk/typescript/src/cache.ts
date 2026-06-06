// SpendGuard SDK — in-process idempotency cache (LRU + TTL).
//
// Same-process latency optimisation that prevents retried `reserve()` calls
// from issuing duplicate `requestDecision` RPCs. The sidecar maintains its
// OWN idempotency cache keyed by `IdempotencyKey.key` (it MUST — see
// design.md §6.5 / Stage 2 §4.6: the sidecar is the correctness gate). This
// cache lives ABOVE the sidecar's gate; a hit short-circuits the UDS round
// trip entirely. A miss falls through to the wire path normally.
//
// ── Spec lineage (LOCKED) ──────────────────────────────────────────────────
//
//   - design.md §3 module layout pins `decisionCache.ts` (line 35). Slice
//     doc renamed this to `cache.ts` to match the slice doc's deliverable
//     list (COV_S05_08 line 22). Both names are honored: the runtime file
//     is `cache.ts`, the design-doc slot is `decisionCache.ts`. **Declared
//     deviation #2** below.
//   - implementation.md §10 (one-line skeleton: "tiny LRU keyed by
//     idempotencyKey, default 1024 entries, TTL 5 minutes").
//   - design.md §6.5 (retry+cache interplay: retry NEVER bypasses the
//     cache because both consult the same idempotencyKey).
//
// ── Why an interface + two impls ───────────────────────────────────────────
//
// The slice doc (COV_S05_08 lines 22-26) calls out a disabled-mode "no-op"
// path. Exposing `IdempotencyCache` as an interface with `InMemoryIdempotencyCache`
// + `NoopIdempotencyCache` impls lets adapters:
//   1. Use the default in-memory LRU (the 99 % case).
//   2. Plug a Redis/Memcached-backed impl in customer code (forward-compat
//      for the ASP §10 multi-process caching slice — not in v0.1.x).
//   3. Disable caching entirely for tests that want to assert sidecar
//      receives every duplicated call.

import type { DecisionOutcome } from "./client.js";

/**
 * Idempotency cache contract. Adapters consume via
 * `SpendGuardClientConfig.idempotencyCache?: IdempotencyCache`.
 *
 * - `get(key)` MUST return the cached outcome for `key` when it is fresh
 *   (TTL-window unexpired), else `undefined`. Implementations MAY treat the
 *   read as a recency signal (LRU move-to-front); MUST NOT bump the TTL.
 * - `set(key, outcome, ttlMs?)` stores the outcome. Optional `ttlMs` overrides
 *   the cache's default TTL for this entry. Implementations MAY evict in
 *   amortised O(1) to honor a `maxEntries` cap.
 * - `clear()` resets the cache to empty.
 * - `size` is a snapshot getter; readers MAY observe a non-monotonic value
 *   across concurrent sets (no atomicity guarantee).
 */
export interface IdempotencyCache {
  get(key: string): DecisionOutcome | undefined;
  set(key: string, outcome: DecisionOutcome, ttlMs?: number): void;
  clear(): void;
  readonly size: number;
}

/**
 * Default cache cap. design.md §3 line 35 LOCKS this at "default 1024
 * entries". The number is a balance:
 *   - 1024 entries × ~512 B/entry (DecisionOutcome with reservation IDs +
 *     reason codes) → ~512 KiB ceiling. Fits comfortably in adapter
 *     working-set budgets.
 *   - 1024 is large enough to absorb a steady-state burst of in-flight
 *     decisions (each adapter request issues at most one `reserve`).
 *   - Override via `new InMemoryIdempotencyCache({ maxEntries: ... })`
 *     when adapters know their working set is larger.
 */
export const DEFAULT_CACHE_MAX_ENTRIES = 1024;

/**
 * Default TTL. design.md §3 line 35 + implementation.md §10 LOCK this at
 * "TTL 5 minutes" — long enough to cover a same-process retry storm
 * (the SLICE 8 retry helper has a 50 ms ceiling per attempt × 2 attempts =
 * ≤100 ms — the cache TTL must dominate by 3-4 orders of magnitude to be
 * useful) but short enough that a cache-bypass-via-eviction edge case
 * doesn't propagate stale outcomes hours after the original reservation
 * was already released by TTL sweep on the sidecar side.
 */
export const DEFAULT_CACHE_TTL_MS = 5 * 60 * 1000;

/** Options for `InMemoryIdempotencyCache`. */
export interface InMemoryIdempotencyCacheOptions {
  /** Max entries before LRU eviction. Default 1024. */
  maxEntries?: number;
  /** Default per-entry TTL in ms. Default 300_000. Per-call `ttlMs` overrides. */
  defaultTtlMs?: number;
  /** Override the clock for tests. Default `Date.now`. */
  now?: () => number;
}

/** Internal cache entry. */
interface CacheEntry {
  outcome: DecisionOutcome;
  expiresAt: number;
}

/**
 * In-memory LRU + TTL cache. Backed by a `Map` whose insertion order doubles
 * as the LRU axis:
 *   - `set` deletes-then-inserts so the entry moves to the most-recently-used
 *     end of the iteration order.
 *   - `get` on a fresh entry also delete-then-inserts (LRU move-to-front).
 *   - Eviction pops the oldest entry via `keys().next().value`.
 *
 * TTL is checked on every `get`: an expired entry is treated as a miss AND
 * is evicted immediately (lazy expiry — no background sweeper).
 *
 * Thread-safety: Node.js is single-threaded for user code. Worker_threads
 * cannot share `Map` references via structured clone, so the cache is
 * inherently per-thread. Multi-process deployments need an external cache;
 * see `IdempotencyCache` interface JSDoc.
 *
 * @example
 *   const cache = new InMemoryIdempotencyCache({ maxEntries: 2048 });
 *   const client = new SpendGuardClient({ idempotencyCache: cache, ... });
 */
export class InMemoryIdempotencyCache implements IdempotencyCache {
  private readonly entries = new Map<string, CacheEntry>();
  private readonly maxEntries: number;
  private readonly defaultTtlMs: number;
  private readonly now: () => number;

  constructor(opts: InMemoryIdempotencyCacheOptions = {}) {
    this.maxEntries = clampPositive(opts.maxEntries ?? DEFAULT_CACHE_MAX_ENTRIES);
    this.defaultTtlMs = clampPositive(opts.defaultTtlMs ?? DEFAULT_CACHE_TTL_MS);
    this.now = opts.now ?? Date.now;
  }

  get(key: string): DecisionOutcome | undefined {
    const entry = this.entries.get(key);
    if (entry === undefined) return undefined;
    if (entry.expiresAt <= this.now()) {
      // Lazy TTL expiry — drop the stale entry so the next set() doesn't
      // see it as a live LRU slot.
      this.entries.delete(key);
      return undefined;
    }
    // LRU move-to-front: re-insert so this key becomes the most recent.
    this.entries.delete(key);
    this.entries.set(key, entry);
    return entry.outcome;
  }

  set(key: string, outcome: DecisionOutcome, ttlMs?: number): void {
    const ttl = ttlMs !== undefined ? clampPositive(ttlMs) : this.defaultTtlMs;
    // Delete first so re-set moves to MRU end (Map preserves insertion order).
    this.entries.delete(key);
    this.entries.set(key, { outcome, expiresAt: this.now() + ttl });
    while (this.entries.size > this.maxEntries) {
      const oldestKey = this.entries.keys().next().value as string | undefined;
      if (oldestKey === undefined) break;
      this.entries.delete(oldestKey);
    }
  }

  clear(): void {
    this.entries.clear();
  }

  get size(): number {
    return this.entries.size;
  }
}

/**
 * No-op cache impl. Always misses on `get`; `set` discards. Useful for:
 *   - Tests that want to assert every reserved decision hits the sidecar.
 *   - Disabled-mode short-circuit (when `SPENDGUARD_DISABLE=1`, the client
 *     never reaches the cache path, but `NoopIdempotencyCache` is the
 *     conservative-default if an adapter passes `idempotencyCache: undefined`
 *     and wants explicit no-cache semantics).
 *
 * The `size` getter always returns 0; `clear()` is a no-op.
 */
export class NoopIdempotencyCache implements IdempotencyCache {
  get(_key: string): DecisionOutcome | undefined {
    return undefined;
  }
  set(_key: string, _outcome: DecisionOutcome, _ttlMs?: number): void {
    // intentional no-op
  }
  clear(): void {
    // intentional no-op
  }
  get size(): number {
    return 0;
  }
}

/** Clamp to a positive finite integer; falls back to the default on garbage. */
function clampPositive(value: number): number {
  if (!Number.isFinite(value) || !Number.isInteger(value) || value <= 0) {
    return DEFAULT_CACHE_MAX_ENTRIES;
  }
  return value;
}
