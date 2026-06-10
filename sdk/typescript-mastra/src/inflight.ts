// src/inflight.ts — bounded inflight reserve↔commit correlation (design
// §6.5 / implementation.md §3.3).
//
// [VERIFY-AT-IMPL: V3] PINNED (COV_D38_02, @mastra/core 1.41.0): NO stable
// per-call/per-step correlation id is visible at both `processInputStep`
// and `processLLMResponse` / `processOutputStep`. The hooks share only
// `stepNumber` (request-scoped, not unique across concurrent runs) and the
// per-processor `state` bag; neither is a stable correlation id, and no
// Mastra run id is exposed at the hook surface. The LOCKED §6.5 fallback
// therefore applies: the map is keyed by the adapter-derived `runId` with
// FIFO-within-run pop. Mastra's agent loop is sequential within a run
// (step N+1 starts after step N settles), so the response hook pops the
// oldest open entry for its run; parallel agents/runs have distinct
// `runId`s and never cross-talk.
//
// Global capacity bound 10_000 entries, FIFO eviction (D04 parity) — a hook
// that never fires cannot leak memory unbounded. Popped (dead) nodes are
// lazily compacted out of the global FIFO (see compactIfNeeded) so a
// long-lived steady state of reserve→commit cycles stays bounded too.

import type { PricingFreeze, UnitRef } from "@spendguard/sdk";

export interface InflightEntry {
  decisionId: string;
  reservationId: string;
  runId: string;
  llmCallId: string;
  idempotencyKey: string;
  /** Reserve-time projection — §6.6 commit-estimation fallback. */
  projectedAmountAtomic: string;
  /**
   * Reserve-time unit (the projected claims' `claim[0].unit`) — the commit
   * must tuple-match the reservation (HARDEN_D05_WI; D04 precedent
   * `pending.unit = projectedClaim.unit`). Additive field per the design.md
   * §6.5 dated amendment (2026-06-10, orchestrator-ratified).
   */
  unit: UnitRef;
  /**
   * Reserve-time pricing freeze stash (`opts.pricing`) — the commit must
   * repeat the reservation's freeze tuple (HARDEN_D05_WI; D04 precedent
   * `pending.pricing = opts.pricing`, handler.ts:316/377). Absent → the
   * commit sends the empty tuple. Additive field per the design.md §6.7
   * dated amendment #3 (2026-06-11, orchestrator-ratified — mirrors the
   * amendment-#1 `unit` field).
   */
  pricing?: PricingFreeze;
}

const DEFAULT_CAPACITY = 10_000;

/** Internal node: tracked in both the per-key queue and the global FIFO. */
interface InflightNode {
  key: string;
  entry: InflightEntry;
  live: boolean;
}

export class InflightMap {
  private readonly capacity: number;
  /** Per-key FIFO queues (key: V3 call id — pinned to fallback runId). */
  private readonly queues = new Map<string, InflightNode[]>();
  /** Global insertion order for capacity eviction (lazy-cleaned). */
  private fifo: InflightNode[] = [];
  private liveCount = 0;

  constructor(capacity?: number) {
    this.capacity =
      capacity !== undefined && Number.isInteger(capacity) && capacity > 0
        ? capacity
        : DEFAULT_CAPACITY;
  }

  /** key: V3 call id, else runId (V3 pinned to the runId fallback). */
  push(key: string, entry: InflightEntry): void {
    const node: InflightNode = { key, entry, live: true };
    let queue = this.queues.get(key);
    if (queue === undefined) {
      queue = [];
      this.queues.set(key, queue);
    }
    queue.push(node);
    this.fifo.push(node);
    this.liveCount += 1;
    while (this.liveCount > this.capacity) {
      this.evictOldest();
    }
  }

  /** FIFO within key; deletes the popped entry. Unknown key → undefined. */
  pop(key: string): InflightEntry | undefined {
    const queue = this.queues.get(key);
    if (queue === undefined || queue.length === 0) {
      return undefined;
    }
    const node = queue.shift();
    if (node === undefined) {
      return undefined;
    }
    if (queue.length === 0) {
      this.queues.delete(key);
    }
    node.live = false;
    this.liveCount -= 1;
    this.compactIfNeeded();
    return node.entry;
  }

  size(): number {
    return this.liveCount;
  }

  /**
   * Test-only hook: internal global-FIFO length (live + lazily-dead nodes).
   * Pins the §6.5 "cannot leak memory unbounded" invariant — see
   * tests/inflight.test.ts TP-36.
   * @internal
   */
  internalFifoLength(): number {
    return this.fifo.length;
  }

  /**
   * Lazy compaction (§6.5 memory bound): `pop()` marks nodes dead without
   * removing them from the global FIFO, so a long-lived steady state of
   * reserve→commit cycles would otherwise grow `fifo` by one dead node per
   * reserve forever. Rebuild from live nodes once dead nodes dominate.
   */
  private compactIfNeeded(): void {
    if (this.fifo.length > 2 * this.capacity) {
      this.fifo = this.fifo.filter((node) => node.live);
    }
  }

  /** Drop the globally-oldest live entry (capacity bound, D04 parity). */
  private evictOldest(): void {
    while (this.fifo.length > 0) {
      const node = this.fifo.shift();
      if (node === undefined || !node.live) {
        continue;
      }
      node.live = false;
      this.liveCount -= 1;
      const queue = this.queues.get(node.key);
      if (queue !== undefined) {
        const idx = queue.indexOf(node);
        if (idx >= 0) {
          queue.splice(idx, 1);
        }
        if (queue.length === 0) {
          this.queues.delete(node.key);
        }
      }
      return;
    }
  }
}
