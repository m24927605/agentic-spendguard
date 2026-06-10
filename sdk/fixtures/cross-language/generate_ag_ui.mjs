// Reference generator for `sdk/fixtures/cross-language/ag_ui_v1.json` — the
// D39 AG-UI spend-event cross-language fixture corpus (design.md §7;
// tests.md §6).
//
// The TypeScript implementation is the REFERENCE (the corpus is minted in
// slice COV_D39_01 from the built `@spendguard/ag-ui` dist builders); the
// Python mirror (slice COV_D39_02) then consumes the SAME file byte-for-byte
// without co-evolution.
//
// Run (from this directory, after `pnpm run build` in sdk/typescript-ag-ui):
//
//     node generate_ag_ui.mjs
//
// Invariants (D05 corpus discipline; review-standards §4.3):
//   - ag_ui_v1.json MUST NEVER be edited in place once committed. A change
//     in `expected_canonical_json` for an existing fixture means the locked
//     §7 canonical rule or a §5 payload schema moved — that is a design.md
//     revision, not a corpus edit. Mint ag_ui_v2.json instead.
//   - This generator therefore refuses to overwrite an existing
//     ag_ui_v1.json unless --force is passed (slice-1 minting only).
//   - Output is fully deterministic: fixed vectors, fixed GENERATED_AT
//     (the slice-1 minting date), pure builders, no clocks, no RNG.
//
// Vector matrix (tests.md §6, >= 20 vectors): minimal + maximal per builder;
// unit_id absent vs present; timestamp_ms absent vs present incl.
// `timestamp_ms: 0` ("0 ≠ absent"); Unicode set (CJK + emoji + astral in
// reason_codes, U+001F in a matched_rule_ids entry); one vector per
// denied_kind (5) incl. APPROVAL_REQUIRED + "approval_required"; one per
// committed outcome (4) plus one with amount_atomic_observed; a 40-digit
// remaining_atomic.
import { existsSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  VERSION,
  buildBudgetSnapshot,
  buildDecisionDenied,
  buildReservationCommitted,
  buildReservationCreated,
  buildReservationReleased,
  canonicalEventJson,
  encodeSse,
} from "../../typescript-ag-ui/dist/index.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT_PATH = join(HERE, "ag_ui_v1.json");

// Fixed minting date — deterministic output (slice COV_D39_01).
const GENERATED_AT = "2026-06-10";

const BUILDERS = {
  buildBudgetSnapshot,
  buildReservationCreated,
  buildReservationCommitted,
  buildReservationReleased,
  buildDecisionDenied,
};

// ── Shared deterministic IDs (substrate-shaped inputs; D39 mints nothing) ──
const BUDGET_ID = "budget-dev-monthly";
const WINDOW_ID = "0197a001-0000-7000-8000-00000000win1";
const UNIT_ID = "0197a001-2222-7000-8000-0000000unit1";
const DECISION_ID = "0197a001-aaaa-7000-8000-000000000d01";
const RESERVATION_ID = "0197a001-bbbb-7000-8000-000000000r01";
const RUN_ID = "0197a001-cccc-7000-8000-00000000run1";
const LLM_CALL_ID = "0197a001-dddd-7000-8000-0000000call1";
const LEDGER_TX_ID = "0197a001-eeee-7000-8000-000000000tx1";
const DENY_DECISION_ID = "0197a001-ffff-7000-8000-000000000d02";
const TS_MS = 1765843200000;

const CREATED_BASE = {
  decision_id: DECISION_ID,
  reservation_id: RESERVATION_ID,
  budget_id: BUDGET_ID,
  window_instance_id: WINDOW_ID,
  unit: "usd_micros",
  amount_atomic_reserved: "1000000",
  decision: "ALLOW",
  ttl_expires_at: "2026-06-10T08:00:00Z",
  event_time: "2026-06-10T07:59:58Z",
};

const COMMITTED_BASE = {
  decision_id: DECISION_ID,
  reservation_id: RESERVATION_ID,
  budget_id: BUDGET_ID,
  window_instance_id: WINDOW_ID,
  unit: "usd_micros",
  amount_atomic_estimated: "950000",
  outcome: "SUCCESS",
  event_time: "2026-06-10T08:00:02Z",
};

const DENIED_BASE = {
  decision_id: DENY_DECISION_ID,
  denied_kind: "DENY",
  reason_codes: ["BUDGET_EXHAUSTED"],
  event_time: "2026-06-10T08:01:00Z",
};

// ── The vector list (inputs are snake_case — the Python dataclass shape;
//    the TS suite maps them to camelCase mechanically) ─────────────────────
const VECTORS = [
  {
    id: "AGUI-FX01",
    builder: "buildBudgetSnapshot",
    description: "snapshot minimal — required-only, no unit_id, no timestamp",
    inputs: {
      budget_id: BUDGET_ID,
      window_instance_id: WINDOW_ID,
      unit: "usd_micros",
      remaining_atomic: "25000000",
      reserved_atomic: "0",
      spent_atomic: "0",
      as_of: "2026-06-10T07:59:00Z",
    },
  },
  {
    id: "AGUI-FX02",
    builder: "buildBudgetSnapshot",
    description: "snapshot maximal — unit_id present, timestamp present",
    inputs: {
      budget_id: BUDGET_ID,
      window_instance_id: WINDOW_ID,
      unit: "usd_micros",
      unit_id: UNIT_ID,
      remaining_atomic: "24000000",
      reserved_atomic: "1000000",
      spent_atomic: "950000",
      as_of: "2026-06-10T08:00:05Z",
    },
    timestamp_ms: TS_MS,
  },
  {
    id: "AGUI-FX03",
    builder: "buildBudgetSnapshot",
    description: "snapshot — 40-digit remaining_atomic + timestamp_ms 0 (0 != absent)",
    inputs: {
      budget_id: BUDGET_ID,
      window_instance_id: WINDOW_ID,
      unit: "output_token",
      remaining_atomic: "1234567890123456789012345678901234567890",
      reserved_atomic: "0",
      spent_atomic: "0",
      as_of: "2026-06-10T07:59:00Z",
    },
    timestamp_ms: 0,
  },
  {
    id: "AGUI-FX04",
    builder: "buildBudgetSnapshot",
    description: "snapshot — unit_id present, timestamp absent",
    inputs: {
      budget_id: BUDGET_ID,
      window_instance_id: WINDOW_ID,
      unit: "usd_micros",
      unit_id: UNIT_ID,
      remaining_atomic: "25000000",
      reserved_atomic: "0",
      spent_atomic: "0",
      as_of: "2026-06-10T07:59:00Z",
    },
  },
  {
    id: "AGUI-FX05",
    builder: "buildReservationCreated",
    description: "created minimal — required-only, ALLOW, no timestamp",
    inputs: { ...CREATED_BASE },
  },
  {
    id: "AGUI-FX06",
    builder: "buildReservationCreated",
    description: "created maximal — every optional set, ALLOW_WITH_CAPS, timestamp",
    inputs: {
      ...CREATED_BASE,
      unit_id: UNIT_ID,
      decision: "ALLOW_WITH_CAPS",
      reason_codes: ["degrade_applied", "model_capped"],
      matched_rule_ids: ["rule-cap-model", "rule-budget-soft"],
      run_id: RUN_ID,
      llm_call_id: LLM_CALL_ID,
    },
    timestamp_ms: TS_MS,
  },
  {
    id: "AGUI-FX07",
    builder: "buildReservationCreated",
    description:
      "created Unicode — CJK + emoji + astral in reason_codes, U+001F in matched_rule_ids",
    inputs: {
      ...CREATED_BASE,
      reason_codes: ["預算已拒絕", "\u{1f4b8}", "\u{10348}"],
      matched_rule_ids: ["rule\u001fctl"],
    },
  },
  {
    id: "AGUI-FX08",
    builder: "buildReservationCommitted",
    description: "committed minimal — outcome SUCCESS, no timestamp",
    inputs: { ...COMMITTED_BASE },
  },
  {
    id: "AGUI-FX09",
    builder: "buildReservationCommitted",
    description: "committed — outcome PROVIDER_ERROR",
    inputs: { ...COMMITTED_BASE, outcome: "PROVIDER_ERROR" },
    timestamp_ms: TS_MS,
  },
  {
    id: "AGUI-FX10",
    builder: "buildReservationCommitted",
    description: "committed — outcome CLIENT_TIMEOUT",
    inputs: { ...COMMITTED_BASE, outcome: "CLIENT_TIMEOUT" },
  },
  {
    id: "AGUI-FX11",
    builder: "buildReservationCommitted",
    description: "committed — outcome RUN_ABORTED",
    inputs: { ...COMMITTED_BASE, outcome: "RUN_ABORTED" },
  },
  {
    id: "AGUI-FX12",
    builder: "buildReservationCommitted",
    description: "committed maximal — amount_atomic_observed + unit_id + run/llm ids",
    inputs: {
      ...COMMITTED_BASE,
      unit_id: UNIT_ID,
      amount_atomic_observed: "940123",
      run_id: RUN_ID,
      llm_call_id: LLM_CALL_ID,
    },
    timestamp_ms: TS_MS,
  },
  {
    id: "AGUI-FX13",
    builder: "buildReservationReleased",
    description: "released minimal — single Draft-01 §4 reason code",
    inputs: {
      reservation_id: RESERVATION_ID,
      reason_codes: ["client_timeout"],
      event_time: "2026-06-10T08:00:30Z",
    },
  },
  {
    id: "AGUI-FX14",
    builder: "buildReservationReleased",
    description: "released maximal — every optional set, timestamp",
    inputs: {
      reservation_id: RESERVATION_ID,
      decision_id: DECISION_ID,
      reason_codes: ["provider_error", "run_cancelled"],
      ledger_transaction_id: LEDGER_TX_ID,
      run_id: RUN_ID,
      llm_call_id: LLM_CALL_ID,
      event_time: "2026-06-10T08:00:30Z",
    },
    timestamp_ms: TS_MS,
  },
  {
    id: "AGUI-FX15",
    builder: "buildDecisionDenied",
    description: "denied minimal — denied_kind DENY",
    inputs: { ...DENIED_BASE },
  },
  {
    id: "AGUI-FX16",
    builder: "buildDecisionDenied",
    description: "denied — denied_kind STOP (run ceiling)",
    inputs: { ...DENIED_BASE, denied_kind: "STOP", reason_codes: ["RUN_CEILING"] },
  },
  {
    id: "AGUI-FX17",
    builder: "buildDecisionDenied",
    description: "denied — denied_kind STOP_RUN_PROJECTION",
    inputs: {
      ...DENIED_BASE,
      denied_kind: "STOP_RUN_PROJECTION",
      reason_codes: ["RUN_BUDGET_PROJECTION_EXCEEDED"],
    },
    timestamp_ms: TS_MS,
  },
  {
    id: "AGUI-FX18",
    builder: "buildDecisionDenied",
    description: "denied — denied_kind SKIP",
    inputs: { ...DENIED_BASE, denied_kind: "SKIP", reason_codes: ["step_budget_skip"] },
  },
  {
    id: "AGUI-FX19",
    builder: "buildDecisionDenied",
    description:
      'denied — denied_kind APPROVAL_REQUIRED with the ASP Draft-01 §2 "approval_required" code',
    inputs: {
      ...DENIED_BASE,
      denied_kind: "APPROVAL_REQUIRED",
      reason_codes: ["approval_required", "over_step_threshold"],
    },
  },
  {
    id: "AGUI-FX20",
    builder: "buildDecisionDenied",
    description: "denied maximal — every optional set incl. unit_id, timestamp",
    inputs: {
      ...DENIED_BASE,
      denied_kind: "DENY",
      reason_codes: ["BUDGET_EXHAUSTED", "rule_hard_cap"],
      matched_rule_ids: ["rule-hard-cap"],
      budget_id: BUDGET_ID,
      window_instance_id: WINDOW_ID,
      unit: "usd_micros",
      unit_id: UNIT_ID,
      run_id: RUN_ID,
      llm_call_id: LLM_CALL_ID,
    },
    timestamp_ms: TS_MS,
  },
  {
    id: "AGUI-FX21",
    builder: "buildDecisionDenied",
    description: "denied Unicode — CJK + emoji + astral reason_codes, U+001F matched_rule_ids",
    inputs: {
      ...DENIED_BASE,
      reason_codes: ["預算已拒絕", "\u{1f4b8}", "\u{10348}"],
      matched_rule_ids: ["rule\u001fctl"],
    },
  },
  {
    id: "AGUI-FX22",
    builder: "buildReservationCreated",
    description: "created — unit_id present, timestamp absent",
    inputs: { ...CREATED_BASE, unit_id: UNIT_ID },
  },
];

// ── snake_case → camelCase for the TS builder call ────────────────────────
function snakeToCamel(key) {
  return key.replace(/_([a-z])/g, (_m, c) => c.toUpperCase());
}

function toCamelInputs(inputs) {
  const out = {};
  for (const [k, v] of Object.entries(inputs)) {
    out[snakeToCamel(k)] = v;
  }
  return out;
}

function main() {
  const force = process.argv.includes("--force");
  if (existsSync(OUT_PATH) && !force) {
    console.error(
      "REFUSING to overwrite ag_ui_v1.json — the corpus is frozen after the " +
        "COV_D39_01 merge (D05 corpus discipline). New vectors mint ag_ui_v2.json. " +
        "Pass --force ONLY for the initial slice-1 minting.",
    );
    process.exit(1);
  }

  const fixtures = VECTORS.map((v) => {
    const build = BUILDERS[v.builder];
    if (!build) {
      throw new Error(`unknown builder ${v.builder}`);
    }
    const ctx = v.timestamp_ms !== undefined ? { timestampMs: v.timestamp_ms } : undefined;
    const evt = build(toCamelInputs(v.inputs), ctx);
    return {
      id: v.id,
      builder: v.builder,
      description: v.description,
      inputs: v.inputs,
      ...(v.timestamp_ms !== undefined ? { timestamp_ms: v.timestamp_ms } : {}),
      expected_canonical_json: canonicalEventJson(evt),
      expected_sse: encodeSse(evt),
    };
  });

  const corpus = {
    version: 1,
    generated_at: GENERATED_AT,
    generated_with: { package: "@spendguard/ag-ui", version: VERSION },
    fixtures,
  };

  writeFileSync(OUT_PATH, `${JSON.stringify(corpus, null, 2)}\n`, "utf8");
  console.log(`wrote ${OUT_PATH} (${fixtures.length} fixtures)`);
}

main();
