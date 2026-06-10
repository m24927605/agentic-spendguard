// COV_D38_02 — InflightMap tests (tests.md TP-32..TP-35, gate A3.6).
//
// V3 PINNED to the LOCKED §6.5 per-runId FIFO fallback (see src/inflight.ts
// header): keys are adapter-derived runIds; pop is FIFO within key; global
// capacity 10_000 with FIFO eviction (D04 parity).

import { describe, expect, it } from "vitest";
import { type InflightEntry, InflightMap } from "../src/inflight.js";

function entry(overrides: Partial<InflightEntry> & { llmCallId: string }): InflightEntry {
  return {
    decisionId: overrides.decisionId ?? `dec-${overrides.llmCallId}`,
    reservationId: overrides.reservationId ?? `res-${overrides.llmCallId}`,
    runId: overrides.runId ?? "run-default",
    llmCallId: overrides.llmCallId,
    idempotencyKey: overrides.idempotencyKey ?? `sg-${overrides.llmCallId}`,
    projectedAmountAtomic: overrides.projectedAmountAtomic ?? "1000",
    unit: overrides.unit ?? { unit: "USD_MICROS", denomination: 1 },
  };
}

describe("COV_D38_02 InflightMap (TP-32..TP-35)", () => {
  it("TP-32: push/pop round-trip; second pop → undefined", () => {
    const map = new InflightMap();
    const e = entry({ llmCallId: "llm-1", runId: "run-1" });
    map.push("run-1", e);
    expect(map.size()).toBe(1);

    expect(map.pop("run-1")).toEqual(e);
    expect(map.size()).toBe(0);
    expect(map.pop("run-1")).toBeUndefined();
    // Unknown key → undefined (commit path warns + no-ops).
    expect(map.pop("run-never-pushed")).toBeUndefined();
  });

  it("TP-33: FIFO-within-key pop order", () => {
    const map = new InflightMap();
    const first = entry({ llmCallId: "llm-1", runId: "run-1" });
    const second = entry({ llmCallId: "llm-2", runId: "run-1" });
    const third = entry({ llmCallId: "llm-3", runId: "run-1" });
    map.push("run-1", first);
    map.push("run-1", second);
    map.push("run-1", third);

    expect(map.pop("run-1")?.llmCallId).toBe("llm-1");
    expect(map.pop("run-1")?.llmCallId).toBe("llm-2");
    expect(map.pop("run-1")?.llmCallId).toBe("llm-3");
    expect(map.pop("run-1")).toBeUndefined();
  });

  it("TP-34: capacity 10_000 → oldest evicted", () => {
    const map = new InflightMap(); // default capacity 10_000
    for (let i = 0; i < 10_001; i += 1) {
      map.push(`run-${i}`, entry({ llmCallId: `llm-${i}`, runId: `run-${i}` }));
    }
    // Bounded: size never exceeds the capacity.
    expect(map.size()).toBe(10_000);
    // The globally-oldest entry (run-0) was evicted; the newest survives.
    expect(map.pop("run-0")).toBeUndefined();
    expect(map.pop("run-10000")?.llmCallId).toBe("llm-10000");
    expect(map.pop("run-1")?.llmCallId).toBe("llm-1");
  });

  it("TP-35: concurrent runs (distinct runIds) never cross-correlate", () => {
    const map = new InflightMap();
    // Interleaved pushes from two "parallel" runs.
    map.push("run-A", entry({ llmCallId: "llm-A1", runId: "run-A" }));
    map.push("run-B", entry({ llmCallId: "llm-B1", runId: "run-B" }));
    map.push("run-A", entry({ llmCallId: "llm-A2", runId: "run-A" }));
    map.push("run-B", entry({ llmCallId: "llm-B2", runId: "run-B" }));

    // Each run pops ONLY its own entries, in its own FIFO order.
    expect(map.pop("run-B")?.llmCallId).toBe("llm-B1");
    expect(map.pop("run-A")?.llmCallId).toBe("llm-A1");
    expect(map.pop("run-A")?.llmCallId).toBe("llm-A2");
    expect(map.pop("run-B")?.llmCallId).toBe("llm-B2");
    expect(map.size()).toBe(0);
  });

  it("TP-36 (R2 regression): steady-state push/pop keeps internal FIFO bounded (§6.5)", () => {
    // R1 Major 1: pop() marks nodes dead but leaves them in the global FIFO;
    // without lazy compaction a long-lived reserve→commit steady state grows
    // the FIFO by one dead node per cycle, forever. N ≫ capacity cycles must
    // keep internal retention bounded by the compaction threshold (2×cap).
    const capacity = 8;
    const map = new InflightMap(capacity);
    const cycles = 1_000; // N ≫ capacity
    for (let i = 0; i < cycles; i += 1) {
      const key = `run-${i}`;
      map.push(key, entry({ llmCallId: `llm-${i}`, runId: key }));
      expect(map.pop(key)?.llmCallId).toBe(`llm-${i}`);
      // @internal test hook: live + lazily-dead nodes never exceed 2×capacity.
      expect(map.internalFifoLength()).toBeLessThanOrEqual(2 * capacity);
    }
    expect(map.size()).toBe(0);
    expect(map.internalFifoLength()).toBeLessThanOrEqual(2 * capacity);
  });

  // ── COV_D38_04 coverage floor top-up (tests.md §1: inflight.ts 100 %) ──

  it("COV_D38_04: eviction walks PAST lazily-dead FIFO nodes to the oldest LIVE entry", () => {
    const map = new InflightMap(2);
    // Push + pop leaves a dead node at the FIFO head (lazy compaction).
    map.push("run-A", entry({ llmCallId: "llm-A", runId: "run-A" }));
    expect(map.pop("run-A")?.llmCallId).toBe("llm-A");
    // Fill to capacity, then overflow: eviction must skip the dead A node
    // and evict B (the oldest LIVE), keeping C and D.
    map.push("run-B", entry({ llmCallId: "llm-B", runId: "run-B" }));
    map.push("run-C", entry({ llmCallId: "llm-C", runId: "run-C" }));
    map.push("run-D", entry({ llmCallId: "llm-D", runId: "run-D" }));
    expect(map.size()).toBe(2);
    expect(map.pop("run-B")).toBeUndefined();
    expect(map.pop("run-C")?.llmCallId).toBe("llm-C");
    expect(map.pop("run-D")?.llmCallId).toBe("llm-D");
  });

  it("COV_D38_04 (white-box): pop's defensive shift-undefined guard returns undefined", () => {
    // src/inflight.ts pop() guards `queue.shift()` returning undefined for
    // type narrowing; a well-formed queue can never produce it (length is
    // checked first), so the guard is white-box-covered here to honestly
    // meet the 100 %-stmt floor WITHOUT touching src (anti-scope).
    const map = new InflightMap();
    (map as unknown as { queues: Map<string, unknown> }).queues.set("ghost", {
      length: 1,
      shift: () => undefined,
    });
    expect(map.pop("ghost")).toBeUndefined();
    expect(map.size()).toBe(0);
  });
});
