// TP-25..TP-26 — SSE helper (tests.md §5; design.md §7 locked framing).
import { describe, expect, it } from "vitest";
import {
  buildBudgetSnapshot,
  buildDecisionDenied,
  buildReservationCommitted,
  buildReservationCreated,
  buildReservationReleased,
  canonicalEventJson,
  encodeSse,
} from "../src/index.js";
import type { AgUiEmit, SpendGuardAgUiEvent } from "../src/index.js";
import {
  COMMITTED_MAX,
  CREATED_MAX,
  DENIED_MAX,
  RELEASED_MAX,
  SNAPSHOT_MAX,
  TS_MS,
} from "./_support/vectors.js";

const EVENTS: ReadonlyArray<SpendGuardAgUiEvent> = [
  buildBudgetSnapshot(SNAPSHOT_MAX, { timestampMs: TS_MS }),
  buildReservationCreated(CREATED_MAX, { timestampMs: TS_MS }),
  buildReservationCommitted(COMMITTED_MAX),
  buildReservationReleased(RELEASED_MAX),
  buildDecisionDenied(DENIED_MAX, { timestampMs: 0 }),
];

describe("TP-25 locked framing", () => {
  it.each(EVENTS.map((e) => [e.name, e] as const))(
    '%s: encodeSse(e) === "data: " + canonicalEventJson(e) + "\\n\\n"',
    (_name, evt) => {
      expect(encodeSse(evt)).toBe(`data: ${canonicalEventJson(evt)}\n\n`);
    },
  );
});

describe("TP-26 transport safety", () => {
  it.each(EVENTS.map((e) => [e.name, e] as const))(
    "%s: frame contains no interior newline",
    (_name, evt) => {
      const frame = encodeSse(evt);
      expect(frame.endsWith("\n\n")).toBe(true);
      // The only newlines are the two terminators.
      expect(frame.indexOf("\n")).toBe(frame.length - 2);
    },
  );

  it("AgUiEmit accepts sync and async emitters (type-level + runtime)", async () => {
    const seen: string[] = [];
    const syncEmit: AgUiEmit = (e) => {
      seen.push(e.name);
    };
    const asyncEmit: AgUiEmit = async (e) => {
      seen.push(e.name);
    };
    syncEmit(EVENTS[0] as SpendGuardAgUiEvent);
    await asyncEmit(EVENTS[1] as SpendGuardAgUiEvent);
    expect(seen).toEqual(["spendguard.budget.snapshot", "spendguard.reservation.created"]);
  });
});
