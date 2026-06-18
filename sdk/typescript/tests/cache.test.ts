// SLICE 8 — idempotency cache tests (design.md §3 layout, impl §10).
//
// LRU + TTL semantics:
//   - `set(key, outcome, ttlMs?)` stores; LRU evicts at maxEntries.
//   - `get(key)` returns the cached outcome if fresh, else undefined; bumps
//     the entry to most-recently-used (LRU move-to-front).
//   - TTL expiry is lazy: an expired entry is dropped on the first `get`
//     that observes it.
//
// NoopIdempotencyCache MUST be transparently no-op so adapters can opt out
// without re-implementing the interface.

import { describe, expect, it } from "vitest";

import {
  DEFAULT_CACHE_MAX_ENTRIES,
  DEFAULT_CACHE_TTL_MS,
  InMemoryIdempotencyCache,
  NoopIdempotencyCache,
} from "../src/cache.js";
import type { DecisionOutcome } from "../src/client.js";

function mkOutcome(decisionId: string): DecisionOutcome {
  return {
    decisionId,
    auditDecisionEventId: `audit-${decisionId}`,
    decision: "CONTINUE",
    mutationPatchJson: "{}",
    effectHash: new Uint8Array(),
    ledgerTransactionId: `ledger-${decisionId}`,
    reservationIds: [`res-${decisionId}`],
    ttlExpiresAtSeconds: 999_999,
    reasonCodes: [],
    matchedRuleIds: [],
  };
}

// ── InMemoryIdempotencyCache ──────────────────────────────────────────────

describe("InMemoryIdempotencyCache — get/set round-trip", () => {
  it("set then get returns the stored outcome", () => {
    const cache = new InMemoryIdempotencyCache();
    const outcome = mkOutcome("d1");
    cache.set("sg-key-1", outcome);
    expect(cache.get("sg-key-1")).toBe(outcome);
  });

  // Body-identity binding (idempotency-key collision protection).
  it("get with a MISMATCHED bodyHash is a miss (collision falls through)", () => {
    const cache = new InMemoryIdempotencyCache();
    const outcome = mkOutcome("d1");
    cache.set("sg-key", outcome, undefined, "hash-A");
    // Same key, different body hash → treated as a miss.
    expect(cache.get("sg-key", "hash-B")).toBeUndefined();
    // Original body hash still hits.
    expect(cache.get("sg-key", "hash-A")).toBe(outcome);
    // A mismatch must NOT evict the live entry (original request may replay).
    expect(cache.size).toBe(1);
  });

  it("get with a MATCHING bodyHash returns the stored outcome", () => {
    const cache = new InMemoryIdempotencyCache();
    const outcome = mkOutcome("d1");
    cache.set("sg-key", outcome, undefined, "hash-A");
    expect(cache.get("sg-key", "hash-A")).toBe(outcome);
  });

  it("get without a bodyHash still hits (key-only legacy semantics preserved)", () => {
    const cache = new InMemoryIdempotencyCache();
    const outcome = mkOutcome("d1");
    cache.set("sg-key", outcome, undefined, "hash-A");
    expect(cache.get("sg-key")).toBe(outcome);
  });

  it("get on an unset key returns undefined", () => {
    const cache = new InMemoryIdempotencyCache();
    expect(cache.get("never-set")).toBeUndefined();
  });

  it("size reflects current entry count", () => {
    const cache = new InMemoryIdempotencyCache();
    expect(cache.size).toBe(0);
    cache.set("a", mkOutcome("a"));
    expect(cache.size).toBe(1);
    cache.set("b", mkOutcome("b"));
    expect(cache.size).toBe(2);
  });

  it("re-set on same key updates the outcome (does not duplicate)", () => {
    const cache = new InMemoryIdempotencyCache();
    cache.set("k", mkOutcome("v1"));
    expect(cache.size).toBe(1);
    cache.set("k", mkOutcome("v2"));
    expect(cache.size).toBe(1);
    expect(cache.get("k")?.decisionId).toBe("v2");
  });
});

describe("InMemoryIdempotencyCache — LRU eviction at maxEntries", () => {
  it("evicts the least-recently-used entry when size > maxEntries", () => {
    const cache = new InMemoryIdempotencyCache({ maxEntries: 2 });
    cache.set("a", mkOutcome("a"));
    cache.set("b", mkOutcome("b"));
    cache.set("c", mkOutcome("c")); // should evict "a"
    expect(cache.size).toBe(2);
    expect(cache.get("a")).toBeUndefined();
    expect(cache.get("b")).toBeDefined();
    expect(cache.get("c")).toBeDefined();
  });

  it("get on a fresh entry bumps it to MRU (delays eviction)", () => {
    const cache = new InMemoryIdempotencyCache({ maxEntries: 2 });
    cache.set("a", mkOutcome("a"));
    cache.set("b", mkOutcome("b"));
    // Touch "a" — moves it to MRU end
    cache.get("a");
    // Now insert "c" — should evict "b" (now LRU), not "a"
    cache.set("c", mkOutcome("c"));
    expect(cache.get("a")).toBeDefined();
    expect(cache.get("b")).toBeUndefined();
    expect(cache.get("c")).toBeDefined();
  });

  it("default maxEntries is 1024 per design.md §3 line 35 LOCK", () => {
    expect(DEFAULT_CACHE_MAX_ENTRIES).toBe(1024);
  });
});

describe("InMemoryIdempotencyCache — TTL expiry", () => {
  it("expired entries are treated as misses", () => {
    let now = 1_000;
    const cache = new InMemoryIdempotencyCache({
      defaultTtlMs: 100,
      now: () => now,
    });
    cache.set("k", mkOutcome("v"));
    expect(cache.get("k")).toBeDefined();
    // Advance clock past TTL
    now = 1_200;
    expect(cache.get("k")).toBeUndefined();
  });

  it("expired entries are evicted on get (lazy expiry)", () => {
    let now = 1_000;
    const cache = new InMemoryIdempotencyCache({
      defaultTtlMs: 100,
      now: () => now,
    });
    cache.set("k", mkOutcome("v"));
    now = 1_200;
    cache.get("k"); // triggers eviction
    expect(cache.size).toBe(0);
  });

  it("per-call ttlMs overrides defaultTtlMs", () => {
    let now = 1_000;
    const cache = new InMemoryIdempotencyCache({
      defaultTtlMs: 100,
      now: () => now,
    });
    cache.set("k", mkOutcome("v"), 1_000); // custom 1s TTL
    now = 1_500;
    // Custom TTL keeps it alive past defaultTtlMs
    expect(cache.get("k")).toBeDefined();
    now = 2_500;
    // Past custom TTL
    expect(cache.get("k")).toBeUndefined();
  });

  it("default TTL is 5 minutes per design.md §3 line 35 LOCK", () => {
    expect(DEFAULT_CACHE_TTL_MS).toBe(5 * 60 * 1000);
  });

  it("an entry at exactly expiresAt is treated as expired (<= boundary)", () => {
    let now = 1_000;
    const cache = new InMemoryIdempotencyCache({
      defaultTtlMs: 100,
      now: () => now,
    });
    cache.set("k", mkOutcome("v")); // expiresAt = 1100
    now = 1_100;
    expect(cache.get("k")).toBeUndefined();
  });
});

describe("InMemoryIdempotencyCache — clear()", () => {
  it("clear() empties the cache", () => {
    const cache = new InMemoryIdempotencyCache();
    cache.set("a", mkOutcome("a"));
    cache.set("b", mkOutcome("b"));
    expect(cache.size).toBe(2);
    cache.clear();
    expect(cache.size).toBe(0);
    expect(cache.get("a")).toBeUndefined();
    expect(cache.get("b")).toBeUndefined();
  });
});

// ── NoopIdempotencyCache ──────────────────────────────────────────────────

describe("NoopIdempotencyCache — disabled-mode shim", () => {
  it("get always returns undefined regardless of prior set", () => {
    const cache = new NoopIdempotencyCache();
    cache.set("k", mkOutcome("v"));
    expect(cache.get("k")).toBeUndefined();
  });

  it("set is a no-op (size stays 0)", () => {
    const cache = new NoopIdempotencyCache();
    expect(cache.size).toBe(0);
    cache.set("a", mkOutcome("a"));
    cache.set("b", mkOutcome("b"));
    expect(cache.size).toBe(0);
  });

  it("clear is a no-op (does not throw)", () => {
    const cache = new NoopIdempotencyCache();
    expect(() => cache.clear()).not.toThrow();
  });

  it("conforms to the IdempotencyCache interface (structural typecheck)", () => {
    const cache = new NoopIdempotencyCache();
    // The fact that this typechecks proves the interface is satisfied.
    // The runtime expectations are also asserted above.
    expect(typeof cache.get).toBe("function");
    expect(typeof cache.set).toBe("function");
    expect(typeof cache.clear).toBe("function");
    expect(typeof cache.size).toBe("number");
  });
});

// ── Garbage-input clamping ────────────────────────────────────────────────

describe("InMemoryIdempotencyCache — garbage-input clamping", () => {
  it("non-finite maxEntries falls back to the default (1024)", () => {
    const cache = new InMemoryIdempotencyCache({ maxEntries: Number.NaN });
    // 1024-entry cap: fill 1025 entries and observe one eviction
    for (let i = 0; i < 1025; i++) {
      cache.set(`k${i}`, mkOutcome(`v${i}`));
    }
    expect(cache.size).toBe(1024);
  });

  it("negative TTL falls back to the default", () => {
    const cache = new InMemoryIdempotencyCache({ defaultTtlMs: -1 });
    cache.set("k", mkOutcome("v"));
    // Should still be retrievable since negative ttl is clamped to default 5m
    expect(cache.get("k")).toBeDefined();
  });

  // Regression: garbage per-call ttlMs must fall back to defaultTtlMs, NOT to
  // the max-entries count constant (1024ms). See clampTtl in cache.ts.
  it.each([0, -5, Number.NaN, Number.POSITIVE_INFINITY, 1.5])(
    "per-call ttlMs=%s falls back to defaultTtlMs (not the 1024 count constant)",
    (badTtl) => {
      let now = 1_000;
      const cache = new InMemoryIdempotencyCache({
        defaultTtlMs: 100_000, // 100s — well past the bogus 1024ms
        now: () => now,
      });
      cache.set("k", mkOutcome("v"), badTtl);
      // If the old clampPositive(1024) bug were present, this would already be
      // expired at now=2_000; with the default 100s TTL it is still live.
      now = 2_000;
      expect(cache.get("k")).toBeDefined();
      // And it does expire at the DEFAULT TTL boundary, proving we used 100s.
      now = 1_000 + 100_000;
      expect(cache.get("k")).toBeUndefined();
    },
  );
});
