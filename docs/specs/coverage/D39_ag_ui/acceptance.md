# D39 — Acceptance Gates

Gates a reviewer (`superpowers:code-reviewer`) re-runs to confirm D39 is
shipped. Every gate is runnable in this repo (commands assume repo root unless
a directory is given). No gate depends on a third-party action SpendGuard
cannot trigger; no gate depends on the AG-UI upstream repo.

## 1. TS build + lint + typecheck (`sdk/typescript-ag-ui/`)

| Gate | Command | Pass condition |
|---|---|---|
| A1.1 | `npm install` (or the workspace's package-manager equivalent used by `sdk/typescript-langchain/`) | exit 0 |
| A1.2 | `npm run lint` | biome zero diagnostics |
| A1.3 | `npm run typecheck` | exit 0 (src AND tests tsconfigs) |
| A1.4 | `npm run build` | tsup emits `dist/index.js` + `dist/index.d.ts` |
| A1.5 | `npm run size` | minified ≤ 8 KB; gz ≤ 3 KB; breach = non-zero exit |
| A1.6 | `grep -E "from ['\"](node:|@ag-ui/|@spendguard/sdk)" -r src/ \|\| true` then `grep -cE "node:|@ag-ui/core" dist/index.js` | both empty / `0` — zero runtime deps, no node builtins, AG-UI never imported at runtime |
| A1.7 | `cat package.json \| python3 -c "import json,sys; p=json.load(sys.stdin); assert 'dependencies' not in p or not p['dependencies']; assert p['peerDependenciesMeta']['@ag-ui/core']['optional'] is True; print('OK')"` | `OK` — zero deps + optional peer locked |

## 2. Test gates

| Gate | Command | Pass condition |
|---|---|---|
| A2.1 | `cd sdk/typescript-ag-ui && npm run test` | vitest exit 0; coverage ≥ 92 % stmt / 88 % branch (tests.md §1) |
| A2.2 | `cd sdk/typescript-ag-ui && npx vitest run tests/builders.test.ts tests/validate.test.ts` | TP-01..TP-19 all pass |
| A2.3 | `cd sdk/typescript-ag-ui && npx vitest run tests/canonical.test.ts tests/sse.test.ts` | TP-20..TP-26 all pass |
| A2.4 | `cd sdk/typescript-ag-ui && npx vitest run tests/agUiCompat.test.ts tests/bundle.test.ts` | TP-28..TP-31 pass under the EXACT pinned `@ag-ui/core` devDep |
| A2.5 | `cd sdk/python && make test` | pytest exit 0 including `tests/integrations/ag_ui/` (TA-01..TA-28) |
| A2.6 | Fresh venv WITHOUT extras: `python3 -m venv /tmp/d39-noextra && /tmp/d39-noextra/bin/pip install -e sdk/python && /tmp/d39-noextra/bin/python -c "import spendguard.integrations.ag_ui as m; print(sorted(m.__all__)[0])"` | imports clean with NO `ag-ui-protocol` installed (zero-extra path) |
| A2.7 | `pip install -e 'sdk/python[ag-ui]'` then `python -m pytest sdk/python/tests/integrations/ag_ui/test_ag_ui_compat.py -q` | TA-28 runs (not skipped) and passes |

## 3. Cross-language byte-equivalence (P0)

| Gate | Command / path | Pass condition |
|---|---|---|
| A3.1 | `sdk/fixtures/cross-language/ag_ui_v1.json` | exists, committed; ≥ 20 vectors; every builder covered; vector matrix of tests.md §6 satisfied (spot-check the named vectors: `timestamp_ms: 0`, all 5 `denied_kind`, all 4 `outcome`, Unicode set) |
| A3.2 | `cd sdk/typescript-ag-ui && npx vitest run tests/crossLanguage.test.ts` | TP-27 green — TS == corpus, byte-for-byte |
| A3.3 | `cd sdk/python && python -m pytest tests/integrations/ag_ui/test_cross_language.py -q` | TA-27 green — Python == same corpus |
| A3.4 | `grep -c '"unit_id":""' sdk/fixtures/cross-language/ag_ui_v1.json` | `0` — empty unit_id never serialized (HARDEN_D05_UR) |
| A3.5 | Manual: pick 3 random vectors; recompute the canonical bytes with `python3 -c "import json,sys; print(json.dumps(json.loads(sys.argv[1]), ensure_ascii=False, sort_keys=True, separators=(',',':')))"` against `expected_canonical_json` | byte-identical — corpus is honest |
| A3.6 | `git log --follow --oneline sdk/fixtures/cross-language/ag_ui_v1.json` after slice 2+ | exactly one content-creating commit (slice 1); zero in-place edits afterward |

## 4. Public-surface gates

| Gate | Command | Pass condition |
|---|---|---|
| A4.1 | `cd sdk/typescript-ag-ui && npm pack` then install the tarball in a scratch dir and run `node -e 'import("@spendguard/ag-ui").then(m => console.log(Object.keys(m).sort().join(",")))'` | exactly: `AgUiEventValidationError,SPENDGUARD_AG_UI_EVENT_NAMES,VERSION,buildBudgetSnapshot,buildDecisionDenied,buildReservationCommitted,buildReservationCreated,buildReservationReleased,canonicalEventJson,encodeSse` (types are type-only) |
| A4.2 | `tar -tzf spendguard-ag-ui-0.1.0.tgz \| grep -E "src/\|tests/\|node_modules"` | empty — only dist + README + LICENSE_NOTICES + CHANGELOG ship; tarball ≤ 25 KB |
| A4.3 | `python3 -c "import spendguard.integrations.ag_ui as m; assert set(m.SPENDGUARD_AG_UI_EVENT_NAMES) == {'spendguard.budget.snapshot','spendguard.reservation.created','spendguard.reservation.committed','spendguard.reservation.released','spendguard.decision.denied'}; print('OK')"` | `OK` — vocabulary lock, both languages (TS twin asserted in TP-01) |
| A4.4 | diff the §5 payload tables in `design.md` against the emitted key sets in the corpus vectors (script or manual) | zero drift — design.md §5 is the wire truth |

## 5. Demo gates (slice 3 — `DEMO_MODE=ag_ui_events`)

| Gate | Command | Pass condition |
|---|---|---|
| A5.1 | `make demo-up DEMO_MODE=ag_ui_events` (from `deploy/demo/`) | exit 0; `ag-ui-runner` healthy; runner log contains the 4 emission lines AND the line proving deny-before-provider (counting-stub counter unchanged) |
| A5.2 | `make demo-verify-ag-ui-events` | exit 0: SSE capture → `verify_sse.py` strict pass (exactly 4 frames, exact order, required fields non-empty, unit_id present, canonical bytes) → ledger reservation join → `verify_step_ag_ui_events.sql` |
| A5.3 | Mutation drill (TD-03): run `python3 deploy/demo/ag_ui_events/verify_sse.py` against each of the 4 documented mutated captures | non-zero exit + `COV_D39_GATE:` message for every mutation — the gate can fail |
| A5.4 | `curl -s http://localhost:<counting-stub-port>/_count` post-run (or compose-exec equivalent) | `{"calls": 1}` — the DENY step never reached the provider |
| A5.5 | `deploy/demo/ag_ui_events/docker-compose.yaml` | overlay declares ONLY counting-stub + ag-ui-runner + sse-probe; no base-service redeclaration (overlay-independence convention) |
| A5.6 | Runner log + README + compose comments | zero enforcement-via-AG-UI wording (P0 — review-standards §1); the deny log line explicitly attributes enforcement to the sidecar |

## 6. Documentation gates

| Gate | Path | Pass condition |
|---|---|---|
| A6.1 | `sdk/typescript-ag-ui/README.md` | display-only notice from design.md §1.1 verbatim, above the fold; quickstart compiles as-is |
| A6.2 | `sdk/python/src/spendguard/integrations/ag_ui/__init__.py` docstring | same notice verbatim |
| A6.3 | `docs/site-v2/src/content/docs/docs/integrations/ag-ui.mdx` | exists; notice first; five-event table matching §5.2; embedded JSON wrapped `is:raw`; demo commands present |
| A6.4 | Repo-root `README.md` integrations table | `@spendguard/ag-ui` row present, labeled display-only events |
| A6.5 | `sdk/typescript-ag-ui/CHANGELOG.md` `0.1.0` + `sdk/python/CHANGELOG.md` entry + repo-root `CHANGELOG.md` entry | all three present; each names the five events and says "display-only" |
| A6.6 | `grep -riE "(enforce\|gate\|deny\|block).{0,40}(via\|through\|using\|with) ag-?ui" sdk/typescript-ag-ui/ sdk/python/src/spendguard/integrations/ag_ui/ docs/site-v2/src/content/docs/docs/integrations/ag-ui.mdx examples/ag-ui-events/ deploy/demo/ag_ui_events/` | empty, or every hit is a negation ("can NOT gate") — reviewer reads each hit |
| A6.7 | Outreach note | design.md §3 keeps upstream vocabulary registration as follow-on outreach; NO upstream PR/issue opened from D39 slices (`gh` search if in doubt) |

## 7. Substrate / repo invariants (P0)

| Gate | Command | Pass condition |
|---|---|---|
| A7.1 | `git diff --stat main -- sdk/typescript/src proto/` scoped to D39 commits | zero changes — D39 never touches the D05 substrate or the wire |
| A7.2 | `grep -rE "blake2b\|createHash\|hashlib\|crypto\.subtle" sdk/typescript-ag-ui/src sdk/python/src/spendguard/integrations/ag_ui/` | empty — no new hashing; IDs are inputs (design §11.6) |
| A7.3 | `git diff main -- sdk/fixtures/cross-language/v1.json` | empty — the D05 corpus is untouched |
| A7.4 | `grep -rn "Date.now\|Math.random\|process.env" sdk/typescript-ag-ui/src/` and `grep -rn "time\.\|random\.\|os.environ" sdk/python/src/spendguard/integrations/ag_ui/_builders.py sdk/python/src/spendguard/integrations/ag_ui/_canonical.py` | empty — purity (clock/RNG/env-free) |

## 8. Slice-level acceptance subset

| Slice | Subset |
|---|---|
| `COV_D39_01_ts_pkg` | A1.1-A1.7, A2.1-A2.4, A3.1-A3.2, A3.4-A3.5, A4.1-A4.2, A7.2 (TS half), A7.4 (TS half) |
| `COV_D39_02_py_mirror` | A2.5-A2.7, A3.3, A3.6, A4.3, A6.2, A7.2/A7.4 (Python half); pyproject `ag-ui` extra present |
| `COV_D39_03_demo_docs` | A5.1-A5.6, A6.1, A6.3-A6.7, A7.1, A7.3 |

## 9. Ship-readiness checklist

- [ ] Every gate in §1-§7 green.
- [ ] `make demo-up DEMO_MODE=ag_ui_events` + `make demo-verify-ag-ui-events`
      run clean back-to-back on a fresh stack (`make demo-down` first).
- [ ] Mutation drill (A5.3) demonstrated at least once in the slice-3 review
      round (paste the 4 failure messages into the review thread).
- [ ] All `[VERIFY-AT-IMPL: ...]` markers from design.md / implementation.md /
      tests.md resolved in the slice docs with the verified value recorded
      (the marker list lives in review-standards §8; an unresolved marker at
      ship time is a P1).
- [ ] `ag_ui_v1.json` frozen — A3.6 history check.
- [ ] No git tag / npm publish in D39 scope unless the orchestrator says so —
      version `0.1.0` is prepared, publish workflow wiring follows the house
      OIDC pattern but the publish action itself is a release decision.

When fully green D39 is **shipped**; write the `project_coverage_D39_shipped`
memory entry per the coverage build-plan convention.
