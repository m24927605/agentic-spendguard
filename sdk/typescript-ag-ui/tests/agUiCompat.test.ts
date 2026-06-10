// TP-28..TP-29 — pinned @ag-ui/core compat (tests.md §7; design.md §10.3).
//
// devDep pinned EXACTLY: @ag-ui/core@0.0.56 (the npm dist-tag `latest` at
// resolution time, 2026-06-10). A compat failure on a NEWER AG-UI version is
// a P1 maintenance finding — it must NOT move the locked wire shape
// (design.md §10.3); `schema_version` in every payload is the consumer lever.
//
// [VERIFY-AT-IMPL resolved 2026-06-10 — CustomEvent type path + runtime
// schema existence (tests.md TP-28/TP-29 markers)]
//   - Type path: `import("@ag-ui/core").CustomEvent` — exported as
//     `type CustomEvent = z.infer<typeof CustomEventSchema>`.
//   - Runtime schema: `CustomEventSchema` IS exported (zod object:
//     { type: literal EventType.CUSTOM, name: string, value: any } with
//     optional `timestamp`/`rawEvent`), so TP-29 uses the PRIMARY path
//     (runtime parse), not the key-set fallback.
//   - TS string enums are nominal: the literal type "CUSTOM" is not
//     assignable to the enum member type `EventType.CUSTOM` even though the
//     runtime value is the identical string "CUSTOM" (verified: a direct
//     `const e: CustomEvent = built` annotation fails ONLY on the `type`
//     key). Structural assignability is therefore asserted as (a) compile-
//     time: every field EXCEPT the enum-nominal `type` tag via the re-tag
//     below, and (b) runtime: `EventType.CUSTOM === built.type` proving the
//     re-tag is a no-op on the wire.
import type { CustomEvent } from "@ag-ui/core";
import { CustomEventSchema, EventType } from "@ag-ui/core";
import { describe, expect, it } from "vitest";
import {
  buildBudgetSnapshot,
  buildDecisionDenied,
  buildReservationCommitted,
  buildReservationCreated,
  buildReservationReleased,
} from "../src/index.js";
import type { SpendGuardAgUiEvent } from "../src/index.js";
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
  buildReservationCommitted(COMMITTED_MAX, { timestampMs: TS_MS }),
  buildReservationReleased(RELEASED_MAX),
  buildDecisionDenied(DENIED_MAX),
];

describe("TP-28 type-level assignability to the pinned @ag-ui/core CustomEvent", () => {
  it("SpendGuardAgUiEvent is structurally assignable (modulo TS enum nominality on `type`)", () => {
    const built = buildDecisionDenied(DENIED_MAX, { timestampMs: TS_MS });
    // Compile-time: if any field other than the enum-nominal `type` tag
    // were incompatible (name, value, timestamp, or a missing/extra
    // envelope key), this annotation would not compile under the exact pin.
    const compat: CustomEvent = { ...built, type: EventType.CUSTOM };
    expect(compat.name).toBe(built.name);
    // Runtime: the enum member IS the literal string "CUSTOM", so the
    // re-tag above changes zero bytes on the wire.
    expect(EventType.CUSTOM).toBe(built.type);
  });
});

describe("TP-29 runtime parse through the pinned CustomEventSchema", () => {
  it.each(EVENTS.map((e) => [e.name, e] as const))("%s parses", (_name, evt) => {
    const parsed = CustomEventSchema.parse(evt);
    expect(parsed.type).toBe("CUSTOM");
    expect(parsed.name).toBe(evt.name);
    expect(parsed.value).toEqual(evt.value);
    if ("timestamp" in evt) {
      expect(parsed.timestamp).toBe(evt.timestamp);
    }
    // The envelope key set stays {type, name, value} + optional timestamp —
    // rawEvent is never emitted (design.md §5.1).
    expect("rawEvent" in evt).toBe(false);
  });
});
