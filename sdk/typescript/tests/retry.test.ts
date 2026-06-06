// SLICE 8 — retry helper tests (design.md §6.5).
//
// Mirrors Python `_classify_rpc_error` parity at
// `sdk/python/src/spendguard/client.py:929-941`:
//
//   - UNAVAILABLE / DEADLINE_EXCEEDED / CANCELLED → "transient"
//   - everything else → "permanent"
//
// And the runWithRetry contract per design §6.5 line 430:
//
//   - max attempts = 2 (initial + 1 retry).
//   - constant backoff 25ms + ≤25ms jitter (not exponential — see retry.ts JSDoc).
//   - REQUIRES `idempotencyKey`; without one, throws SidecarUnavailable
//     immediately on transient with `cause` set to the original error.
//
// Tests use a no-op sleeper so the retry loop runs synchronously.

import { describe, expect, it, vi } from "vitest";

import { SidecarUnavailable, SpendGuardError } from "../src/errors.js";
import { TRANSIENT_STATUS_CODES, classifyRpcError, runWithRetry } from "../src/retry.js";

const noSleep = async (_ms: number) => {
  // synchronous in tests
};

// ── classifyRpcError ──────────────────────────────────────────────────────

describe("classifyRpcError — Python parity (_classify_rpc_error)", () => {
  it("classifies UNAVAILABLE as transient", () => {
    expect(classifyRpcError({ code: "UNAVAILABLE" })).toBe("transient");
  });

  it("classifies DEADLINE_EXCEEDED as transient", () => {
    expect(classifyRpcError({ code: "DEADLINE_EXCEEDED" })).toBe("transient");
  });

  it("classifies CANCELLED as transient", () => {
    expect(classifyRpcError({ code: "CANCELLED" })).toBe("transient");
  });

  it("classifies INVALID_ARGUMENT as permanent (does not retry policy errors)", () => {
    expect(classifyRpcError({ code: "INVALID_ARGUMENT" })).toBe("permanent");
  });

  it("classifies FAILED_PRECONDITION as permanent (would retry-loop budget exceeded)", () => {
    expect(classifyRpcError({ code: "FAILED_PRECONDITION" })).toBe("permanent");
  });

  it("classifies NOT_FOUND as permanent (would loop on missing reservation)", () => {
    expect(classifyRpcError({ code: "NOT_FOUND" })).toBe("permanent");
  });

  it("classifies a SidecarUnavailable instance as transient (already classified upstream)", () => {
    const err = new SidecarUnavailable("rpc down");
    expect(classifyRpcError(err)).toBe("transient");
  });

  it("classifies undefined as permanent (conservative default)", () => {
    expect(classifyRpcError(undefined)).toBe("permanent");
  });

  it("classifies plain string as permanent", () => {
    expect(classifyRpcError("oops")).toBe("permanent");
  });

  it("locks the transient-status-codes set to exactly the three Python codes", () => {
    // §1.2 P0 verbatim signature gate: this set IS the cross-language contract
    // — Python's `_classify_rpc_error` lists exactly these three statuses.
    expect([...TRANSIENT_STATUS_CODES].sort()).toEqual([
      "CANCELLED",
      "DEADLINE_EXCEEDED",
      "UNAVAILABLE",
    ]);
  });
});

// ── runWithRetry: no idempotencyKey ───────────────────────────────────────

describe("runWithRetry — no idempotencyKey path (design §6.5 line 430)", () => {
  it("runs fn exactly once on success even without idempotencyKey", async () => {
    const fn = vi.fn(async () => "ok");
    const result = await runWithRetry(fn, { sleep: noSleep });
    expect(result).toBe("ok");
    expect(fn).toHaveBeenCalledOnce();
  });

  it("does NOT retry on transient error without idempotencyKey; throws SidecarUnavailable with cause", async () => {
    const original = { code: "UNAVAILABLE", message: "sidecar down" };
    const fn = vi.fn(async () => {
      throw original;
    });
    let caught: unknown;
    try {
      await runWithRetry(fn, { sleep: noSleep });
    } catch (err) {
      caught = err;
    }
    expect(fn).toHaveBeenCalledOnce(); // EXACTLY once — no retry
    expect(caught).toBeInstanceOf(SidecarUnavailable);
    expect((caught as SidecarUnavailable).cause).toBe(original);
  });

  it("preserves an already-SidecarUnavailable throw without double-wrapping", async () => {
    const original = new SidecarUnavailable("upstream wrapped", { cause: { code: "UNAVAILABLE" } });
    const fn = async () => {
      throw original;
    };
    let caught: unknown;
    try {
      await runWithRetry(fn, { sleep: noSleep });
    } catch (err) {
      caught = err;
    }
    // Identity preserved — no double-wrap on retry refusal
    expect(caught).toBe(original);
    expect((caught as SidecarUnavailable).cause).toEqual({ code: "UNAVAILABLE" });
  });

  it("re-throws permanent errors verbatim (no SidecarUnavailable wrap)", async () => {
    const original = { code: "INVALID_ARGUMENT", message: "bad arg" };
    const fn = async () => {
      throw original;
    };
    let caught: unknown;
    try {
      await runWithRetry(fn, { sleep: noSleep });
    } catch (err) {
      caught = err;
    }
    expect(caught).toBe(original);
  });
});

// ── runWithRetry: with idempotencyKey ─────────────────────────────────────

describe("runWithRetry — with idempotencyKey path", () => {
  it("retries once on transient error; total 2 invocations (design §6.5 line 428)", async () => {
    let calls = 0;
    const fn = vi.fn(async () => {
      calls++;
      if (calls === 1) throw { code: "UNAVAILABLE", message: "first attempt fails" };
      return "ok";
    });
    const result = await runWithRetry(fn, {
      idempotencyKey: "sg-abc",
      sleep: noSleep,
    });
    expect(result).toBe("ok");
    expect(fn).toHaveBeenCalledTimes(2);
  });

  it("retries up to maxAttempts then throws the last error", async () => {
    const lastErr = { code: "DEADLINE_EXCEEDED", message: "still timing out" };
    let calls = 0;
    const fn = vi.fn(async () => {
      calls++;
      throw lastErr;
    });
    let caught: unknown;
    try {
      await runWithRetry(fn, {
        idempotencyKey: "sg-abc",
        maxAttempts: 2,
        sleep: noSleep,
      });
    } catch (err) {
      caught = err;
    }
    expect(fn).toHaveBeenCalledTimes(2);
    expect(caught).toBe(lastErr);
  });

  it("does NOT retry on PERMANENT error even with idempotencyKey", async () => {
    const fn = vi.fn(async () => {
      throw { code: "INVALID_ARGUMENT", message: "bad arg" };
    });
    await expect(
      runWithRetry(fn, { idempotencyKey: "sg-abc", sleep: noSleep }),
    ).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
    expect(fn).toHaveBeenCalledOnce();
  });

  it("respects custom maxAttempts (3 → max 3 calls)", async () => {
    let calls = 0;
    const fn = vi.fn(async () => {
      calls++;
      if (calls < 3) throw { code: "CANCELLED" };
      return "third-time-lucky";
    });
    const result = await runWithRetry(fn, {
      idempotencyKey: "sg-abc",
      maxAttempts: 3,
      sleep: noSleep,
    });
    expect(result).toBe("third-time-lucky");
    expect(fn).toHaveBeenCalledTimes(3);
  });

  it("calls sleep between attempts (constant backoff per design §6.5 line 429)", async () => {
    const sleep = vi.fn(async (_ms: number) => {});
    const fn = vi.fn(async () => {
      throw { code: "UNAVAILABLE" };
    });
    try {
      await runWithRetry(fn, {
        idempotencyKey: "sg-abc",
        maxAttempts: 2,
        baseBackoffMs: 25,
        jitterMs: 25,
        sleep,
      });
    } catch {
      // expected
    }
    // sleep called exactly (maxAttempts - 1) times = 1
    expect(sleep).toHaveBeenCalledOnce();
    const firstCall = sleep.mock.calls[0];
    expect(firstCall).toBeDefined();
    const delayArg = firstCall?.[0] ?? -1;
    // baseBackoffMs + uniform[0, jitterMs] → 25..50
    expect(delayArg).toBeGreaterThanOrEqual(25);
    expect(delayArg).toBeLessThanOrEqual(50);
  });

  it("treats empty-string idempotencyKey as no key (refuses retry)", async () => {
    const fn = vi.fn(async () => {
      throw { code: "UNAVAILABLE" };
    });
    await expect(runWithRetry(fn, { idempotencyKey: "", sleep: noSleep })).rejects.toBeInstanceOf(
      SidecarUnavailable,
    );
    expect(fn).toHaveBeenCalledOnce();
  });
});

// ── runWithRetry: edge cases ──────────────────────────────────────────────

describe("runWithRetry — edge cases", () => {
  it("clamps maxAttempts to [1, 5] (out-of-range becomes nearest bound)", async () => {
    let calls = 0;
    const fn = vi.fn(async () => {
      calls++;
      throw { code: "UNAVAILABLE" };
    });
    // maxAttempts = 100 should clamp to 5
    try {
      await runWithRetry(fn, {
        idempotencyKey: "sg-abc",
        maxAttempts: 100,
        sleep: noSleep,
      });
    } catch {
      // expected
    }
    expect(fn).toHaveBeenCalledTimes(5);
  });

  it("maxAttempts < 1 clamps to 1 (single attempt)", async () => {
    const fn = vi.fn(async () => "ok");
    await runWithRetry(fn, {
      idempotencyKey: "sg-abc",
      maxAttempts: 0,
      sleep: noSleep,
    });
    expect(fn).toHaveBeenCalledOnce();
  });

  it("preserves a SpendGuardError throw verbatim through retry loop", async () => {
    const err = new SpendGuardError("custom failure");
    const fn = vi.fn(async () => {
      throw err;
    });
    // SpendGuardError without explicit transient classification is permanent
    await expect(runWithRetry(fn, { idempotencyKey: "sg-abc", sleep: noSleep })).rejects.toBe(err);
    expect(fn).toHaveBeenCalledOnce(); // NEVER retried (permanent)
  });
});
