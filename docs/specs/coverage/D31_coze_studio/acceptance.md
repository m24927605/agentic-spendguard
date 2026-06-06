# D31 — Coze Studio Model Provider — Acceptance

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`review-standards.md`](review-standards.md).

D31 ships when **every gate** in §1 is green AND the §2 invariants are unbreakable AND the §3 ship checklist is fully checked. Per build-plan §3 "100% feasible" rule: every gate below is runnable in the repo's current state, no third-party action required (Coze upstream PR is not a ship gate, neither is Coze Cloud marketplace anything).

## §1. Hard gates

### G1 — Recipe lint clean (Slice 1)

```bash
python -c "import yaml; yaml.safe_load(open('examples/coze-studio/coze-workspace-config.yaml'))"
yamllint examples/coze-studio/coze-workspace-config.yaml
grep -q "X-SpendGuard-Tenant-Id" examples/coze-studio/coze-workspace-config.yaml
grep -q "X-SpendGuard-Budget-Id" examples/coze-studio/coze-workspace-config.yaml
grep -q "X-SpendGuard-Window-Instance-Id" examples/coze-studio/coze-workspace-config.yaml
! grep -E 'api_key:\s*"?sk-' examples/coze-studio/coze-workspace-config.yaml
```

Expected: all commands exit 0 (the last is a negative grep — no real key in repo).

### G2 — README + cheatsheet completeness (Slice 1)

```bash
grep -c "X-SpendGuard-" examples/coze-studio/README.md           # >= 3
grep -q "Troubleshooting" examples/coze-studio/README.md
grep -c "X-SpendGuard-" examples/coze-studio/headers-cheatsheet.md  # >= 3
test -s examples/coze-studio/README.md
test -s examples/coze-studio/headers-cheatsheet.md
```

Expected: all exit 0; README and cheatsheet non-empty; three required headers documented in both.

### G3 — D09 SLICE 1 HTTP companion is on main (Slice 2 prerequisite)

```bash
git log main --pretty=oneline | grep -qE "COV_D09_01_sidecar_http_companion|D09 SLICE 1"
# OR equivalent: assert the companion endpoint exists in main's tree
test -f services/sidecar/src/server/http_companion.rs
```

Expected: exits 0. If this fails, D31 SLICES 2-4 are blocked (per design.md §6).

### G4 — Smoke against Coze stack (Slice 2)

```bash
export OPENAI_API_KEY=sk-...       # real key, real upstream
bash examples/coze-studio/smoke.sh
```

Expected:

- All 6 smoke assertions (T-SMOKE-01..06 from tests.md §2) pass.
- Exit 0.
- A real OpenAI billable call is observed (audit row's `provider_event_id` is non-empty).
- Sidecar audit row visible in `spendguard_ledger.audit_outbox` with `decision_context->>'integration' = 'coze_studio'`.

### G5 — Existing demos still pass (regression, Slice 3)

```bash
make demo-down
make demo-up DEMO_MODE=decision
make demo-down
make demo-up DEMO_MODE=litellm_real
make demo-down
```

Expected: both exit 0. D31 is purely additive — the existing demo modes' compose files were not edited (Slice 3 lives in `coze_studio/compose.override.yaml`).

### G6 — Coze demo boots and passes (Slice 3, headline acceptance)

```bash
make demo-down       # clean slate
export OPENAI_API_KEY=sk-...
make demo-up DEMO_MODE=coze_studio_real
```

Expected:

- All compose services (Coze Studio + Coze Postgres + Coze Redis + SpendGuard sidecar with HTTP companion + ledger Postgres + canonical ingest + outbox forwarder + tokenizer) reach healthy.
- Demo driver exits 0.
- stdout contains the literal line `[demo] coze_studio_real ALL 3 steps PASS (ALLOW + DENY + STREAM)`.
- stdout contains `D31_COZE OK: coze decisions=N` for `N >= 2`.
- stdout contains the outbox-closure verification line.
- `verify_step_coze_studio_real.sql` all 7 assertions return success (no `RAISE EXCEPTION`).
- `spendguard_verify_chain('coze_studio_real')` returns `t`.

### G7 — Demo mode tears down cleanly (Slice 3)

```bash
make demo-down
docker ps -a | grep -E "coze-studio|coze-postgres|coze-redis" || true
docker volume ls | grep coze || true
```

Expected: no orphaned containers, no orphaned volumes, `make demo-down` exits 0.

### G8 — Public docs page renders (Slice 4)

```bash
cd docs/site && npm run build
test -s dist/docs/integrations/coze-studio/index.html
```

Expected: build succeeds; rendered HTML non-empty.

### G9 — Docs page content gates (Slice 4)

```bash
F=docs/site/docs/integrations/coze-studio.md
grep -q "Install (self-hosted Coze)" $F
grep -q "Decision matrix" $F
grep -q "Limitations" $F
grep -q "Troubleshooting" $F
grep -q "X-SpendGuard-Tenant-Id" $F
grep -q "Coze Cloud" $F
grep -q "Anthropic" $F
```

Expected: all 7 grep commands exit 0. Page covers install, decision matrix, limitations (including Coze Cloud unsupported + Anthropic/Gemini/Bedrock in v1.1), and troubleshooting.

### G10 — README adapter row (Slice 4)

```bash
grep -q "Coze Studio" README.md
grep -q "examples/coze-studio" README.md
```

Expected: exactly one new row in the adapter integrations table referencing Coze Studio and the recipe path.

### G11 — No proto / no Rust / no SDK / no schema drift

```bash
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '\.(proto|rs)$' && echo FAIL || echo OK
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '^sdk/' && echo FAIL || echo OK
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '\.sql$' | grep -v -E '^(deploy/demo/(verify_step_coze_studio_real\.sql|coze_studio/seed_workspace\.sql))$' && echo FAIL || echo OK
```

Expected: all three lines print `OK`. The only new SQL files are the demo verify gate and the Coze workspace seed — both in the demo allow-list. **No Rust, no proto, no SDK changes. No new sidecar endpoint.**

### G12 — No edits to existing demo compose files

```bash
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '^deploy/demo/(compose\.yaml|litellm_proxy/|init/|runtime/)' && echo FAIL || echo OK
```

Expected: prints `OK`. Slice 3 wires via overlay (`coze_studio/compose.override.yaml`), Makefile branch only.

## §2. Invariants (must never regress)

| ID | Invariant | How verified |
|----|-----------|--------------|
| INV-1 | **DENY never hits the upstream provider.** Counting stub in front of `api.openai.com` MUST register zero hits across all DENY decisions in the demo. | `verify_step_coze_studio_real.sql` `stub_hits` assertion + demo driver step 2 + Slice 2 smoke T-SMOKE-04 negative case |
| INV-2 | **Reservation precedes upstream HTTP.** SpendGuard `PRE_LLM_CALL.RESERVE` row exists before any OpenAI billable. | demo driver step 1 ordering + Slice 2 smoke T-SMOKE-03 |
| INV-3 | **Fail-closed default.** Sidecar DENY/DEGRADE → Coze sees real HTTP error (502 / 503). No `fail_open` flag in the v1 snippet. | Slice 1 anti-regression grep + demo driver step 2 |
| INV-4 | **Tenant required.** Missing `X-SpendGuard-Tenant-Id` → companion returns 400. No "default tenant" silent allow. | Slice 2 smoke T-SMOKE-04 |
| INV-5 | **End-of-stream commit uses real usage when present.** Inherits from D09 SLICE 1 companion; D31 demo step 3 proves it lights up. | demo step 3 + SQL `stream=true` assertion |
| INV-6 | **Secret hygiene.** No `OPENAI_API_KEY` literal anywhere in `examples/coze-studio/`. Snippet uses `${OPENAI_API_KEY}` form only. | G1 negative grep + Slice 1 anti-regression |
| INV-7 | **No sidecar code drift.** D31 does NOT touch `services/sidecar/`, `crates/`, `sdk/`, or `proto/`. Recipe + docs + demo overlay only. | G11 |
| INV-8 | **Coze DB isolation.** Coze stack uses a separate database name (`coze_db`) on the shared Postgres instance — never co-tenants with `spendguard_ledger`. | demo `compose.override.yaml` env-var check + Slice 3 implementation §2 |
| INV-9 | **Image pin discipline.** `docker-compose.coze.yaml` pins Coze Studio by SHA256 digest, not floating tag. | Slice 2 grep `@sha256:` |

## §3. Ship checklist

```
[ ] G1 recipe lint clean
[ ] G2 README + cheatsheet completeness
[ ] G3 D09 SLICE 1 HTTP companion is on main
[ ] G4 examples/coze-studio/smoke.sh passes against real Coze + real OpenAI
[ ] G5 existing demo modes unbroken (decision + litellm_real)
[ ] G6 make demo-up DEMO_MODE=coze_studio_real exits 0 + success lines printed
[ ] G7 make demo-down clean (no orphan containers/volumes)
[ ] G8 docs/site build succeeds + coze-studio page renders
[ ] G9 docs page covers Install / Decision matrix / Limitations / Troubleshooting
[ ] G10 README adapter row landed
[ ] G11 no proto / Rust / SDK / unauthorised SQL drift
[ ] G12 no edits to existing demo compose files
[ ] INV-1 .. INV-9 all green
[ ] All 4 slices merged in order S1 → S2 → S3 → S4 with R1-R5 review loop per build-plan §1.1
[ ] Memory write-back entry project_coverage_D31_shipped.md drafted per build-plan §8
```

## §4. Definition of done (per build-plan §7)

- All 4 slices merged into main.
- Acceptance gates G1..G12 + invariants INV-1..INV-9 green.
- README adapter row landed.
- `docs/site/docs/integrations/coze-studio.md` live.
- `Makefile` `DEMO_MODE=coze_studio_real` branch live.
- `examples/coze-studio/` recipe tree complete (config snippet + cheatsheet + smoke + compose).
- Memory entry written per build-plan §8 (single paragraph: merge commit + round count + arbitration y/n + closed issues).

## §5. Out-of-scope explicit declarations

D31 v1 does NOT close any of:

- Native Coze plugin SDK (Pattern 3 — Go plugin tool intercepting LLM calls intra-process). Tracked as v1.1 GH issue at ship time.
- Anthropic / Gemini / Bedrock provider slots in Coze. Tracked as v1.1.
- Mid-stream token cap enforcement (end-of-stream only — inherits D09 §3.3).
- Coze Cloud (SaaS) integration. Coze Cloud's provider config is gated and we cannot validate the gate without a Coze partnership.
- Multi-tenant federation across multiple Coze workspaces via a single sidecar. Per-workspace snippet only.
- Upstream PR to Coze for native SpendGuard recognition. Recipe lives in our repo.

These are documented in `docs/site/docs/integrations/coze-studio.md` "Limitations" section so operator expectation matches the ship surface (G9 enforces).

## §6. Risk register

| Risk | Mitigation |
|------|-----------|
| D09 SLICE 1 has not landed when D31 starts | Slice 1 (snippet + docs) ships; Slices 2-4 block. G3 enforces. |
| Coze Studio image tag drifts between spec-write and ship | Compose pins by SHA256 digest. Smoke (G4) is the canary. |
| Coze Studio breaks the OpenAI-compatible base-URL contract in a minor version | Pin to a tested digest; new digest goes through Slice 2 smoke before bump. Bump tracked as a "pin and bump" follow-up. |
| Coze workspace YAML schema changes | Snippet is documented as "v1 schema"; doc lists the verified Coze version range. Schema change tracked as v1.x bump. |
| Demo stack heavy (~1.5 GB Coze image) | `DEMO_MODE=coze_studio_real` is opt-in only; not in default `make demo-up`. CI runs it on tagged builds only. |
| Operator pastes the wrong workspace ID into tenant header | Companion returns 400 on missing/malformed tenant (INV-4); error is surfaced through Coze's UI. |
| Operator wants fail-open in v1 | Docs decision matrix steers them to D02/D03 egress-proxy install instead. No `fail_open` in v1 D31 snippet (INV-3). |
