// TP-01..TP-13 — builder behavior (tests.md §2).
import { afterEach, describe, expect, it } from "vitest";
import {
  AgUiEventValidationError,
  SPENDGUARD_AG_UI_EVENT_NAMES,
  buildBudgetSnapshot,
  buildDecisionDenied,
  buildReservationCommitted,
  buildReservationCreated,
  buildReservationReleased,
  canonicalEventJson,
} from "../src/index.js";
import type { BuildContext, SpendGuardAgUiEvent } from "../src/index.js";
import {
  COMMITTED_MAX,
  COMMITTED_MIN,
  CREATED_MAX,
  CREATED_MIN,
  DENIED_MAX,
  DENIED_MIN,
  RELEASED_MAX,
  RELEASED_MIN,
  SNAPSHOT_MAX,
  SNAPSHOT_MIN,
  TS_MS,
  deepFreeze,
} from "./_support/vectors.js";

type AnyBuilder = (input: never, ctx?: BuildContext) => SpendGuardAgUiEvent;

const ALL: ReadonlyArray<[string, AnyBuilder, unknown, string]> = [
  [
    "buildBudgetSnapshot",
    buildBudgetSnapshot as AnyBuilder,
    SNAPSHOT_MAX,
    "spendguard.budget.snapshot",
  ],
  [
    "buildReservationCreated",
    buildReservationCreated as AnyBuilder,
    CREATED_MAX,
    "spendguard.reservation.created",
  ],
  [
    "buildReservationCommitted",
    buildReservationCommitted as AnyBuilder,
    COMMITTED_MAX,
    "spendguard.reservation.committed",
  ],
  [
    "buildReservationReleased",
    buildReservationReleased as AnyBuilder,
    RELEASED_MAX,
    "spendguard.reservation.released",
  ],
  [
    "buildDecisionDenied",
    buildDecisionDenied as AnyBuilder,
    DENIED_MAX,
    "spendguard.decision.denied",
  ],
];

describe("TP-01 envelope + vocabulary lock", () => {
  it.each(ALL)("%s returns type CUSTOM and the exact §5.2 name", (_n, build, input, name) => {
    const evt = build(input as never);
    expect(evt.type).toBe("CUSTOM");
    expect(evt.name).toBe(name);
  });

  it("the five-name constant matches design.md §5.2 byte-for-byte, no sixth name", () => {
    expect(SPENDGUARD_AG_UI_EVENT_NAMES).toEqual({
      budgetSnapshot: "spendguard.budget.snapshot",
      reservationCreated: "spendguard.reservation.created",
      reservationCommitted: "spendguard.reservation.committed",
      reservationReleased: "spendguard.reservation.released",
      decisionDenied: "spendguard.decision.denied",
    });
    expect(Object.keys(SPENDGUARD_AG_UI_EVENT_NAMES)).toHaveLength(5);
  });
});

describe("TP-02 purity / determinism", () => {
  it.each(ALL)("%s: deep-equal inputs yield deep-equal events", (_n, build, input) => {
    const a = build(structuredClone(input) as never, { timestampMs: TS_MS });
    const b = build(structuredClone(input) as never, { timestampMs: TS_MS });
    expect(a).toEqual(b);
  });

  it.each(ALL)("%s: 100 repeated calls yield identical canonical bytes", (_n, build, input) => {
    const outputs = new Set<string>();
    for (let i = 0; i < 100; i++) {
      outputs.add(canonicalEventJson(build(input as never, { timestampMs: TS_MS })));
    }
    expect(outputs.size).toBe(1);
  });
});

describe("TP-03 clock-free", () => {
  const realNow = Date.now;
  afterEach(() => {
    Date.now = realNow;
  });

  it("builders succeed with Date.now monkeypatched to throw", () => {
    Date.now = () => {
      throw new Error("clock read — purity violation (design.md §11.3)");
    };
    for (const [, build, input] of ALL) {
      const evt = build(input as never, { timestampMs: TS_MS });
      expect(canonicalEventJson(evt)).toContain('"type":"CUSTOM"');
    }
  });
});

describe("TP-04 envelope timestamp option", () => {
  it("timestampMs provided → envelope timestamp equals it exactly", () => {
    const evt = buildBudgetSnapshot(SNAPSHOT_MIN, { timestampMs: TS_MS });
    expect(evt.timestamp).toBe(TS_MS);
  });

  it("timestampMs: 0 is a valid epoch ms and IS emitted (0 ≠ absent)", () => {
    const evt = buildBudgetSnapshot(SNAPSHOT_MIN, { timestampMs: 0 });
    expect("timestamp" in evt).toBe(true);
    expect(evt.timestamp).toBe(0);
  });

  it("omitted → timestamp key ABSENT (not null, not 0)", () => {
    for (const evt of [buildBudgetSnapshot(SNAPSHOT_MIN), buildBudgetSnapshot(SNAPSHOT_MIN, {})]) {
      expect("timestamp" in evt).toBe(false);
      expect(Object.keys(evt).sort()).toEqual(["name", "type", "value"]);
    }
  });

  it("non-integer / negative / -0 timestampMs throws", () => {
    for (const bad of [1.5, -1, -0, Number.NaN, 2 ** 53]) {
      expect(() => buildBudgetSnapshot(SNAPSHOT_MIN, { timestampMs: bad })).toThrow(
        AgUiEventValidationError,
      );
    }
  });
});

describe("TP-05 budget.snapshot — §5.3 key set", () => {
  it("payload is exactly the §5.3 key set (required-only)", () => {
    const evt = buildBudgetSnapshot(SNAPSHOT_MIN);
    expect(Object.keys(evt.value).sort()).toEqual([
      "as_of",
      "budget_id",
      "remaining_atomic",
      "reserved_atomic",
      "schema_version",
      "spent_atomic",
      "unit",
      "window_instance_id",
    ]);
    expect(evt.value.schema_version).toBe("1");
  });

  it("payload is exactly the §5.3 key set (+unit_id when supplied)", () => {
    const evt = buildBudgetSnapshot(SNAPSHOT_MAX);
    expect(Object.keys(evt.value).sort()).toEqual([
      "as_of",
      "budget_id",
      "remaining_atomic",
      "reserved_atomic",
      "schema_version",
      "spent_atomic",
      "unit",
      "unit_id",
      "window_instance_id",
    ]);
  });
});

describe("TP-06 reservation.created — §5.4 key set + ASP decision enum", () => {
  it("payload matches the §5.4 key set", () => {
    const min = buildReservationCreated(CREATED_MIN);
    expect(Object.keys(min.value).sort()).toEqual([
      "amount_atomic_reserved",
      "budget_id",
      "decision",
      "decision_id",
      "event_time",
      "reservation_id",
      "schema_version",
      "ttl_expires_at",
      "unit",
      "window_instance_id",
    ]);
    const max = buildReservationCreated(CREATED_MAX);
    expect(Object.keys(max.value).sort()).toEqual([
      "amount_atomic_reserved",
      "budget_id",
      "decision",
      "decision_id",
      "event_time",
      "llm_call_id",
      "matched_rule_ids",
      "reason_codes",
      "reservation_id",
      "run_id",
      "schema_version",
      "ttl_expires_at",
      "unit",
      "unit_id",
      "window_instance_id",
    ]);
  });

  it('decision passes through "ALLOW" and "ALLOW_WITH_CAPS" verbatim', () => {
    for (const decision of ["ALLOW", "ALLOW_WITH_CAPS"] as const) {
      const evt = buildReservationCreated({ ...CREATED_MIN, decision });
      expect(evt.value.decision).toBe(decision);
    }
  });

  it("a third decision value throws", () => {
    expect(() =>
      buildReservationCreated({ ...CREATED_MIN, decision: "DENY" as never }),
    ).toThrowError(AgUiEventValidationError);
  });
});

describe("TP-07 reservation.committed — §5.5 key set + outcome enum lock", () => {
  it("payload matches the §5.5 key set", () => {
    const min = buildReservationCommitted(COMMITTED_MIN);
    expect(Object.keys(min.value).sort()).toEqual([
      "amount_atomic_estimated",
      "budget_id",
      "decision_id",
      "event_time",
      "outcome",
      "reservation_id",
      "schema_version",
      "unit",
      "window_instance_id",
    ]);
    const max = buildReservationCommitted(COMMITTED_MAX);
    expect(Object.keys(max.value).sort()).toEqual([
      "amount_atomic_estimated",
      "amount_atomic_observed",
      "budget_id",
      "decision_id",
      "event_time",
      "llm_call_id",
      "outcome",
      "reservation_id",
      "run_id",
      "schema_version",
      "unit",
      "unit_id",
      "window_instance_id",
    ]);
  });

  it("all four outcome values are accepted verbatim", () => {
    for (const outcome of ["SUCCESS", "PROVIDER_ERROR", "CLIENT_TIMEOUT", "RUN_ABORTED"] as const) {
      const evt = buildReservationCommitted({ ...COMMITTED_MIN, outcome });
      expect(evt.value.outcome).toBe(outcome);
    }
  });

  it("a fifth outcome value throws", () => {
    let thrown: unknown;
    try {
      buildReservationCommitted({ ...COMMITTED_MIN, outcome: "CANCELLED" as never });
    } catch (e) {
      thrown = e;
    }
    expect(thrown).toBeInstanceOf(AgUiEventValidationError);
    expect((thrown as AgUiEventValidationError).field).toBe("outcome");
  });
});

describe("TP-08 amount_atomic_estimated vs amount_atomic_observed (ASP-delta naming)", () => {
  it("emits amount_atomic_estimated; amount_atomic_observed ABSENT unless supplied", () => {
    const evt = buildReservationCommitted(COMMITTED_MIN);
    expect(evt.value.amount_atomic_estimated).toBe("950000");
    expect("amount_atomic_observed" in evt.value).toBe(false);
  });

  it("amount_atomic_observed present verbatim when supplied", () => {
    const evt = buildReservationCommitted(COMMITTED_MAX);
    expect(evt.value.amount_atomic_observed).toBe("940123");
  });
});

describe("TP-09 reservation.released — §5.6 key set", () => {
  it("payload matches the §5.6 key set", () => {
    const min = buildReservationReleased(RELEASED_MIN);
    expect(Object.keys(min.value).sort()).toEqual([
      "event_time",
      "reason_codes",
      "reservation_id",
      "schema_version",
    ]);
    const max = buildReservationReleased(RELEASED_MAX);
    expect(Object.keys(max.value).sort()).toEqual([
      "decision_id",
      "event_time",
      "ledger_transaction_id",
      "llm_call_id",
      "reason_codes",
      "reservation_id",
      "run_id",
      "schema_version",
    ]);
  });

  it("Draft-01 §4 example reason_codes round-trip verbatim", () => {
    const evt = buildReservationReleased({
      ...RELEASED_MIN,
      reasonCodes: ["provider_error", "client_timeout", "run_cancelled"],
    });
    expect(evt.value.reason_codes).toEqual(["provider_error", "client_timeout", "run_cancelled"]);
  });
});

describe("TP-10 decision.denied — injected DENY + §5.7 key set", () => {
  it('injects literal decision: "DENY" regardless of deniedKind', () => {
    for (const deniedKind of [
      "DENY",
      "STOP",
      "STOP_RUN_PROJECTION",
      "SKIP",
      "APPROVAL_REQUIRED",
    ] as const) {
      const reasonCodes =
        deniedKind === "APPROVAL_REQUIRED" ? ["approval_required"] : ["BUDGET_EXHAUSTED"];
      const evt = buildDecisionDenied({ ...DENIED_MIN, deniedKind, reasonCodes });
      expect(evt.value.decision).toBe("DENY");
    }
  });

  it("payload matches the §5.7 key set", () => {
    const min = buildDecisionDenied(DENIED_MIN);
    expect(Object.keys(min.value).sort()).toEqual([
      "decision",
      "decision_id",
      "denied_kind",
      "event_time",
      "reason_codes",
      "schema_version",
    ]);
    const max = buildDecisionDenied(DENIED_MAX);
    expect(Object.keys(max.value).sort()).toEqual([
      "budget_id",
      "decision",
      "decision_id",
      "denied_kind",
      "event_time",
      "llm_call_id",
      "matched_rule_ids",
      "reason_codes",
      "run_id",
      "schema_version",
      "unit",
      "unit_id",
      "window_instance_id",
    ]);
  });
});

describe("TP-11 denied_kind taxonomy lock", () => {
  it("all five deniedKind values are accepted verbatim as denied_kind", () => {
    for (const deniedKind of [
      "DENY",
      "STOP",
      "STOP_RUN_PROJECTION",
      "SKIP",
      "APPROVAL_REQUIRED",
    ] as const) {
      const reasonCodes =
        deniedKind === "APPROVAL_REQUIRED" ? ["approval_required"] : ["BUDGET_EXHAUSTED"];
      const evt = buildDecisionDenied({ ...DENIED_MIN, deniedKind, reasonCodes });
      expect(evt.value.denied_kind).toBe(deniedKind);
    }
  });

  it("a sixth deniedKind value throws", () => {
    let thrown: unknown;
    try {
      buildDecisionDenied({ ...DENIED_MIN, deniedKind: "PAUSED" as never });
    } catch (e) {
      thrown = e;
    }
    expect(thrown).toBeInstanceOf(AgUiEventValidationError);
    expect((thrown as AgUiEventValidationError).field).toBe("denied_kind");
  });
});

describe("TP-12 no side effects", () => {
  it.each(ALL)("%s: deep-frozen input is not mutated", (_n, build, input) => {
    const frozen = deepFreeze(structuredClone(input));
    const snapshot = JSON.stringify(frozen);
    build(frozen as never, { timestampMs: TS_MS });
    expect(JSON.stringify(frozen)).toBe(snapshot);
  });

  it.each(ALL)("%s: returned event is frozen", (_n, build, input) => {
    const evt = build(input as never, { timestampMs: TS_MS });
    expect(Object.isFrozen(evt)).toBe(true);
    expect(Object.isFrozen(evt.value)).toBe(true);
    for (const v of Object.values(evt.value)) {
      if (Array.isArray(v)) {
        expect(Object.isFrozen(v)).toBe(true);
      }
    }
  });

  it("later caller mutation of an input array cannot reach the event", () => {
    const reasonCodes = ["provider_error"];
    const evt = buildReservationReleased({ ...RELEASED_MIN, reasonCodes });
    reasonCodes.push("mutated_after_build");
    expect(evt.value.reason_codes).toEqual(["provider_error"]);
  });
});

describe("TP-13 created optional arrays — omit-if-absent/empty", () => {
  it("non-empty arrays are emitted verbatim in caller order", () => {
    const evt = buildReservationCreated({
      ...CREATED_MIN,
      reasonCodes: ["z_code", "a_code"],
      matchedRuleIds: ["rule-9", "rule-1"],
    });
    expect(evt.value.reason_codes).toEqual(["z_code", "a_code"]);
    expect(evt.value.matched_rule_ids).toEqual(["rule-9", "rule-1"]);
  });

  it("empty array → key ABSENT", () => {
    const evt = buildReservationCreated({ ...CREATED_MIN, reasonCodes: [], matchedRuleIds: [] });
    expect("reason_codes" in evt.value).toBe(false);
    expect("matched_rule_ids" in evt.value).toBe(false);
  });

  it("omitted → key ABSENT", () => {
    const evt = buildReservationCreated(CREATED_MIN);
    expect("reason_codes" in evt.value).toBe(false);
    expect("matched_rule_ids" in evt.value).toBe(false);
  });
});
