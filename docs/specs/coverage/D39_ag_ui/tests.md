# D39 — Tests

Numbering: **TP-XX** = TypeScript package suite (`sdk/typescript-ag-ui/tests/`),
**TA-XX** = Python suite (`sdk/python/tests/integrations/ag_ui/`),
**TD-XX** = demo-mode regression gates (slice 3). TP-NN and TA-NN with the
same NN verify the same behavior in the mirror language — a TP without its TA
twin (or vice versa) is a review finding (review-standards §5.4), except
TP-28..TP-31 / TA-28 where the concerns are language-specific.

## 1. Coverage targets

| Module | Target |
|---|---|
| `builders.ts` / `_builders.py` | ≥ 95 % stmt, ≥ 90 % branch |
| `validate.ts` / `_validate.py` | 100 % stmt + branch |
| `canonical.ts` / `_canonical.py` | 100 % stmt, ≥ 95 % branch |
| `sse.ts` (and Python `encode_sse`) | 100 % |
| Package floor | ≥ 92 % statements, ≥ 88 % branches |

## 2. Builder behavior — TP-01..TP-13 / TA-01..TA-13

| # | Test | Verifies |
|---|---|---|
| TP/TA-01 | Each of the 5 builders returns `type: "CUSTOM"` and the exact §5.2 `name` string | envelope + vocabulary lock |
| TP/TA-02 | **Purity**: calling a builder twice with deep-equal inputs yields deep-equal events; 100 repeated calls yield identical `canonicalEventJson` bytes | determinism |
| TP/TA-03 | **Clock-free**: builders succeed with no ambient clock — test freezes/monkeypatches `Date.now` / `time.time` to throw, builders still work | no hidden clock reads |
| TP/TA-04 | `timestampMs` / `timestamp_ms` provided → envelope `timestamp` equals it exactly; omitted → `timestamp` key ABSENT (not null, not 0) | envelope option |
| TP/TA-05 | `buildBudgetSnapshot` payload contains exactly the §5.3 required keys (set equality on `Object.keys` / `dict.keys`) with `schema_version === "1"` | verbatim schema |
| TP/TA-06 | `buildReservationCreated` payload matches §5.4 key set; `decision` passes through `"ALLOW"` and `"ALLOW_WITH_CAPS"` verbatim | verbatim schema + ASP enum |
| TP/TA-07 | `buildReservationCommitted` payload matches §5.5; all four `outcome` values accepted verbatim; a 5th value throws | outcome enum lock |
| TP/TA-08 | `buildReservationCommitted` emits `amount_atomic_estimated`; `amount_atomic_observed` ABSENT unless supplied, present verbatim when supplied | ASP-delta naming (design §5.5) |
| TP/TA-09 | `buildReservationReleased` payload matches §5.6; `reason_codes` with the Draft-01 §4 example values round-trips verbatim | verbatim schema |
| TP/TA-10 | `buildDecisionDenied` injects literal `decision: "DENY"` regardless of `deniedKind`; payload matches §5.7 key set | ASP DENY mapping |
| TP/TA-11 | All five `deniedKind` values accepted; each appears verbatim as `denied_kind`; a 6th value throws | taxonomy lock |
| TP/TA-12 | Builder inputs are NOT mutated (deep-freeze input in TS / compare snapshot in Python) and returned event is frozen (TS: `Object.isFrozen`; Python: dataclass inputs unchanged) | no side effects |
| TP/TA-13 | `reason_codes` / `matched_rule_ids` on created: provided non-empty → emitted verbatim in caller order; provided empty array or omitted → key ABSENT | omit-if-absent/empty arrays |

## 3. Validation — TP-14..TP-19 / TA-14..TA-19

| # | Test | Verifies |
|---|---|---|
| TP/TA-14 | **unit_id omission (P0, HARDEN_D05_UR)**: `unitId` = `undefined`/`None` → no `unit_id` key; `unitId` = `""` → no `unit_id` key; `unitId` = non-empty → emitted verbatim. Asserted on snapshot, created, committed, AND denied. The string `"unit_id":""` never appears in any canonical output across the whole fixture corpus (corpus-wide grep assertion) | never emit empty unit_id |
| TP/TA-15 | Empty required string (each required field of each builder, parameterized) → `AgUiEventValidationError` with `field` naming the payload key | required-field gate |
| TP/TA-16 | `requireAtomic` rejects: `""`, `"-1"`, `"1.5"`, `"01"`, `"1e3"`, `" 1"`, `"+1"`; accepts `"0"`, `"1"`, `"100000"`, 40-digit string | atomic decimal-string rule |
| TP/TA-17 | RFC 3339 gate rejects `"2026-06-10"`, `"yesterday"`, `""`, epoch ints; accepts `"2026-06-10T07:59:58Z"`, `"2026-06-10T07:59:58.123+08:00"` | event_time/as_of/ttl format |
| TP/TA-18 | **Denied reason taxonomy**: `reasonCodes: []` throws; `deniedKind: "APPROVAL_REQUIRED"` without `"approval_required"` in `reasonCodes` throws with a message citing ASP Draft-01 §2; with it present → builds, no silent append, array order preserved | ASP approval mapping (design §5.7) |
| TP/TA-19 | `releasedInput.reasonCodes: []` throws (≥ 1 required); created `reasonCodes` may be omitted | per-event array arity |

## 4. Canonical JSON — TP-20..TP-24 / TA-20..TA-24

| # | Test | Verifies |
|---|---|---|
| TP/TA-20 | Key sorting: an input crafted so the natural construction order is unsorted serializes with recursively sorted keys; nested object inside `value` also sorted | §7.3 |
| TP/TA-21 | Separators/whitespace: output contains no `": "`, `", "`, newline, or trailing whitespace; bytes are UTF-8 without BOM | §7.1-7.2 |
| TP/TA-22 | Unicode passthrough: CJK (`"預算已拒絕"`), emoji (`"💸"`), and an astral-plane char in `reason_codes` serialize as raw UTF-8, NOT `\uXXXX` escapes; control chars in strings escape identically (`\n` shorthand, `\u001f` for U+001F) | §7.5 escape-set parity |
| TP/TA-23 | Rejections each throw: float value, `NaN`/`Infinity`, `-0`, int > 2^53−1, `null` value, non-ASCII object key, unpaired surrogate string | §7.4-7.5 constraint set |
| TP/TA-24 | `canonicalEventJson` is idempotent: `parse → canonicalize` of its own output is byte-identical | stability |

## 5. SSE helper — TP-25..TP-26 / TA-25..TA-26

| # | Test | Verifies |
|---|---|---|
| TP/TA-25 | `encodeSse(e) === "data: " + canonicalEventJson(e) + "\n\n"` (exact string equality, every event type) | locked framing (design §7) |
| TP/TA-26 | Frame contains no interior newline (canonical JSON is single-line by construction) — guards against a future pretty-print regression breaking SSE | transport safety |

## 6. Cross-language byte-equivalence — TP-27 / TA-27 (P0)

Corpus: `sdk/fixtures/cross-language/ag_ui_v1.json`, minted by
`generate_ag_ui.mjs` from the TS builders in slice 1 and **frozen**. Schema
follows the D05 corpus convention:

```json
{
  "version": 1,
  "generated_at": "YYYY-MM-DD",
  "generated_with": { "package": "@spendguard/ag-ui", "version": "0.1.0" },
  "fixtures": [
    {
      "id": "AGUI-FX01",
      "builder": "buildReservationCreated",
      "inputs": { "...": "builder input fields, snake_case" },
      "timestamp_ms": 1765843200000,
      "expected_canonical_json": "{...exact bytes...}",
      "expected_sse": "data: {...}\n\n"
    }
  ]
}
```

| # | Test | Verifies |
|---|---|---|
| TP-27 | TS suite: for every vector, `canonicalEventJson(build*(inputs, ctx))` equals `expected_canonical_json` byte-for-byte and `encodeSse` equals `expected_sse` | TS == corpus |
| TA-27 | Python suite: same assertions from the SAME file via the mirrored builders | Python == corpus ⇒ Python == TS |

Vector matrix (**≥ 20 vectors**, all five builders covered):

- each builder: 1 minimal (required-only) + 1 maximal (every optional set) vector;
- `unit_id` absent vs present (snapshot + created);
- `timestamp_ms` absent vs present, including one `timestamp_ms: 0` vector —
  `0` is a valid epoch ms and MUST be emitted when explicitly provided
  (pins "0 ≠ absent");
- Unicode vectors: CJK + emoji + astral in `reason_codes` and a control char
  (U+001F, written as `\u001f` in the generator source) in a `matched_rule_ids` entry;
- denied: one vector per `denied_kind` value (5), including the
  APPROVAL_REQUIRED + `"approval_required"` vector;
- committed: one vector per `outcome` value (4); one with
  `amount_atomic_observed` set;
- a 40-digit `remaining_atomic` (big-amount string passthrough).

Corpus discipline (P0): `ag_ui_v1.json` is never edited in place after the
slice-1 merge; additions mint `ag_ui_v2.json` with both suites consuming both.

## 7. AG-UI compat (pinned) — TP-28..TP-29 / TA-28

| # | Test | Verifies |
|---|---|---|
| TP-28 | Type-level: `const _e: import("@ag-ui/core").CustomEvent = buildDecisionDenied(vector)` compiles under the EXACT devDep pin (`tsc -p tsconfig.tests.json`) | structural assignability `[VERIFY-AT-IMPL: exact CustomEvent type name/path in the pinned @ag-ui/core]` |
| TP-29 | Runtime: built events parse through @ag-ui/core's exported CUSTOM validator/schema if one exists; otherwise this test asserts the envelope key set `{type,name,value}` ⊆ pinned package's parsed shape | `[VERIFY-AT-IMPL: whether @ag-ui/core exports a runtime schema]` |
| TA-28 | `pytest.importorskip("ag_ui")`; `CustomEvent.model_validate(built_event)` succeeds for all five builders under the exact test pin | Python pydantic compat `[VERIFY-AT-IMPL: import path, e.g. ag_ui.core.CustomEvent]` |

A compat failure on a NEWER AG-UI version than the pin is a P1 maintenance
finding — it must NOT move the locked wire shape (design §10.3).

## 8. Packaging hygiene — TP-30..TP-31

| # | Test | Verifies |
|---|---|---|
| TP-30 | `dist/index.js` contains no `node:` import, no `require(`, and no import of `@ag-ui/core` or `@spendguard/sdk` (string scan of the built bundle) | zero-dep, browser-safe |
| TP-31 | Size budget: minified ≤ 8 KB, gz ≤ 3 KB (`scripts/size-budget.sh`) | implementation §3 cap |

Python twin lives in acceptance (A2.6): `python -c "import spendguard.integrations.ag_ui"`
succeeds in a venv WITHOUT the `ag-ui` extra installed.

## 9. Demo-mode regression — TD-01..TD-06 (slice 3)

| # | Gate | Pass condition |
|---|---|---|
| TD-01 | `make demo-up DEMO_MODE=ag_ui_events` | exit 0; `ag-ui-runner` healthy; runner log shows the 4 emission lines and the deny-enforced-by-sidecar line |
| TD-02 | `make demo-verify-ag-ui-events` | exit 0 end-to-end (capture + verify_sse.py + ledger join + SQL gate) |
| TD-03 | `verify_sse.py` strictness self-test: feed it a mutated capture (drop `unit_id`, reorder events, blank a `reservation_id`, de-canonicalize whitespace — 4 mutations) | non-zero exit with `COV_D39_GATE:` message for EACH mutation (the gate gates) |
| TD-04 | Counting-stub invariance: `GET /_count` after the run shows exactly 1 call | the DENY step provably never reached the provider; enforcement happened at the sidecar, display at AG-UI |
| TD-05 | Ledger join: `RESERVATION_ID` printed by verify_sse.py exists in `reservations` for the demo tenant | display↔ledger consistency |
| TD-06 | `verify_step_ag_ui_events.sql` | `reserve >= 1`, `commit_estimated >= 1`, `denied_decision >= 1` for the demo tenant; `ON_ERROR_STOP=1` |

TD-03 is mandatory: a verify script that cannot fail is not a gate
(demo-as-quality-gate memory; HARDEN_D05_UR removed exactly this class of
softening).

## 10. Slice → test mapping

| Slice | Tests added |
|---|---|
| `COV_D39_01_ts_pkg` | TP-01..TP-31; corpus `ag_ui_v1.json` + generator; TP-27 green against the freshly minted corpus |
| `COV_D39_02_py_mirror` | TA-01..TA-28; TA-27 green against the FROZEN corpus (no corpus edits permitted in this slice — a needed edit means slice 1 was wrong and goes back to review) |
| `COV_D39_03_demo_docs` | TD-01..TD-06 |
