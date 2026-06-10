# COV_D39_02_py_mirror — D39 AG-UI spend-event family: Python mirror

> **Deliverable**: D39 AG-UI spend-event family (display-only)
> **Slice**: 2 of 3 (M)
> **Spec set**: [`docs/specs/coverage/D39_ag_ui/`](../specs/coverage/D39_ag_ui/)
> **LOCKED design.md trumps this slice doc** (coverage build-plan §1.2 P0; D05/7 slice-author bug pattern). Schema/API text below is copied verbatim from the spec set — if any copy here disagrees with `design.md`, `design.md` wins and the disagreement is a slice-author bug.

## Scope

Implement `spendguard.integrations.ag_ui` — the 1:1 snake_case Python mirror of the slice-1 TS package: five name constants, five frozen-dataclass inputs, five pure builders, mirrored validators (regexes character-identical to `validate.ts`), `canonical_event_json` / `encode_sse`, and `AgUiEventValidationError` — shipping in the next `spendguard-sdk` minor (design.md §2.3). Add the `pyproject.toml` optional extra `ag-ui` and the full Python suite TA-01..TA-28, including TA-27 which consumes the **FROZEN** slice-1 corpus `sdk/fixtures/cross-language/ag_ui_v1.json` byte-for-byte.

The discipline of this slice IS the cross-language check: Python is an independent implementation against frozen bytes, not a co-evolving twin (design.md §12 justification). Zero corpus edits are permitted here — a "needed" edit means slice 1 was wrong and slice 1 goes back to review (tests.md §10). The module has zero runtime deps beyond stdlib and works with no extras installed. Size class: **M**.

## Files touched

All NEW except `sdk/python/pyproject.toml`, per implementation.md §1.2 (verbatim layout):

```
sdk/python/src/spendguard/integrations/ag_ui/
├── __init__.py                # docstring w/ display-only notice + __all__ re-exports
├── _types.py                  # 5 frozen dataclasses (design.md §8.2)
├── _builders.py               # build_* functions + shared payload assembly
├── _validate.py               # field validators (mirrors validate.ts rules)
├── _canonical.py              # canonical_event_json + encode_sse
└── _errors.py                 # AgUiEventValidationError

sdk/python/tests/integrations/ag_ui/
├── __init__.py
├── test_builders.py           # TA-01..TA-13
├── test_validate.py           # TA-14..TA-19
├── test_canonical.py          # TA-20..TA-24
├── test_sse.py                # TA-25..TA-26
├── test_cross_language.py     # TA-27 (same ag_ui_v1.json, byte-for-byte)
└── test_ag_ui_compat.py       # TA-28 (pinned ag-ui-protocol parse; skipif not installed)
```

`pyproject.toml` additions (implementation.md §1.2, verbatim): `[project.optional-dependencies]` gains
`ag-ui = ["ag-ui-protocol>=0.1.19,<0.2"]`
`[VERIFY-AT-IMPL: latest version — design.md §10]`; the dev/test dependency
group pins the exact version used by `test_ag_ui_compat.py`.

Runtime-import rule (implementation.md §1.2, verbatim):

> **No runtime imports** from `_builders.py`/`_canonical.py` beyond stdlib
> (`json`, `dataclasses`, `typing`, `re`). In particular: no `spendguard.client`,
> no `_proto`, no `ag_ui` import anywhere outside the compat test. Unlike the
> dspy `__init__.py`, this module has **no import-time extras guard** — the
> package works with zero extras installed (that is the point).

## LOCKED surface — quoted verbatim

### design.md §8.2 — Public API surface, Python (verbatim signatures)

```python
# Event-name constants (values identical to §5.2)
BUDGET_SNAPSHOT: str       # "spendguard.budget.snapshot"
RESERVATION_CREATED: str   # "spendguard.reservation.created"
RESERVATION_COMMITTED: str # "spendguard.reservation.committed"
RESERVATION_RELEASED: str  # "spendguard.reservation.released"
DECISION_DENIED: str       # "spendguard.decision.denied"
SPENDGUARD_AG_UI_EVENT_NAMES: tuple[str, ...]  # all five, §5.2 table order

# Frozen-dataclass inputs mirroring §8.1 field-for-field (snake_case):
@dataclass(frozen=True, slots=True)
class BudgetSnapshotInput: ...        # budget_id, window_instance_id, unit,
                                      # unit_id=None, remaining_atomic,
                                      # reserved_atomic, spent_atomic, as_of
@dataclass(frozen=True, slots=True)
class ReservationCreatedInput: ...
@dataclass(frozen=True, slots=True)
class ReservationCommittedInput: ...
@dataclass(frozen=True, slots=True)
class ReservationReleasedInput: ...
@dataclass(frozen=True, slots=True)
class DecisionDeniedInput: ...

# Builders (pure; return a plain dict shaped per §5.1)
def build_budget_snapshot(
    input: BudgetSnapshotInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]: ...
def build_reservation_created(
    input: ReservationCreatedInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]: ...
def build_reservation_committed(
    input: ReservationCommittedInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]: ...
def build_reservation_released(
    input: ReservationReleasedInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]: ...
def build_decision_denied(
    input: DecisionDeniedInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]: ...

# Serialization + transport helper
def canonical_event_json(event: Mapping[str, Any]) -> str: ...
def encode_sse(event: Mapping[str, Any]) -> str: ...
AgUiEmit = Callable[[Mapping[str, Any]], None]

class AgUiEventValidationError(ValueError):
    field: str
```

Optional-field defaults are `None`; `None` and `""` both mean "omit" (§6).

The dataclasses mirror the §8.1 TS inputs **field-for-field** (snake_case). The payload key sets the builders emit are locked in design.md §5.3-§5.7 — the full tables are quoted verbatim in [`COV_D39_01_ts_pkg.md`](./COV_D39_01_ts_pkg.md) and remain the single shape for both languages; this slice adds NO key, renames NO key, and injects the same literals (`schema_version: "1"` on every event; `decision: "DENY"` on denied; APPROVAL_REQUIRED ⇒ `reason_codes` must already include `"approval_required"`, validate-and-throw, never silently append — design §5.7).

### design.md §6 — unitId invariant (HARDEN_D05_UR)

Any payload that references a unit MUST carry `unit_id` when the caller has it.
**Never emit an empty `unit_id`**: if the input `unitId` is `undefined`/`None`
or the empty string, the builder **omits the key entirely** — documented here
and asserted by tests (tests.md TP-14/TA-14). The same omit-if-empty rule
applies uniformly to every optional string field (`run_id`, `llm_call_id`,
`decision_id` on released, `budget_id` on denied, …): empty string and absent
are the same thing and serialize identically — this collapse is load-bearing
for cross-language byte-equivalence.

### design.md §7 — Canonical JSON rule (LOCKED)

1. **Encoding**: UTF-8, no BOM, no trailing newline.
2. **Whitespace**: none. Separators are `,` and `:` exactly
   (Python: `json.dumps(obj, ensure_ascii=False, sort_keys=True, separators=(",", ":"))`;
   TS: recursive key-sorted rebuild + `JSON.stringify`).
3. **Key order**: object keys sorted lexicographically by Unicode code point,
   recursively at every nesting level.
4. **Keys**: ASCII-only (`[\x21-\x7e]`), enforced by the serializer (throw on
   violation). This makes Python's code-point sort and JS's UTF-16 code-unit
   sort provably identical for every legal key.
5. **Values**: strings, booleans, integers, arrays, objects only.
   - `null` is forbidden — omit the key instead.
   - Floats / non-finite numbers are forbidden (throw). Integers must satisfy
     `|n| ≤ 2^53 − 1` and `-0` is forbidden. (In practice the only number in a
     D39 event is the envelope `timestamp`; every amount is a decimal string.)
   - Strings must be valid Unicode; unpaired surrogates are rejected (throw).
     Both `json.dumps(ensure_ascii=False)` and `JSON.stringify` then agree on
     the escape set: `"` `\` and the C0 controls (shorthand `\b \t \n \f \r`,
     `\u00XX` otherwise); all other characters pass through as raw UTF-8.
6. **Array order**: preserved as given (arrays are caller-ordered, e.g.
   `reason_codes`).

`canonicalEventJson(event)` / `canonical_event_json(event)` apply this rule to
the **whole envelope** (`type`, `name`, `value`, `timestamp?`). The SSE helper
is defined on top of it:

```
encodeSse(e) === "data: " + canonicalEventJson(e) + "\n\n"
```

### Frozen-corpus rule (design.md §7 + tests.md §6, verbatim)

design.md §7:

> **Fixture corpus**: `sdk/fixtures/cross-language/ag_ui_v1.json`, minted in
> slice 1 by the TS reference generator, frozen, then consumed byte-for-byte by
> both the TS and Python suites (D05 corpus discipline: never edit in place; new
> vectors → `ag_ui_v2.json`). ≥ 20 vectors per tests.md §4.

tests.md §6:

> Corpus discipline (P0): `ag_ui_v1.json` is never edited in place after the
> slice-1 merge; additions mint `ag_ui_v2.json` with both suites consuming both.

tests.md §10, slice-2 row:

> TA-01..TA-28; TA-27 green against the FROZEN corpus (no corpus edits
> permitted in this slice — a needed edit means slice 1 was wrong and goes back
> to review)

### implementation.md §5.1 — `_canonical.py` skeleton (verbatim)

```python
import json
import re
from typing import Any, Mapping

_ASCII_KEY_RE = re.compile(r"^[\x21-\x7e]+$")

def canonical_event_json(event: Mapping[str, Any]) -> str:
    _check(event)
    return json.dumps(
        event, ensure_ascii=False, sort_keys=True, separators=(",", ":"),
        allow_nan=False,
    )

def _check(v: Any) -> None:
    # Recursive constraints per design.md §7: ASCII keys, no null, no float,
    # safe-range ints, well-formed strings (no unpaired surrogates).
    ...

def encode_sse(event: Mapping[str, Any]) -> str:
    return f"data: {canonical_event_json(event)}\n\n"
```

`bool` is checked BEFORE `int` in `_check` (Python `bool` is an `int` subclass); `True`/`False` serialize as `true`/`false` in both languages (implementation §5.1).

### implementation.md §5.2 — `_builders.py` rule (verbatim)

> Each `build_*` mirrors its TS twin key-for-key, including the injected
> `schema_version: "1"`, the injected `decision: "DENY"` on denied, the
> omit-if-empty rule, and the `approval_required` validation. Return value is a
> plain `dict` in **already-sorted key insertion order is NOT required** — the
> canonical serializer owns ordering; builders own shape.

### implementation.md §5.3 — `__init__.py` `__all__` (verbatim)

> `__all__` lists exactly: the 5 input dataclasses, the 5 builders, the 6 name
> constants (5 names + the tuple), `canonical_event_json`, `encode_sse`,
> `AgUiEmit`, `AgUiEventValidationError`. Nothing else.

The `__init__.py` docstring opens with the design.md §1.1 display-only notice **verbatim** (acceptance A6.2; review-standards §1.2):

> **Display-only.** AG-UI events are a presentation surface. SpendGuard
> enforcement happens in the SpendGuard adapters and sidecar before the
> provider call; these events report decisions already made and can neither
> grant nor deny spend.

Validator parity: `_validate.py` mirrors the implementation.md §4.2 rules and regexes character-for-character (`requireAtomic` `/^(0|[1-9][0-9]*)$/`; RFC 3339 `/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/`) — "the regexes are part of the cross-language contract — a string accepted by one language and rejected by the other is a fixture-level break" (implementation §4.2; review-standards §4.5).

## VERIFY-AT-IMPL markers owned by this slice

From review-standards §8 (slice column = 2). Inventing a value is a P0.

| Marker | Where | Pre-declared fallback |
|---|---|---|
| `ag-ui-protocol` latest version / extra range / test pin | design §10.2, impl §1.2 | Spec baseline is `ag-ui = ["ag-ui-protocol>=0.1.19,<0.2"]` with an exact test pin (`ag-ui-protocol==0.1.19`-era). Verify the latest 0.1.x at impl time; record the chosen pin here. The extra range stays `<0.2` (design §10.2). |
| Python `CustomEvent` import path (e.g. `ag_ui.core.CustomEvent`) | tests TA-28 | TA-28 is `pytest.importorskip("ag_ui")`-guarded; if the import path differs from the spec's example, record the verified path here — the test pin makes it deterministic. If no pydantic `CustomEvent` model exists at the pin, mirror the TP-29 fallback (assert envelope key set `{type,name,value}` against the pinned package's parsed shape) and flag for orchestrator review. |
| `DecisionStopped` STOP vs STOP_RUN_PROJECTION distinguishability — **Python err-class half** (slice 3 owns the demo-mapping half) | design §5.7 | Per the §5.7 marker text: "if not, callers emit `STOP` and the projection nuance stays visible in `reason_codes` (`RUN_BUDGET_PROJECTION_EXCEEDED`, `RUN_STEPS_EXCEEDED`, `RUN_CEILING`, `RUN_DRIFT_DETECTED`)". Record what the Python SDK error class actually exposes; do NOT change the §5.7 `denied_kind` enum either way. |

## Test/verification plan

Delivers TA-01..TA-28 (tests.md §10). TA-01..TA-27 are the exact mirror twins of TP-01..TP-27 — a TP without its TA twin (or vice versa) is a review finding (tests.md preamble; review-standards §5.4). One-line descriptions:

| ID | One-line description (tests.md; mirror of the TP twin) |
|---|---|
| TA-01 | Each of the 5 builders returns `type: "CUSTOM"` and the exact §5.2 `name` string |
| TA-02 | Purity: deep-equal inputs → deep-equal events; 100 repeated calls → identical `canonical_event_json` bytes |
| TA-03 | Clock-free: `time.time` monkeypatched to throw; builders still work |
| TA-04 | `timestamp_ms` provided → envelope `timestamp` equals it exactly; omitted → key ABSENT (not null, not 0) |
| TA-05 | `build_budget_snapshot` payload = exactly the §5.3 key set; `schema_version == "1"` |
| TA-06 | `build_reservation_created` matches §5.4 key set; `decision` passes through `"ALLOW"` / `"ALLOW_WITH_CAPS"` verbatim |
| TA-07 | `build_reservation_committed` matches §5.5; all four `outcome` values accepted verbatim; a 5th throws |
| TA-08 | Emits `amount_atomic_estimated`; `amount_atomic_observed` ABSENT unless supplied, verbatim when supplied |
| TA-09 | `build_reservation_released` matches §5.6; Draft-01 §4 example `reason_codes` round-trip verbatim |
| TA-10 | `build_decision_denied` injects literal `decision: "DENY"` regardless of `denied_kind`; §5.7 key set |
| TA-11 | All five `denied_kind` values accepted verbatim; a 6th throws |
| TA-12 | Builder inputs unchanged (frozen-dataclass inputs compared against snapshot) |
| TA-13 | created `reason_codes`/`matched_rule_ids`: non-empty → verbatim caller order; empty/omitted → key ABSENT |
| TA-14 | unit_id omission (P0, HARDEN_D05_UR): `None`/`""` → no key; non-empty → verbatim; snapshot, created, committed, AND denied; `"unit_id":""` never appears corpus-wide |
| TA-15 | Empty required string (parameterized) → `AgUiEventValidationError` with `field` naming the payload key |
| TA-16 | Atomic rule rejects `""`, `"-1"`, `"1.5"`, `"01"`, `"1e3"`, `" 1"`, `"+1"`; accepts `"0"`, `"1"`, `"100000"`, 40-digit string |
| TA-17 | RFC 3339 gate rejects `"2026-06-10"`, `"yesterday"`, `""`, epoch ints; accepts the two valid forms |
| TA-18 | Denied taxonomy: `reason_codes=[]` throws; APPROVAL_REQUIRED without `"approval_required"` throws citing ASP Draft-01 §2; with it → builds, no silent append, order preserved |
| TA-19 | released `reason_codes=[]` throws (≥ 1 required); created `reason_codes` may be omitted |
| TA-20 | Recursive key sorting incl. nested objects inside `value` |
| TA-21 | No `": "`, `", "`, newline, trailing whitespace; UTF-8 without BOM |
| TA-22 | Unicode passthrough: CJK/emoji/astral raw UTF-8, not `\uXXXX`; control chars escape identically |
| TA-23 | Rejections: float, `NaN`/`Infinity`, `-0`, int > 2^53−1, `null` value, non-ASCII key, unpaired surrogate |
| TA-24 | `canonical_event_json` idempotent on its own output |
| TA-25 | `encode_sse(e) == "data: " + canonical_event_json(e) + "\n\n"` exact, every event type |
| TA-26 | Frame contains no interior newline |
| TA-27 | **P0**: every vector of the SAME frozen `ag_ui_v1.json`: builders + `canonical_event_json` == `expected_canonical_json` byte-for-byte; `encode_sse` == `expected_sse` (Python == corpus ⇒ Python == TS) |
| TA-28 | `pytest.importorskip("ag_ui")`; `CustomEvent.model_validate(built_event)` succeeds for all five builders under the exact test pin |

## Acceptance gates (slice subset per acceptance.md §8)

```bash
# A2.5  full Python suite incl. tests/integrations/ag_ui/ (TA-01..TA-28)
cd sdk/python && make test
# A2.6  zero-extra import path (no ag-ui-protocol installed)
python3 -m venv /tmp/d39-noextra && /tmp/d39-noextra/bin/pip install -e sdk/python && \
  /tmp/d39-noextra/bin/python -c "import spendguard.integrations.ag_ui as m; print(sorted(m.__all__)[0])"
# A2.7  extra installed → TA-28 runs (not skipped) and passes
pip install -e 'sdk/python[ag-ui]'
python -m pytest sdk/python/tests/integrations/ag_ui/test_ag_ui_compat.py -q

# A3.3  TA-27 green — Python == same corpus
cd sdk/python && python -m pytest tests/integrations/ag_ui/test_cross_language.py -q
# A3.6  corpus history: exactly one content-creating commit (slice 1); zero in-place edits afterward
git log --follow --oneline sdk/fixtures/cross-language/ag_ui_v1.json

# A4.3  vocabulary lock, Python side
python3 -c "import spendguard.integrations.ag_ui as m; assert set(m.SPENDGUARD_AG_UI_EVENT_NAMES) == {'spendguard.budget.snapshot','spendguard.reservation.created','spendguard.reservation.committed','spendguard.reservation.released','spendguard.decision.denied'}; print('OK')"

# A6.2  __init__.py docstring carries the §1.1 display-only notice verbatim
sed -n '1,20p' sdk/python/src/spendguard/integrations/ag_ui/__init__.py

# A7.2 (Python half)  no hashing
grep -rE "blake2b|createHash|hashlib|crypto\.subtle" sdk/python/src/spendguard/integrations/ag_ui/   # expect empty
# A7.4 (Python half)  purity
grep -rn "time\.\|random\.\|os.environ" sdk/python/src/spendguard/integrations/ag_ui/_builders.py sdk/python/src/spendguard/integrations/ag_ui/_canonical.py   # expect empty
```

Plus (acceptance §8 slice-2 row): pyproject `ag-ui` extra present.

## Anti-scope (NOT in this slice)

- **ZERO corpus edits** — `ag_ui_v1.json` is frozen; "a needed edit means slice 1 was wrong and goes back to review" (tests.md §10); any in-place edit is a Blocker (review-standards §4.3, sign-off §12: "slice 2: no corpus edits, no TS src edits").
- **No TS `src/` edits** — `sdk/typescript-ag-ui/` is untouched here (review-standards §12).
- **No demo, overlay, Makefile, `verify_sse.py`, SQL gate** — slice 3.
- **No docs-site page, repo-root README row, repo-root CHANGELOG or `sdk/python/CHANGELOG.md` entries** — slice 3 (implementation §1.3 places `sdk/python/CHANGELOG.md` in slice 3).
- **No upstream ag-ui repo contribution** (design §3; acceptance A6.7) and **no browser UI** (design §3).
- **No gating/enforcement claims anywhere** — display-only, P0 (design §1.1, review-standards §1).
- **No queryBudget RPC work** (design §5.3 NB; non-goal carried by the whole deliverable).
- **No import-time extras guard** in `__init__.py` (implementation §1.2 — explicitly unlike dspy) and **no runtime `ag_ui` / `spendguard.client` / `_proto` imports** outside the compat test.
- **No ID minting / hashing** — every ID is an input (design §11.6, review-standards §3).
- **No PyPI publish / version tag** — release decisions stay with the orchestrator (acceptance §9).

## Backlinks

- [`design.md`](../specs/coverage/D39_ag_ui/design.md) — §5 vocabulary (payload truth), §6 unitId, §7 canonical + corpus, §8.2 Python API, §10 pinning, §12 slice plan
- [`implementation.md`](../specs/coverage/D39_ag_ui/implementation.md) — §1.2 layout + extra, §4.2 validator parity, §5 Python skeletons, §6 extra mechanics
- [`tests.md`](../specs/coverage/D39_ag_ui/tests.md) — TA-01..TA-28, §6 corpus discipline, §10 mapping
- [`acceptance.md`](../specs/coverage/D39_ag_ui/acceptance.md) — A2.5-A2.7, A3.3, A3.6, A4.3, A6.2, A7.2/A7.4, §8 slice subset
- [`review-standards.md`](../specs/coverage/D39_ag_ui/review-standards.md) — §4 byte-equivalence, §5.4 twins, §6.6, §8 marker table, §12 sign-off
