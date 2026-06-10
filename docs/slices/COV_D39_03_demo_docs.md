# COV_D39_03_demo_docs — D39 AG-UI spend-event family: demo + docs

> **Deliverable**: D39 AG-UI spend-event family (display-only)
> **Slice**: 3 of 3 (M)
> **Spec set**: [`docs/specs/coverage/D39_ag_ui/`](../specs/coverage/D39_ag_ui/)
> **LOCKED design.md trumps this slice doc** (coverage build-plan §1.2 P0; D05/7 slice-author bug pattern). Schema/gate text below is copied verbatim from the spec set — if any copy here disagrees with `design.md`, `design.md` wins and the disagreement is a slice-author bug.

## Scope

Ship the demo-as-quality-gate proof and all deliverable-level docs: compose overlay `deploy/demo/ag_ui_events/` (counting-stub + ag-ui-runner + sse-probe) layered on the base stack, the runner `examples/ag-ui-events/index.mjs` driving a REAL sidecar run (handshake → snapshot → ALLOW reserve/commit → DENY → SSE replay), the hard verify gate `verify_sse.py` + ledger join + house-style SQL gate `verify_step_ag_ui_events.sql`, `deploy/demo/Makefile` branches for `DEMO_MODE=ag_ui_events` + `demo-verify-ag-ui-events`, the docs-site page `ag-ui.mdx`, the repo-root README integrations row, and the three CHANGELOG entries. Both published surfaces from slices 1-2 are exercised end-to-end here (design.md §12).

The demo's honesty rules are load-bearing: real `decision_id`/`reservation_id` from real RPCs (no fabricated IDs/amounts — HARDEN_04 lesson), the snapshot from seeded env values cross-checked against the ledger, the deny provably enforced by the sidecar pre-dispatch (counting-stub counter unchanged), and a verify gate that can FAIL (TD-03 mutation drill — HARDEN_D05_UR removed exactly this class of softening). Size class: **M**.

## Files touched

Per implementation.md §1.3 (verbatim list):

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

## LOCKED surface — quoted verbatim

### design.md §1.1 — display-only notice (must appear VERBATIM in `examples/ag-ui-events/README.md` and the docs page; review-standards §1.2)

> **Display-only.** AG-UI events are a presentation surface. SpendGuard
> enforcement happens in the SpendGuard adapters and sidecar before the
> provider call; these events report decisions already made and can neither
> grant nor deny spend.

### design.md §9 — Demo design, `DEMO_MODE=ag_ui_events` (the demo gate spec, verbatim)

Overlay `deploy/demo/ag_ui_events/docker-compose.yaml` layered on the base
stack (postgres + sidecar + ledger + outbox-forwarder), same layering as
`deploy/demo/langchain_ts/docker-compose.yaml`. Reuses the langchain_ts demo's
seeded tenant/budget/window/unit IDs and its `counting-stub` provider pattern.

Services:

- `counting-stub` — by-value copy of the langchain_ts overlay's mock OpenAI
  provider (overlay-independence convention).
- `ag-ui-runner` — Node 20 container (langchain-runner staging pattern) running
  `examples/ag-ui-events/index.mjs`:
  1. `SpendGuardClient` connect + handshake on the sidecar UDS
     (`@spendguard/sdk`, real gRPC, real ledger).
  2. Build `spendguard.budget.snapshot` from the demo seed values passed via
     env (`SPENDGUARD_BUDGET_ID` / `SPENDGUARD_WINDOW_INSTANCE_ID` /
     `SPENDGUARD_UNIT_ID` / `SPENDGUARD_DEMO_OPENING_BALANCE_ATOMIC`), with
     `reserved_atomic="0"` / `spent_atomic="0"` — true at fresh-stack start and
     cross-checked against the ledger by the verify gate, so nothing is
     fabricated. (queryBudget RPC does not exist yet — §5.3 note.)
  3. **ALLOW step**: `client.reserve(...)` (substrate-derived
     `run_id`/`llm_call_id`/`decision_id` via `newUuid7` /
     `deriveIdempotencyKey` — D39 derives nothing) → build
     `reservation.created` from the `DecisionOutcome` → HTTP call to
     `counting-stub` → `client.commitEstimated(outcome="SUCCESS")` → build
     `reservation.committed`.
  4. **DENY step**: `client.reserve` with a claim exceeding the remaining
     budget → catch `DecisionDenied` → build `decision.denied` → assert the
     counting-stub hit counter did NOT increase (runner-side proof that the
     deny was enforced by the sidecar, not by AG-UI — the demo log line says
     exactly that).
  5. Serve HTTP on `:8077`: `GET /healthz`; `GET /events` replays all recorded
     frames (`encodeSse` output, in emission order) then closes.
- `sse-probe` — one-shot `curlimages/curl` service (compose `profiles:
  ["verify"]`) that fetches `http://ag-ui-runner:8077/events` to stdout.

**Hard gate** (`make demo-verify-ag-ui-events`, acceptance.md §5):

1. Capture: `sse-probe` output → host temp file.
2. `deploy/demo/ag_ui_events/verify_sse.py` asserts (exact, not `>=`, since
   the capture is one fresh scripted run):
   - exactly 4 `data:` frames; order: `budget.snapshot`,
     `reservation.created`, `reservation.committed`, `decision.denied`;
   - every required field of §5.3-§5.7 present and non-empty;
   - `unit_id` present and non-empty on snapshot/created/committed (demo
     passes `SPENDGUARD_UNIT_ID`);
   - `created.reservation_id == committed.reservation_id` and
     `created.decision_id == committed.decision_id`;
   - `denied.decision == "DENY"` and `denied.reason_codes` non-empty;
   - every frame's payload re-serializes to the identical bytes under the §7
     rule (wire == canonical form);
   - prints `RESERVATION_ID=<uuid>` for step 3.
3. Display↔ledger join: psql asserts the `RESERVATION_ID` from the event
   stream exists in the ledger `reservations` table for the demo tenant —
   display events provably describe real ledger state.
   `[VERIFY-AT-IMPL: exact reservations PK column name for the join.]`
4. `deploy/demo/verify_step_ag_ui_events.sql`: house-style ledger gates
   (`reserve >= 1`, `commit_estimated >= 1`, `denied_decision >= 1` for the
   demo tenant; `>=` per the SQL-gate robustness convention).

A browser UI is OPTIONAL and out of scope; the asserted SSE content is the
gate.

### design.md §11.9 — demo event count (locked decision, verbatim)

> 9. **Demo emits 4 events** (snapshot, created, committed, denied);
>    `released` is fixture/unit-tested only in v0.1.0 — the demo's deny step
>    never creates a reservation to release, and adding an abort step would
>    grow the demo past the display-play size. Documented non-gap.

### implementation.md §7 — runner steps + env (verbatim)

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

Staging: follows `examples/langchain-ts/index.mjs` conventions — the compose overlay stages the example into a writable tmpdir and `file:`-overrides `@spendguard/sdk` + `@spendguard/ag-ui` to the in-tree builds (see `deploy/demo/langchain_ts/docker-compose.yaml` for the exact entrypoint pattern, reused by-value) (implementation §7).

### implementation.md §8 — Makefile wiring (verbatim)

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

Also: add `ag_ui_events` to the demo-mode help text and to the
`demo-verify-all-*` master target family if the marathon target list is
regenerated during slice 3 (do NOT retro-edit other deliverables' targets).

### implementation.md §9 — `verify_sse.py` (verbatim)

> Python 3 stdlib only. Parses SSE frames (`data: ` prefix, blank-line
> delimited). Exits non-zero with a `COV_D39_GATE:` message on the first
> failure. Asserts exactly the list in design.md §9 (exact frame count 4, exact
> order, required non-empty fields per event, unit_id presence, ID joins,
> denied taxonomy, canonical-bytes round-trip) and prints
> `RESERVATION_ID=<uuid>` + `DENIED_DECISION_ID=<uuid>` on success. The
> canonical-bytes round-trip re-implements the §7 rule inline via
> `json.dumps(json.loads(payload), ensure_ascii=False, sort_keys=True,
> separators=(",", ":"))` equality — deliberately NOT importing
> `spendguard.integrations.ag_ui`, so the gate is independent of the library
> under test.

### implementation.md §10 — docs page + README row (verbatim)

> Sections: display-only notice (verbatim, first), what AG-UI is + where
> SpendGuard sits (the §4 diagram), the five events with one JSON example each
> (wrap embedded JSON in `is:raw` — Astro memory), TS + Python quickstarts,
> ASP mapping table link, demo instructions
> (`make demo-up DEMO_MODE=ag_ui_events && make demo-verify-ag-ui-events`),
> 0.x churn note. Repo-root `README.md` integrations table gains the
> `@spendguard/ag-ui` row labeled **display-only events** (not "adapter" — it
> does not enforce).

Docs must also state that `spendguard.*` AG-UI events are unsigned UI hints and MUST NOT be treated as the audit chain (design §5.8 last row; review-standards §1.4). CHANGELOG entries (×3: `sdk/typescript-ag-ui/CHANGELOG.md` 0.1.0 already from slice 1, `sdk/python/CHANGELOG.md`, repo-root `CHANGELOG.md`) each name the five events and say "display-only" (acceptance A6.5).

## VERIFY-AT-IMPL markers owned by this slice

From review-standards §8 (slice column = 3, plus the slice-3 half of the shared marker). Inventing a value is a P0.

| Marker | Where | Pre-declared fallback |
|---|---|---|
| `reservations` PK column for the ledger join (`<pk_column>` in the Makefile snippet) | design §9.3, impl §8 | None invented — read the actual ledger migration/schema and record the verified column name here. The join itself is non-negotiable (review-standards §7.6). |
| queryBudget wire status at demo time | design §5.3 | Per the §5.3 marker text: "if the queryBudget wire lands before COV_D39_03, the demo MUST source the snapshot from it instead of seed env vars." Otherwise: seed env values with the ledger cross-check exactly as design §9.2 specifies. Record which path was taken. |
| `DecisionStopped` STOP vs STOP_RUN_PROJECTION distinguishability — **demo-mapping half** (slice 2 owned the Python err-class half) | design §5.7 | The demo's deny step catches `DecisionDenied` → `denied_kind: "DENY"`, so the demo does not depend on the distinction; record the slice-2 finding and per §5.7: if not distinguishable, callers emit `STOP` and the projection nuance stays in `reason_codes`. |
| AG-UI SSE frame shape (data-only) — **consumed** (resolved by slice 1) | design §7 | Use the slice-1 recorded resolution for demo consumers; `encodeSse` framing (`"data: " + canonical + "\n\n"`) is locked regardless (design §7). |

## Test/verification plan

Delivers TD-01..TD-06 (tests.md §9-§10):

| ID | Gate | Pass condition (tests.md §9) |
|---|---|---|
| TD-01 | `make demo-up DEMO_MODE=ag_ui_events` | exit 0; `ag-ui-runner` healthy; runner log shows the 4 emission lines and the deny-enforced-by-sidecar line |
| TD-02 | `make demo-verify-ag-ui-events` | exit 0 end-to-end (capture + verify_sse.py + ledger join + SQL gate) |
| TD-03 | `verify_sse.py` strictness self-test: feed it a mutated capture (drop `unit_id`, reorder events, blank a `reservation_id`, de-canonicalize whitespace — 4 mutations) | non-zero exit with `COV_D39_GATE:` message for EACH mutation (the gate gates) |
| TD-04 | Counting-stub invariance: `GET /_count` after the run shows exactly 1 call | the DENY step provably never reached the provider; enforcement happened at the sidecar, display at AG-UI |
| TD-05 | Ledger join: `RESERVATION_ID` printed by verify_sse.py exists in `reservations` for the demo tenant | display↔ledger consistency |
| TD-06 | `verify_step_ag_ui_events.sql` | `reserve >= 1`, `commit_estimated >= 1`, `denied_decision >= 1` for the demo tenant; `ON_ERROR_STOP=1` |

TD-03 is mandatory: "a verify script that cannot fail is not a gate" (tests.md §9; demo-as-quality-gate memory; HARDEN_D05_UR removed exactly this class of softening). The four mutated captures and their failure messages are pasted into the slice-3 review thread (acceptance §9 checklist).

## Acceptance gates (slice subset per acceptance.md §8)

```bash
# A5.1  demo up (from deploy/demo/): runner healthy, 4 emission lines + deny-before-provider line in log
make demo-up DEMO_MODE=ag_ui_events
# A5.2  full verify chain: SSE capture → verify_sse.py strict pass → ledger join → SQL gate
make demo-verify-ag-ui-events
# A5.3  mutation drill (TD-03): each of the 4 documented mutated captures fails with COV_D39_GATE:
python3 deploy/demo/ag_ui_events/verify_sse.py <each-mutated-capture>
# A5.4  deny never reached the provider
curl -s http://localhost:<counting-stub-port>/_count        # expect {"calls": 1}
# A5.5  overlay declares ONLY counting-stub + ag-ui-runner + sse-probe; no base-service redeclaration
cat deploy/demo/ag_ui_events/docker-compose.yaml
# A5.6  zero enforcement-via-AG-UI wording in runner log + README + compose comments; deny log line attributes enforcement to the sidecar

# A6.1  TS README: §1.1 notice verbatim, above the fold; quickstart compiles as-is (file landed in slice 1; gate re-runs here)
# A6.3  docs page exists; notice first; five-event table matching §5.2; JSON wrapped is:raw; demo commands present
cat docs/site-v2/src/content/docs/docs/integrations/ag-ui.mdx
# A6.4  repo-root README integrations row, labeled display-only events
# A6.5  all three CHANGELOG entries present; each names the five events and says "display-only"
# A6.6  enforcement-wording grep — empty, or every hit is a negation (reviewer reads each hit)
grep -riE "(enforce|gate|deny|block).{0,40}(via|through|using|with) ag-?ui" \
  sdk/typescript-ag-ui/ sdk/python/src/spendguard/integrations/ag_ui/ \
  docs/site-v2/src/content/docs/docs/integrations/ag-ui.mdx examples/ag-ui-events/ deploy/demo/ag_ui_events/
# A6.7  no upstream PR/issue opened from D39 slices (gh search if in doubt)

# A7.1  D39 never touches the D05 substrate or the wire
git diff --stat main -- sdk/typescript/src proto/            # expect zero changes in D39 commits
# A7.3  D05 corpus untouched
git diff main -- sdk/fixtures/cross-language/v1.json         # expect empty
```

Ship-readiness (acceptance §9) also lands with this slice: fresh-stack back-to-back `demo-down` → `demo-up` → `demo-verify-ag-ui-events`; all `[VERIFY-AT-IMPL]` markers across the three slice docs resolved; `ag_ui_v1.json` frozen (A3.6); the `project_coverage_D39_shipped` memory entry is written when fully green.

## Anti-scope (NOT in this slice)

- **No edits to either package's `src/`** — `sdk/typescript-ag-ui/src/**` and `sdk/python/src/spendguard/integrations/ag_ui/*.py` are frozen for this slice (review-standards §12: "slice 3: no edits to either package's `src/`"). A demo-discovered library bug reopens the owning slice.
- **No upstream ag-ui repo contribution** — vocabulary registration stays follow-on outreach only; no upstream PRs/issues from D39 slices (design §3; acceptance A6.7).
- **No browser UI / frontend rendering components** — the asserted SSE content is the gate, not pixels (design §3, §9 last line).
- **No gating/enforcement claims anywhere** — P0; the deny log line attributes enforcement to the sidecar explicitly, and no demo/docs/compose/CHANGELOG text may state or imply AG-UI enforces, gates, denies, reserves, blocks, or limits spend (design §1.1; review-standards §1; acceptance A5.6/A6.6).
- **No queryBudget RPC work** — slice 3 only *checks* the wire status marker; it does not implement, stub, or extend `QueryBudget` on `adapter.proto` or the substrate placeholder (design §5.3 NB; implementation §11).
- **No `released` demo step** — locked decision design §11.9; documented non-gap, fixture/unit-tested only in v0.1.0.
- **No retro-edits to other deliverables' Makefile targets** (implementation §8).
- **No fabricated IDs, amounts, or calibration-style numbers** — every event field traces to a real RPC outcome or a ledger-cross-checked seed value (review-standards §7.1-§7.2; HARDEN_04 lesson).
- **No gate softening** — loosening `verify_sse.py` assertions to make the demo pass is a Blocker-class finding, not a fix (review-standards §7.4 and the reviewer-prompt softening rule).
- **No npm/PyPI publish, no git tag** (acceptance §9).

## Backlinks

- [`design.md`](../specs/coverage/D39_ag_ui/design.md) — §1.1 notice, §9 demo design (the gate spec), §11.9 four-event lock, §12 slice plan
- [`implementation.md`](../specs/coverage/D39_ag_ui/implementation.md) — §1.3 files, §7 runner, §8 Makefile, §9 verify_sse, §10 docs page
- [`tests.md`](../specs/coverage/D39_ag_ui/tests.md) — §9 TD-01..TD-06, §10 mapping
- [`acceptance.md`](../specs/coverage/D39_ag_ui/acceptance.md) — §5 demo gates, §6 docs gates, A7.1/A7.3, §8 slice subset, §9 ship-readiness
- [`review-standards.md`](../specs/coverage/D39_ag_ui/review-standards.md) — §1 display-only, §7 demo correctness, §8 marker table, §12 sign-off
