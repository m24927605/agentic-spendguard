# D39 — Review Standards

Use this checklist with the `superpowers:code-reviewer` Claude Code subagent on
every D39 slice (reviewer policy memo 2026-06-08: Claude Code CLI reviewer
exclusively for coverage Phase 4+; max 5 rounds per slice; findings are fixed
for real, not deferred; Staff+ panel arbitration if still blocked at R5).
Findings are categorised P0 / P1 / P2 / Polish; P0 + P1 block.

## 1. Display-only claim discipline (P0 — Blocker)

AG-UI sits between agent backend and frontend, post-decision. It can NOT gate.
Any artifact implying otherwise is a Blocker, full stop.

| Check | Pass condition |
|---|---|
| 1.1 | No code, comment, docstring, README, docs page, CHANGELOG entry, demo log line, compose comment, or commit message states or implies that AG-UI events enforce, gate, deny, reserve, block, or limit spend |
| 1.2 | The design.md §1.1 display-only notice appears VERBATIM in: TS README, Python `__init__` docstring, docs site page, `examples/ag-ui-events/README.md` |
| 1.3 | The demo's deny log line attributes enforcement to the sidecar explicitly (acceptance A5.6) |
| 1.4 | Docs state that `spendguard.*` AG-UI events are unsigned UI hints and MUST NOT be treated as the audit chain (design §5.8 last row) |
| 1.5 | Run acceptance A6.6 grep; every hit is a negation |

## 2. Verbatim event schema (P0 — Blocker)

design.md §5 is a verbatim contract. **LOCKED design.md trumps slice docs**
(coverage build-plan §1.2 P0; the D05/7 slice-author bug pattern was exactly a
slice doc drifting from the locked design — check the design first, then the
diff).

| Check | Pass condition |
|---|---|
| 2.1 | The five event-name strings match §5.2 byte-for-byte; no sixth name exists anywhere |
| 2.2 | Each builder emits exactly the §5.3-§5.7 key set — no extra keys, no renamed keys, no `camelCase` leakage into payloads |
| 2.3 | `schema_version: "1"` injected by every builder; `decision: "DENY"` injected by the denied builder |
| 2.4 | ASP-verbatim fields are spelled exactly as Draft-01 spells them (`reason_codes`, `reservation_id`, `decision_id`, `ttl_expires_at`, `amount_atomic_reserved`, `event_time`, `budget_id`, `window_instance_id`, `unit`) |
| 2.5 | The committed event uses `amount_atomic_estimated` (NOT `amount_atomic_observed`) for the estimated lane; `amount_atomic_observed` only passes through when explicitly supplied |
| 2.6 | Envelope is `{type, name, value}` + optional `timestamp` only; `rawEvent` never emitted |
| 2.7 | Any schema change in the diff (key add/rename/remove, enum value change) without a design.md revision in the same review → Blocker, no exceptions |

## 3. No new hashing / no ID minting (P0 — Blocker)

| Check | Pass condition |
|---|---|
| 3.1 | acceptance A7.2 grep is empty — no hash primitive anywhere in D39 source |
| 3.2 | No UUID generation in builders (`newUuid7`, `crypto.randomUUID`, `uuid.uuid*` absent from `src/` and `_*.py`) — every ID is an input |
| 3.3 | BLAKE2b / idempotency-key / prompt-hash code paths untouched (A7.1, A7.3) — those P0s live in the D05 substrate |

## 4. Cross-language byte-equivalence (P0 — Blocker)

| Check | Pass condition |
|---|---|
| 4.1 | TP-27 and TA-27 consume the SAME `ag_ui_v1.json` file; no per-language fixture forks |
| 4.2 | Corpus has ≥ 20 vectors and covers the tests.md §6 matrix (verify the named vectors exist: `timestamp_ms: 0`, 5 denied_kinds, 4 outcomes, Unicode set, 40-digit amount) |
| 4.3 | Corpus frozen after slice 1 (A3.6 git history); slice 2 contains ZERO corpus edits — a "needed" edit reopens slice 1 |
| 4.4 | The §7 canonical rule is implemented as locked: sorted keys, `,`/`:` separators, `ensure_ascii=False`-equivalent output, ASCII-only keys enforced, null/float/-0/unsafe-int/unpaired-surrogate rejection on BOTH sides |
| 4.5 | Validation regexes (`requireAtomic`, RFC 3339) are character-identical between `validate.ts` and `_validate.py` |
| 4.6 | `encodeSse` framing identical: `"data: " + canonical + "\n\n"` |

## 5. Builder purity + API surface (P0/P1)

| Check | Sev | Pass condition |
|---|---|---|
| 5.1 | P0 | No clock, RNG, env, filesystem, or network access in builders/serializer (A7.4) |
| 5.2 | P0 | Public surface matches design.md §8 verbatim — TS export list exactly A4.1's set; Python `__all__` exactly implementation §5.3's list; no default export |
| 5.3 | P1 | Inputs not mutated; returned TS events frozen (TP/TA-12) |
| 5.4 | P1 | Every TP-NN (01-27) has its TA-NN twin and vice versa (tests.md numbering rule) |
| 5.5 | P1 | Empty-string optionals and absent optionals serialize identically (omit) — uniform §6 rule, with `unit_id` explicitly tested on all four unit-bearing events (TP/TA-14) |
| 5.6 | P1 | `AgUiEventValidationError.field` names the payload key (snake_case), not the input property |

## 6. Dependency + packaging hygiene (P1)

| Check | Pass condition |
|---|---|
| 6.1 | `@ag-ui/core` / `ag-ui-protocol` never imported at runtime; optional peer + extra mechanics per implementation §6 |
| 6.2 | TS `dependencies` block absent/empty; `peerDependenciesMeta.optional` true (A1.7) |
| 6.3 | Exact devDep / test-dep pins for the compat suites; pins recorded in the slice doc with the `[VERIFY-AT-IMPL]` resolution |
| 6.4 | Bundle budget enforced as a build failure: ≤ 8 KB min / ≤ 3 KB gz / ≤ 25 KB tarball |
| 6.5 | No `node:` imports in `src/` (browser-safe); `sideEffects: false` |
| 6.6 | Python module imports clean with zero extras (A2.6); no import-time extras guard |

## 7. Demo correctness (P1)

| Check | Pass condition |
|---|---|
| 7.1 | The demo run is REAL: handshake + reserve + commit against the live sidecar UDS; events carry the real `decision_id`/`reservation_id` from those RPCs (no fabricated IDs, no fabricated amounts — HARDEN_04 lesson) |
| 7.2 | Snapshot inputs come from seed env values that the SQL/ledger gates corroborate; if the queryBudget wire shipped meanwhile, the demo uses it (design §5.3 marker) |
| 7.3 | `verify_sse.py` asserts the FULL design §9 list with exact counts/order and exits non-zero on first failure; it does NOT import the library under test |
| 7.4 | Mutation drill (TD-03/A5.3) present and demonstrated — a gate that cannot fail is a Blocker-level softening (HARDEN_D05_UR class) |
| 7.5 | Counting-stub invariance assertion present (deny never reached provider) |
| 7.6 | Ledger join: SSE `reservation_id` exists in `reservations` (A5.2) |
| 7.7 | Overlay declares only its own services; Makefile branches mirror the langchain_ts pattern; no edits to other deliverables' targets |

## 8. `[VERIFY-AT-IMPL]` marker resolution (P1)

The LOCKED docs carry these markers; each slice that touches the marked area
MUST resolve its markers and record the verified value in the slice doc.
Inventing a value without verification is a P0 (it forges a contract detail).

| Marker | Where | Resolving slice |
|---|---|---|
| AG-UI BaseEvent `timestamp` field name + epoch-ms semantics | design §5.1 | 1 |
| `@ag-ui/core` exact devDep pin + peer-range floor | design §10.2, impl §2 | 1 |
| `prepublishOnly` cross-package script path viability | impl §2 | 1 |
| `@ag-ui/core` CustomEvent type path + runtime schema existence | tests TP-28/29 | 1 |
| AG-UI SSE frame shape (data-only) | design §7 | 1 (3 consumes) |
| `ag-ui-protocol` latest version / extra range / test pin | design §10.2, impl §1.2 | 2 |
| Python `CustomEvent` import path | tests TA-28 | 2 |
| `DecisionStopped` STOP vs STOP_RUN_PROJECTION distinguishability | design §5.7 | 2 (Python err class) + 3 (demo mapping) |
| `reservations` PK column for the ledger join | design §9.3, impl §8 | 3 |
| queryBudget wire status at demo time | design §5.3 | 3 |

## 9. Documentation (P2)

| Check | Pass condition |
|---|---|
| 9.1 | Docs page JSON examples wrapped `is:raw` (Astro memory); examples byte-match corpus vectors where feasible |
| 9.2 | ASP mapping table (design §5.8) reproduced or linked on the docs page |
| 9.3 | READMEs note the 0.x churn-isolation stance (design §10) so consumers understand why `@ag-ui/core` is optional |
| 9.4 | Upstream vocabulary registration documented as follow-on outreach only |

## 10. Findings categorisation

| Category | Definition | Action |
|---|---|---|
| **P0** | Enforcement claim, schema drift vs design §5, new hashing/ID minting, cross-language byte drift, purity break, public-surface drift, fabricated `[VERIFY-AT-IMPL]` resolution | Block. Fix before re-run. |
| **P1** | Missing test/twin, dep hygiene, budget breach, demo gate softness, unresolved marker at ship | Block. Fix before re-run. |
| **P2** | Docs polish, naming nits inside private modules | Track as residual; may merge with note. |
| **Polish** | Wording preferences | Residual; never blocks. |

## 11. Escalation

- Same finding open two consecutive rounds without progress → Staff+ panel
  arbitration (hardening-workflow memo).
- Any P0 open at R5 → automatic Staff+ arbitration.
- Deferred P2/Polish → `gh issue` titled `[D39 residual] ...` referencing the
  slice doc, using the house residual template (slice / round / category /
  spec ref / repro / why deferred / suggested follow-up).

## 12. Sign-off

Sign off a slice only when: every P0+P1 in §1-§8 green; the slice's acceptance
subset (acceptance.md §8) green; slice anti-scope honored (slice 1: no demo or
Python files; slice 2: no corpus edits, no TS src edits; slice 3: no edits to
either package's `src/`); all findings resolved or filed as residuals.

## Reviewer prompt template

Use this EXACT prompt for the `superpowers:code-reviewer` subagent, replacing
the `{...}` placeholders. R1 uses the full text; R2-R5 replace the scope
sentence as noted below.

```
You are reviewing slice {SLICE_ID} of coverage deliverable D39 (AG-UI
spend-event family) in /Users/michael.chen/products/agentic-spendguard.

Read, in this order, BEFORE the diff:
1. docs/specs/coverage/D39_ag_ui/design.md          (LOCKED — the contract)
2. docs/specs/coverage/D39_ag_ui/review-standards.md (this checklist)
3. docs/specs/coverage/D39_ag_ui/tests.md and acceptance.md (the gates)
4. The slice doc for {SLICE_ID}
Then review the diff: git diff {BASE_REF}...{HEAD_REF}

Precedence rule (P0): the LOCKED design.md trumps the slice doc and trumps
the implementation. If the slice doc or code disagrees with design.md §5
(event names, payload keys, enums), the design is right and the code/slice
doc is the finding.

Check every item in review-standards.md sections 1-9 that the diff touches.
Non-negotiable P0 invariants — treat ANY violation as a Blocker:
  (a) Display-only discipline: no code/doc/log/comment may state or imply
      that AG-UI events enforce, gate, deny, or block spend. AG-UI is
      presentation-side; enforcement is the sidecar's. Grep the diff for
      enforcement wording near "AG-UI"/"ag_ui" and read every hit.
  (b) Verbatim schema: emitted event names and payload keys must match
      design.md §5.2-§5.7 byte-for-byte; ASP field spellings exact;
      committed events use amount_atomic_estimated, never relabeled.
  (c) No new hashing, no ID minting: builders receive every ID as input;
      any hash/uuid primitive in D39 source is a Blocker.
  (d) Cross-language byte-equivalence: TS and Python canonical output must
      match the frozen sdk/fixtures/cross-language/ag_ui_v1.json corpus;
      any in-place corpus edit after slice 1 is a Blocker.
  (e) unit_id invariant (HARDEN_D05_UR): empty/absent unitId means the
      unit_id key is OMITTED; "unit_id":"" anywhere is a Blocker.
  (f) Builder purity: no clock/RNG/env/IO in builders or serializer.

Also verify: every [VERIFY-AT-IMPL] marker assigned to this slice
(review-standards §8) is resolved with an actually-verified value (cite the
evidence in the slice doc); test twins TP-NN/TA-NN are symmetric; demo gates
can FAIL (mutation drill exists, slice 3 only).

Run what you can run (build, tests, greps from acceptance.md sections that
apply to this slice) rather than trusting the diff narrative. Evidence
before assertions.

Output format — a findings list, most severe first:
  [Blocker|Major|minor] file:line — one-sentence finding; spec ref
  (design.md §N / review-standards §N / tests.md TP/TA-NN); concrete fix.
Use Blocker for P0, Major for P1, minor for P2/Polish. If a category has no
findings, state "none". End with verdict: PASS (no Blocker/Major) or
BLOCK (list the blocking finding IDs), plus the acceptance-subset gates you
executed and their exit status.
```

Round-scope substitutions:
- **R1**: as written (full checklist).
- **R2-R5**: replace the sentence "Check every item in review-standards.md
  sections 1-9 that the diff touches." with "Re-verify ONLY the open findings
  from round {N-1} (listed below) plus any code the fixes touched; confirm
  each finding is truly fixed — not deferred, not softened — and check the
  fixes introduced no new P0 violations of (a)-(f). Open findings:
  {PASTE_FINDINGS}".
- A finding "fixed" by weakening a gate (loosening an assertion, widening a
  regex, deleting a test) is NOT fixed — flag it as a new Blocker citing
  review-standards §7.4.
