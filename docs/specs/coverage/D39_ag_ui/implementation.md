# D39 — Implementation

Directory layout, file responsibilities, and code skeletons for
`@spendguard/ag-ui` (TS) and `spendguard.integrations.ag_ui` (Python). Pair
with `design.md` (LOCKED vocabulary + API surface — §5/§8 there are copied
verbatim, never paraphrased) and `tests.md`. Where this file and `design.md`
disagree, `design.md` wins.

## 1. Repo layout

### 1.1 TypeScript — `sdk/typescript-ag-ui/` (slice 1)

Top-level sibling of `sdk/typescript-langchain/` (house convention for TS
packages: one directory per published package under `sdk/`).

```
sdk/typescript-ag-ui/
├── package.json
├── tsconfig.json
├── tsconfig.tests.json
├── tsup.config.ts
├── biome.json
├── vitest.config.ts
├── README.md                  # carries the §1.1 display-only notice verbatim
├── CHANGELOG.md
├── LICENSE_NOTICES.md
├── scripts/
│   └── size-budget.sh         # copy of sdk/typescript-langchain/scripts pattern
├── src/
│   ├── index.ts               # public barrel — design.md §8.1, nothing else
│   ├── names.ts               # SPENDGUARD_AG_UI_EVENT_NAMES + name type
│   ├── events.ts              # SpendGuardAgUiEvent + BuildContext + 5 Input interfaces
│   ├── builders.ts            # the 5 build* functions + shared payload assembly
│   ├── validate.ts            # field validators (non-empty, decimal, RFC3339, arrays)
│   ├── canonical.ts           # canonicalEventJson (design.md §7 rule)
│   ├── sse.ts                 # encodeSse + AgUiEmit type
│   ├── errors.ts              # AgUiEventValidationError
│   └── version.ts             # VERSION constant (generated, matches package.json)
└── tests/
    ├── builders.test.ts       # TP-01..TP-13 (purity, payload shape, mapping)
    ├── validate.test.ts       # TP-14..TP-19 (omission, rejection, taxonomy)
    ├── canonical.test.ts      # TP-20..TP-24 (canonical rule conformance)
    ├── sse.test.ts            # TP-25..TP-26
    ├── crossLanguage.test.ts  # TP-27 (fixture corpus byte-equality)
    ├── agUiCompat.test.ts     # TP-28..TP-29 (pinned @ag-ui/core assignability + parse)
    ├── bundle.test.ts         # TP-30..TP-31 (no node:/dep imports; size budget)
    └── _support/
        └── vectors.ts         # shared deterministic input vectors
```

Fixture corpus + generator (also slice 1):

```
sdk/fixtures/cross-language/
├── ag_ui_v1.json              # frozen corpus — NEVER edit in place (D05 discipline)
└── generate_ag_ui.mjs         # reference generator; runs against dist/ builders
```

### 1.2 Python — `spendguard.integrations.ag_ui` (slice 2)

Mirrors the dspy module layout (`__init__` + underscore-private modules):

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

`pyproject.toml` additions: `[project.optional-dependencies]` gains
`ag-ui = ["ag-ui-protocol>=0.1.19,<0.2"]`
`[VERIFY-AT-IMPL: latest version — design.md §10]`; the dev/test dependency
group pins the exact version used by `test_ag_ui_compat.py`.

**No runtime imports** from `_builders.py`/`_canonical.py` beyond stdlib
(`json`, `dataclasses`, `typing`, `re`). In particular: no `spendguard.client`,
no `_proto`, no `ag_ui` import anywhere outside the compat test. Unlike the
dspy `__init__.py`, this module has **no import-time extras guard** — the
package works with zero extras installed (that is the point).

### 1.3 Demo + docs (slice 3)

```
deploy/demo/ag_ui_events/
├── docker-compose.yaml        # overlay: counting-stub + ag-ui-runner + sse-probe
└── verify_sse.py              # the hard event-stream gate (design.md §9)

deploy/demo/verify_step_ag_ui_events.sql   # ledger gates (house style)

examples/ag-ui-events/
├── package.json
├── README.md                  # display-only notice verbatim
└── index.mjs                  # demo runner (design.md §9 steps 1-5)

deploy/demo/Makefile           # DEMO_MODE=ag_ui_events up/run branches
                               # + demo-verify-ag-ui-events target

docs/site-v2/src/content/docs/docs/integrations/ag-ui.mdx
README.md                      # repo root: adapter/integrations table row
CHANGELOG.md                   # repo root entry
sdk/python/CHANGELOG.md        # ag_ui module + extra entry
```

## 2. `package.json` skeleton (TS)

```json
{
  "name": "@spendguard/ag-ui",
  "version": "0.1.0",
  "description": "SpendGuard spend-event family for AG-UI — display-only CUSTOM event builders (budget snapshot, reservation lifecycle, deny reasons). Enforcement stays in the SpendGuard adapters and sidecar.",
  "license": "Apache-2.0",
  "author": "Michael Chen <m24927605@gmail.com>",
  "homepage": "https://github.com/m24927605/agentic-spendguard",
  "repository": {
    "type": "git",
    "url": "https://github.com/m24927605/agentic-spendguard.git",
    "directory": "sdk/typescript-ag-ui"
  },
  "bugs": "https://github.com/m24927605/agentic-spendguard/issues",
  "keywords": ["ag-ui", "agent", "llm", "spend", "budget", "spendguard", "events"],
  "type": "module",
  "engines": { "node": ">=20.10.0" },
  "sideEffects": false,
  "publishConfig": { "access": "public", "provenance": true },
  "files": ["dist/**/*.js", "dist/**/*.d.ts", "README.md", "LICENSE_NOTICES.md", "CHANGELOG.md"],
  "main": "./dist/index.js",
  "module": "./dist/index.js",
  "types": "./dist/index.d.ts",
  "exports": {
    ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js" },
    "./package.json": "./package.json"
  },
  "scripts": {
    "build": "tsup",
    "test": "vitest run",
    "lint": "biome check src tests",
    "typecheck": "tsc --noEmit && tsc -p tsconfig.tests.json --noEmit",
    "size": "bash scripts/size-budget.sh",
    "prepublishOnly": "bash ../typescript-langchain/scripts/prepublish.sh"
  },
  "peerDependencies": {
    "@ag-ui/core": "<0.1.0"
  },
  "peerDependenciesMeta": {
    "@ag-ui/core": { "optional": true }
  },
  "devDependencies": {
    "@ag-ui/core": "0.0.x-EXACT-PIN",
    "@biomejs/biome": "^1.9.4",
    "@types/node": "^20.14.0",
    "tsup": "^8.3.0",
    "typescript": "^5.6.0",
    "vitest": "^2.1.0"
  }
}
```

`[VERIFY-AT-IMPL: "@ag-ui/core" exact devDep pin + the peer-range floor
(design.md §10.2); `prepublishOnly` path — copy the script locally if the
cross-package reference is brittle in CI.]`

Notes:
- **Zero `dependencies`.** Review gate; a non-empty `dependencies` block is a
  P1 (review-standards §6.2).
- `@spendguard/sdk` appears NOWHERE in this manifest — not even peer. The
  package consumes plain data; the demo (not the package) glues SDK outcomes to
  builder inputs.

## 3. Bundle budget

| Artifact | Cap |
|---|---|
| `dist/index.js` minified | **≤ 8 KB** |
| `dist/index.js` gzipped | **≤ 3 KB** |
| `npm pack` tarball | ≤ 25 KB |

Tiny by design — five builders, one serializer, one string helper. The cap is a
build failure (`scripts/size-budget.sh` wired into `prepublishOnly`), not a
warning. No `node:` imports in `src/` keeps the bundle browser-safe
(tests.md TP-30).

## 4. TS skeletons (locked behavior, illustrative bodies)

### 4.1 `src/builders.ts` — shared assembly

```ts
import type { BuildContext, SpendGuardAgUiEvent } from "./events.js";
import { SPENDGUARD_AG_UI_EVENT_NAMES } from "./names.js";
import {
  requireNonEmpty, requireAtomic, requireRfc3339,
  requireStringArray, optionalEntry,
} from "./validate.js";

/** Assemble the envelope. Purity contract: no Date.now(), no randomness. */
function envelope(
  name: SpendGuardAgUiEvent["name"],
  value: Record<string, unknown>,
  ctx?: BuildContext,
): SpendGuardAgUiEvent {
  if (ctx?.timestampMs !== undefined) {
    requireSafeInteger("timestamp", ctx.timestampMs);
    return Object.freeze({ type: "CUSTOM", name, value: Object.freeze(value), timestamp: ctx.timestampMs });
  }
  return Object.freeze({ type: "CUSTOM", name, value: Object.freeze(value) });
}

export function buildBudgetSnapshot(input: BudgetSnapshotInput, ctx?: BuildContext): SpendGuardAgUiEvent {
  const value: Record<string, unknown> = {
    schema_version: "1",
    budget_id: requireNonEmpty("budget_id", input.budgetId),
    window_instance_id: requireNonEmpty("window_instance_id", input.windowInstanceId),
    unit: requireNonEmpty("unit", input.unit),
    remaining_atomic: requireAtomic("remaining_atomic", input.remainingAtomic),
    reserved_atomic: requireAtomic("reserved_atomic", input.reservedAtomic),
    spent_atomic: requireAtomic("spent_atomic", input.spentAtomic),
    as_of: requireRfc3339("as_of", input.asOf),
    ...optionalEntry("unit_id", input.unitId),   // omit-if-empty (design §6)
  };
  return envelope(SPENDGUARD_AG_UI_EVENT_NAMES.budgetSnapshot, value, ctx);
}

// buildReservationCreated / buildReservationCommitted /
// buildReservationReleased / buildDecisionDenied follow the same shape,
// emitting exactly the keys locked in design.md §5.4-§5.7. Two special rules:
//
// buildDecisionDenied:
//   value.decision = "DENY"  (injected literal)
//   if (input.deniedKind === "APPROVAL_REQUIRED" &&
//       !input.reasonCodes.includes("approval_required")) {
//     throw new AgUiEventValidationError("reason_codes",
//       'denied_kind APPROVAL_REQUIRED requires reason_codes to include "approval_required" (ASP Draft-01 §2)');
//   }
//
// buildReservationCreated / buildDecisionDenied:
//   reason_codes / matched_rule_ids: emitted only when provided AND non-empty
//   (design §5.4/§5.7 omit-if-absent/empty), EXCEPT denied.reason_codes and
//   released.reason_codes which are REQUIRED with ≥ 1 entry.
```

### 4.2 `src/validate.ts` — rules

| Validator | Rule | Throws |
|---|---|---|
| `requireNonEmpty(field, s)` | `typeof s === "string" && s.length > 0` (no trimming — exactness) | `AgUiEventValidationError(field)` |
| `requireAtomic(field, s)` | `/^(0\|[1-9][0-9]*)$/` — non-negative atomic decimal string, no sign, no leading zeros | same |
| `requireRfc3339(field, s)` | regex gate: `/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z\|[+-]\d{2}:\d{2})$/` — format check only, no date parsing libs | same |
| `requireStringArray(field, a, {minLen})` | array of non-empty strings; `minLen` 0 or 1 per design §5 | same |
| `requireSafeInteger(field, n)` | `Number.isSafeInteger(n) && !Object.is(n, -0) && n >= 0` | same |
| `optionalEntry(field, s)` | returns `{ [field]: s }` when `s` is a non-empty string, else `{}` | never |

Python `_validate.py` mirrors these rules and regexes character-for-character
(the regexes are part of the cross-language contract — a string accepted by
one language and rejected by the other is a fixture-level break).

### 4.3 `src/canonical.ts`

```ts
import { AgUiEventValidationError } from "./errors.js";

/** design.md §7 — locked rule. Recursive key-sort + constraints, then
 *  JSON.stringify on the rebuilt structure (stringify of a key-ordered
 *  object emits keys in insertion order, which we set to sorted order). */
export function canonicalEventJson(event: SpendGuardAgUiEvent): string {
  return JSON.stringify(canonicalize(event));
}

function canonicalize(v: unknown): unknown {
  if (typeof v === "string") { assertWellFormed(v); return v; }
  if (typeof v === "boolean") return v;
  if (typeof v === "number") { assertCanonicalInt(v); return v; }
  if (Array.isArray(v)) return v.map(canonicalize);
  if (v !== null && typeof v === "object") {
    const out: Record<string, unknown> = {};
    for (const k of Object.keys(v).sort()) {       // code-point sort; keys are ASCII-enforced
      assertAsciiKey(k);
      out[k] = canonicalize((v as Record<string, unknown>)[k]);
    }
    return out;
  }
  throw new AgUiEventValidationError("(value)", "null/undefined/unsupported type in canonical payload");
}
```

`Object.keys(...).sort()` sorts by UTF-16 code units; `assertAsciiKey`
(printable ASCII only) makes that identical to Python's code-point sort
(design §7.4). `assertWellFormed` rejects unpaired surrogates
(`String.prototype.isWellFormed()` on Node 20+). `assertCanonicalInt` enforces
safe-integer / no `-0` / finite.

### 4.4 `src/sse.ts`

```ts
export function encodeSse(event: SpendGuardAgUiEvent): string {
  return `data: ${canonicalEventJson(event)}\n\n`;
}
export type AgUiEmit = (event: SpendGuardAgUiEvent) => void | Promise<void>;
```

That is the whole transport story in v0.1.0 — anything richer (event ids,
retry fields, AG-UI client wiring) belongs to the host app.

## 5. Python skeletons

### 5.1 `_canonical.py`

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

`json.dumps(..., ensure_ascii=False, sort_keys=True, separators=(",", ":"))`
matches the TS rebuild byte-for-byte under the §7 constraints — that statement
is not an assumption, it is what TP-27/TA-27 prove against the shared corpus.

`bool` is checked BEFORE `int` in `_check` (Python `bool` is an `int`
subclass); `True`/`False` serialize as `true`/`false` in both languages.

### 5.2 `_builders.py`

Each `build_*` mirrors its TS twin key-for-key, including the injected
`schema_version: "1"`, the injected `decision: "DENY"` on denied, the
omit-if-empty rule, and the `approval_required` validation. Return value is a
plain `dict` in **already-sorted key insertion order is NOT required** — the
canonical serializer owns ordering; builders own shape.

### 5.3 `__init__.py`

Docstring opens with the design.md §1.1 display-only notice verbatim, then the
quickstart:

```python
from spendguard.integrations.ag_ui import (
    ReservationCreatedInput, build_reservation_created, encode_sse,
)

evt = build_reservation_created(ReservationCreatedInput(
    decision_id=outcome.decision_id,
    reservation_id=outcome.reservation_ids[0],
    budget_id=budget_id, window_instance_id=window_instance_id,
    unit="usd_micros", unit_id=unit_id,
    amount_atomic_reserved="1000000",
    decision="ALLOW", ttl_expires_at="2026-06-10T08:00:00Z",
    event_time="2026-06-10T07:59:58Z",
))
sse_frame = encode_sse(evt)   # hand to your AG-UI transport
```

`__all__` lists exactly: the 5 input dataclasses, the 5 builders, the 6 name
constants (5 names + the tuple), `canonical_event_json`, `encode_sse`,
`AgUiEmit`, `AgUiEventValidationError`. Nothing else.

## 6. Optional-peer / extra mechanics

| Concern | TS | Python |
|---|---|---|
| Runtime import of AG-UI lib | NEVER (grep gate, acceptance A1.6) | NEVER outside `tests/.../test_ag_ui_compat.py` |
| Typed-consumer path | optional `peerDependencies` + `peerDependenciesMeta.optional` — installs nothing by default; consumers who want `CustomEvent` typing install `@ag-ui/core` themselves | `pip install 'spendguard-sdk[ag-ui]'` pulls `ag-ui-protocol` for users who validate through pydantic models |
| Compat proof | `tests/agUiCompat.test.ts` — type-level: `const _t: import("@ag-ui/core").CustomEvent = builtEvent;` behind the exact devDep pin; runtime: parse through the exported schema if any `[VERIFY-AT-IMPL]` | `test_ag_ui_compat.py` — `CustomEvent.model_validate(event_dict)` `[VERIFY-AT-IMPL: import path]`, `pytest.importorskip("ag_ui")` guarded |
| Churn blast radius | compat test only; published wire shape never moves on an AG-UI release | same |

## 7. Demo runner — `examples/ag-ui-events/index.mjs` (slice 3)

Follows `examples/langchain-ts/index.mjs` staging conventions (the compose
overlay stages the example into a writable tmpdir and `file:`-overrides
`@spendguard/sdk` + `@spendguard/ag-ui` to the in-tree builds — see
`deploy/demo/langchain_ts/docker-compose.yaml` for the exact entrypoint
pattern, reused by-value).

```
step 0  connect+handshake (SpendGuardClient, sidecar UDS)
step 1  emit budget.snapshot   (seed env values; design.md §9.2)
step 2  ALLOW: reserve → emit reservation.created
        → fetch http://counting-stub:8765/v1/chat/completions
        → commitEstimated(SUCCESS) → emit reservation.committed
step 3  DENY: reserve(amount > remaining) → catch DecisionDenied
        → emit decision.denied
        → assert counting-stub /_count UNCHANGED and log:
          "[demo] deny enforced by sidecar pre-dispatch; AG-UI event is display-only"
step 4  serve :8077  GET /healthz → 200 "ok"
                     GET /events  → replay recorded encodeSse frames, close
```

Event recording: in-memory array of `encodeSse(event)` strings, appended in
emission order. The HTTP server is Node stdlib `http` — no express, no deps
beyond the two `file:` packages.

Env vars (set in the overlay, mirroring the langchain_ts service):
`SPENDGUARD_SIDECAR_UDS`, `SPENDGUARD_TENANT_ID`, `SPENDGUARD_BUDGET_ID`,
`SPENDGUARD_WINDOW_INSTANCE_ID`, `SPENDGUARD_UNIT_ID`,
`SPENDGUARD_PRICING_VERSION`, `SPENDGUARD_DEMO_OPENING_BALANCE_ATOMIC`,
`SPENDGUARD_COUNTING_STUB_URL`.

## 8. Makefile wiring (slice 3)

`deploy/demo/Makefile` gains, following the `langchain_ts` branch pattern
verbatim (lines ~149 and ~739 of the current file):

- `DEMO_MODE=ag_ui_events` up-branch: base stack then
  `$(COMPOSE) -f ag_ui_events/docker-compose.yaml up -d --build counting-stub ag-ui-runner`.
- `DEMO_MODE=ag_ui_events` run/verify-branch: wait for `ag-ui-runner` health,
  then `demo-verify-ag-ui-events`.
- `demo-verify-ag-ui-events` target:

```make
demo-verify-ag-ui-events:
	@$(COMPOSE) -f ag_ui_events/docker-compose.yaml run --rm --no-deps sse-probe \
	  > /tmp/spendguard-ag-ui-capture.sse
	@python3 ag_ui_events/verify_sse.py /tmp/spendguard-ag-ui-capture.sse \
	  | tee /tmp/spendguard-ag-ui-verify.out
	@RES_ID=$$(grep '^RESERVATION_ID=' /tmp/spendguard-ag-ui-verify.out | cut -d= -f2); \
	  $(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 -c \
	  "DO \$$\$$ BEGIN IF NOT EXISTS (SELECT 1 FROM reservations WHERE <pk_column> = '$$RES_ID'::uuid AND tenant_id = '00000000-0000-4000-8000-000000000001') THEN RAISE EXCEPTION 'COV_D39_GATE: SSE reservation_id % not found in ledger reservations', '$$RES_ID'; END IF; END; \$$\$$;"
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 \
	  < verify_step_ag_ui_events.sql
	@echo "[demo] COV_D39 ag_ui_events verification done"
```

`[VERIFY-AT-IMPL: `<pk_column>` — exact reservations PK column name
(design.md §9.3).]`

Also: add `ag_ui_events` to the demo-mode help text and to the
`demo-verify-all-*` master target family if the marathon target list is
regenerated during slice 3 (do NOT retro-edit other deliverables' targets).

## 9. `verify_sse.py` assertions (slice 3 — the hard gate)

Python 3 stdlib only. Parses SSE frames (`data: ` prefix, blank-line
delimited). Exits non-zero with a `COV_D39_GATE:` message on the first
failure. Asserts exactly the list in design.md §9 (exact frame count 4, exact
order, required non-empty fields per event, unit_id presence, ID joins,
denied taxonomy, canonical-bytes round-trip) and prints
`RESERVATION_ID=<uuid>` + `DENIED_DECISION_ID=<uuid>` on success. The
canonical-bytes round-trip re-implements the §7 rule inline via
`json.dumps(json.loads(payload), ensure_ascii=False, sort_keys=True,
separators=(",", ":"))` equality — deliberately NOT importing
`spendguard.integrations.ag_ui`, so the gate is independent of the library
under test.

## 10. Docs page — `docs/site-v2/src/content/docs/docs/integrations/ag-ui.mdx`

Sections: display-only notice (verbatim, first), what AG-UI is + where
SpendGuard sits (the §4 diagram), the five events with one JSON example each
(wrap embedded JSON in `is:raw` — Astro memory), TS + Python quickstarts,
ASP mapping table link, demo instructions
(`make demo-up DEMO_MODE=ag_ui_events && make demo-verify-ag-ui-events`),
0.x churn note. Repo-root `README.md` integrations table gains the
`@spendguard/ag-ui` row labeled **display-only events** (not "adapter" — it
does not enforce).

## 11. What D39 must NOT touch

- `sdk/typescript/src/**` (D05 substrate) — no changes, no new exports.
- `proto/**` — no wire changes; D39 is presentation-side only.
- Existing adapters — no auto-emit hooks in v0.1.0 (design §3).
- `sdk/fixtures/cross-language/v1.json` — frozen D05 corpus; D39 mints its own
  `ag_ui_v1.json`.
- Any text claiming enforcement via AG-UI (review-standards §1, P0).
