// TP-14..TP-19 — validation (tests.md §3).
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  AgUiEventValidationError,
  buildBudgetSnapshot,
  buildDecisionDenied,
  buildReservationCommitted,
  buildReservationCreated,
  buildReservationReleased,
} from "../src/index.js";
import {
  optionalEntry,
  requireAtomic,
  requireNonEmpty,
  requireRfc3339,
  requireSafeInteger,
  requireStringArray,
} from "../src/validate.js";
import {
  COMMITTED_MIN,
  CREATED_MIN,
  DENIED_MIN,
  RELEASED_MIN,
  SNAPSHOT_MIN,
} from "./_support/vectors.js";

const CORPUS_PATH = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "../../fixtures/cross-language/ag_ui_v1.json",
);

describe("TP-14 unit_id omission (P0, HARDEN_D05_UR)", () => {
  const cases = [
    {
      name: "snapshot",
      build: (unitId: string | undefined) =>
        buildBudgetSnapshot({ ...SNAPSHOT_MIN, unitId } as never),
    },
    {
      name: "created",
      build: (unitId: string | undefined) =>
        buildReservationCreated({ ...CREATED_MIN, unitId } as never),
    },
    {
      name: "committed",
      build: (unitId: string | undefined) =>
        buildReservationCommitted({ ...COMMITTED_MIN, unitId } as never),
    },
    {
      name: "denied",
      build: (unitId: string | undefined) =>
        buildDecisionDenied({ ...DENIED_MIN, unitId } as never),
    },
  ] as const;

  it.each(cases)("$name: unitId undefined → no unit_id key", ({ build }) => {
    expect("unit_id" in build(undefined).value).toBe(false);
  });

  it.each(cases)('$name: unitId "" → no unit_id key (empty ≡ absent)', ({ build }) => {
    expect("unit_id" in build("").value).toBe(false);
  });

  it.each(cases)("$name: non-empty unitId emitted verbatim", ({ build }) => {
    expect(build("0197a001-2222-7000-8000-0000000unit1").value.unit_id).toBe(
      "0197a001-2222-7000-8000-0000000unit1",
    );
  });

  it('corpus-wide: "unit_id":"" never appears in ag_ui_v1.json (acceptance A3.4)', () => {
    expect(existsSync(CORPUS_PATH)).toBe(true);
    const raw = readFileSync(CORPUS_PATH, "utf8");
    expect(raw.includes('"unit_id":""')).toBe(false);
    // The escaped form inside expected_canonical_json strings:
    expect(raw.includes('\\"unit_id\\":\\"\\"')).toBe(false);
  });
});

describe("TP-15 empty required string → AgUiEventValidationError naming the payload key", () => {
  // [builder label, build with one camelCase field blanked, expected payload key]
  const cases: ReadonlyArray<[string, () => unknown, string]> = [
    [
      "snapshot.budget_id",
      () => buildBudgetSnapshot({ ...SNAPSHOT_MIN, budgetId: "" }),
      "budget_id",
    ],
    [
      "snapshot.window_instance_id",
      () => buildBudgetSnapshot({ ...SNAPSHOT_MIN, windowInstanceId: "" }),
      "window_instance_id",
    ],
    ["snapshot.unit", () => buildBudgetSnapshot({ ...SNAPSHOT_MIN, unit: "" }), "unit"],
    [
      "snapshot.remaining_atomic",
      () => buildBudgetSnapshot({ ...SNAPSHOT_MIN, remainingAtomic: "" }),
      "remaining_atomic",
    ],
    [
      "snapshot.reserved_atomic",
      () => buildBudgetSnapshot({ ...SNAPSHOT_MIN, reservedAtomic: "" }),
      "reserved_atomic",
    ],
    [
      "snapshot.spent_atomic",
      () => buildBudgetSnapshot({ ...SNAPSHOT_MIN, spentAtomic: "" }),
      "spent_atomic",
    ],
    ["snapshot.as_of", () => buildBudgetSnapshot({ ...SNAPSHOT_MIN, asOf: "" }), "as_of"],
    [
      "created.decision_id",
      () => buildReservationCreated({ ...CREATED_MIN, decisionId: "" }),
      "decision_id",
    ],
    [
      "created.reservation_id",
      () => buildReservationCreated({ ...CREATED_MIN, reservationId: "" }),
      "reservation_id",
    ],
    [
      "created.budget_id",
      () => buildReservationCreated({ ...CREATED_MIN, budgetId: "" }),
      "budget_id",
    ],
    [
      "created.window_instance_id",
      () => buildReservationCreated({ ...CREATED_MIN, windowInstanceId: "" }),
      "window_instance_id",
    ],
    ["created.unit", () => buildReservationCreated({ ...CREATED_MIN, unit: "" }), "unit"],
    [
      "created.amount_atomic_reserved",
      () => buildReservationCreated({ ...CREATED_MIN, amountAtomicReserved: "" }),
      "amount_atomic_reserved",
    ],
    [
      "created.ttl_expires_at",
      () => buildReservationCreated({ ...CREATED_MIN, ttlExpiresAt: "" }),
      "ttl_expires_at",
    ],
    [
      "created.event_time",
      () => buildReservationCreated({ ...CREATED_MIN, eventTime: "" }),
      "event_time",
    ],
    [
      "committed.decision_id",
      () => buildReservationCommitted({ ...COMMITTED_MIN, decisionId: "" }),
      "decision_id",
    ],
    [
      "committed.reservation_id",
      () => buildReservationCommitted({ ...COMMITTED_MIN, reservationId: "" }),
      "reservation_id",
    ],
    [
      "committed.budget_id",
      () => buildReservationCommitted({ ...COMMITTED_MIN, budgetId: "" }),
      "budget_id",
    ],
    [
      "committed.window_instance_id",
      () => buildReservationCommitted({ ...COMMITTED_MIN, windowInstanceId: "" }),
      "window_instance_id",
    ],
    ["committed.unit", () => buildReservationCommitted({ ...COMMITTED_MIN, unit: "" }), "unit"],
    [
      "committed.amount_atomic_estimated",
      () => buildReservationCommitted({ ...COMMITTED_MIN, amountAtomicEstimated: "" }),
      "amount_atomic_estimated",
    ],
    [
      "committed.event_time",
      () => buildReservationCommitted({ ...COMMITTED_MIN, eventTime: "" }),
      "event_time",
    ],
    [
      "released.reservation_id",
      () => buildReservationReleased({ ...RELEASED_MIN, reservationId: "" }),
      "reservation_id",
    ],
    [
      "released.event_time",
      () => buildReservationReleased({ ...RELEASED_MIN, eventTime: "" }),
      "event_time",
    ],
    [
      "denied.decision_id",
      () => buildDecisionDenied({ ...DENIED_MIN, decisionId: "" }),
      "decision_id",
    ],
    [
      "denied.event_time",
      () => buildDecisionDenied({ ...DENIED_MIN, eventTime: "" }),
      "event_time",
    ],
  ];

  it.each(cases)("%s", (_label, run, payloadKey) => {
    let thrown: unknown;
    try {
      run();
    } catch (e) {
      thrown = e;
    }
    expect(thrown).toBeInstanceOf(AgUiEventValidationError);
    expect((thrown as AgUiEventValidationError).field).toBe(payloadKey);
  });
});

describe("TP-16 requireAtomic — atomic decimal-string rule", () => {
  it.each(["", "-1", "1.5", "01", "1e3", " 1", "+1"])("rejects %j", (bad) => {
    expect(() => requireAtomic("amount_atomic_reserved", bad)).toThrowError(
      AgUiEventValidationError,
    );
  });

  it("rejects non-strings", () => {
    expect(() => requireAtomic("amount_atomic_reserved", 100 as never)).toThrowError(
      AgUiEventValidationError,
    );
  });

  it.each(["0", "1", "100000", "1234567890123456789012345678901234567890"])(
    "accepts %j (incl. 40-digit string)",
    (good) => {
      expect(requireAtomic("amount_atomic_reserved", good)).toBe(good);
      expect(good === "1234567890123456789012345678901234567890" ? good.length : 40).toBe(40);
    },
  );
});

describe("TP-17 RFC 3339 format gate", () => {
  it.each(["2026-06-10", "yesterday", ""])("rejects %j", (bad) => {
    expect(() => requireRfc3339("event_time", bad)).toThrowError(AgUiEventValidationError);
  });

  it("rejects epoch ints (non-strings)", () => {
    expect(() => requireRfc3339("event_time", 1765843200 as never)).toThrowError(
      AgUiEventValidationError,
    );
  });

  it.each(["2026-06-10T07:59:58Z", "2026-06-10T07:59:58.123+08:00"])("accepts %j", (good) => {
    expect(requireRfc3339("event_time", good)).toBe(good);
  });
});

describe("TP-18 denied reason taxonomy (ASP approval mapping)", () => {
  it("reasonCodes: [] throws", () => {
    let thrown: unknown;
    try {
      buildDecisionDenied({ ...DENIED_MIN, reasonCodes: [] });
    } catch (e) {
      thrown = e;
    }
    expect(thrown).toBeInstanceOf(AgUiEventValidationError);
    expect((thrown as AgUiEventValidationError).field).toBe("reason_codes");
  });

  it('APPROVAL_REQUIRED without "approval_required" throws citing ASP Draft-01 §2', () => {
    let thrown: unknown;
    try {
      buildDecisionDenied({
        ...DENIED_MIN,
        deniedKind: "APPROVAL_REQUIRED",
        reasonCodes: ["needs_human"],
      });
    } catch (e) {
      thrown = e;
    }
    expect(thrown).toBeInstanceOf(AgUiEventValidationError);
    expect((thrown as AgUiEventValidationError).field).toBe("reason_codes");
    expect((thrown as AgUiEventValidationError).message).toContain("ASP Draft-01 §2");
  });

  it("with it present → builds, no silent append, array order preserved", () => {
    const evt = buildDecisionDenied({
      ...DENIED_MIN,
      deniedKind: "APPROVAL_REQUIRED",
      reasonCodes: ["zz_costly_model", "approval_required", "aa_over_threshold"],
    });
    expect(evt.value.reason_codes).toEqual([
      "zz_costly_model",
      "approval_required",
      "aa_over_threshold",
    ]);
  });
});

describe("TP-19 per-event array arity", () => {
  it("released reasonCodes: [] throws (≥ 1 required)", () => {
    let thrown: unknown;
    try {
      buildReservationReleased({ ...RELEASED_MIN, reasonCodes: [] });
    } catch (e) {
      thrown = e;
    }
    expect(thrown).toBeInstanceOf(AgUiEventValidationError);
    expect((thrown as AgUiEventValidationError).field).toBe("reason_codes");
  });

  it("created reasonCodes may be omitted", () => {
    const evt = buildReservationCreated(CREATED_MIN);
    expect("reason_codes" in evt.value).toBe(false);
  });

  it("array entries must be non-empty strings", () => {
    expect(() => buildReservationReleased({ ...RELEASED_MIN, reasonCodes: [""] })).toThrowError(
      AgUiEventValidationError,
    );
    expect(() =>
      buildReservationCreated({ ...CREATED_MIN, matchedRuleIds: ["ok", 7 as never] }),
    ).toThrowError(AgUiEventValidationError);
    expect(() => requireStringArray("reason_codes", "not-array", { minLen: 1 })).toThrowError(
      AgUiEventValidationError,
    );
  });

  it("validator edges: requireNonEmpty non-string, requireSafeInteger, optionalEntry", () => {
    expect(() => requireNonEmpty("budget_id", 42 as never)).toThrowError(AgUiEventValidationError);
    expect(requireSafeInteger("timestamp", 0)).toBe(0);
    expect(() => requireSafeInteger("timestamp", "0" as never)).toThrowError(
      AgUiEventValidationError,
    );
    expect(optionalEntry("run_id", "r-1")).toEqual({ run_id: "r-1" });
    expect(optionalEntry("run_id", "")).toEqual({});
    expect(optionalEntry("run_id", undefined)).toEqual({});
  });
});
