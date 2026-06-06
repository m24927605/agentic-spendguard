# D10 — Acceptance

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`review-standards.md`](review-standards.md).

D10 ships when **every gate** in §1 is green AND the §2 invariants are unbreakable AND the §3 ship checklist is fully checked. Per build-plan §3 "100% feasible" rule: every gate below is runnable in the repo's current state, no third-party action required (Dify Cloud marketplace push is gated behind a secret-availability check, not a hard ship gate).

## 1. Hard gates

### G1 — Plugin scaffold validates

```bash
cd plugins/dify && python -m dify_plugin.cli check
```

Expected: exits 0, reports `manifest.yaml` + `provider/spendguard.yaml` + `models/llm/spendguard.yaml` schema-valid.

### G2 — Plugin daemon imports cleanly

```bash
cd plugins/dify && python -c "from models.llm.llm import SpendGuardLLM; from provider.spendguard import SpendGuardProvider; print(SpendGuardLLM.__name__, SpendGuardProvider.__name__)"
```

Expected: prints `SpendGuardLLM SpendGuardProvider`. No `ImportError`.

### G3 — Unit suite

```bash
cd plugins/dify && pytest tests/test_provider.py tests/test_reservation.py tests/test_openai_invoke.py tests/test_anthropic_invoke.py tests/test_streaming.py -v
```

Expected: 39 tests pass (count from `tests.md` §2; final count may rise during implementation but never fall below 39).

### G4 — Plugin daemon integration suite (in-tree, no Dify core)

```bash
cd plugins/dify && pytest tests/test_plugin_daemon_e2e.py -v
```

Expected: 4 tests pass. Subprocess plugin daemon boots, fake sidecar + respx upstream both observe correct ordering.

### G5 — Existing demos still pass (regression)

```bash
make demo-down
make demo-up DEMO_MODE=decision
make demo-down
make demo-up DEMO_MODE=litellm_real
```

Expected: both demos still exit 0. D10 is purely additive — the existing demo modes' compose files were not edited (Slice 7 lives in an overlay file).

### G6 — Dify demo boots and passes

```bash
make demo-down       # clean slate
make demo-up DEMO_MODE=dify_plugin_real
```

Expected:

- All compose services (Dify api + worker + plugin daemon + SpendGuard sidecar + Postgres + ledger + canonical ingest + outbox forwarder) reach healthy.
- Demo driver exits 0.
- stdout contains `[demo] dify_plugin_real ALL 3 steps PASS (ALLOW + DENY + STREAM)`.
- stdout contains `D10_DIFY OK: dify decisions=N` for `N >= 2`.
- stdout contains the outbox-closure verification line.

### G7 — Demo mode tear-down clean

```bash
make demo-down
```

Expected: no orphaned containers, no orphaned volumes, exit 0.

### G8 — Public docs page renders

```bash
cd docs/site && npm run build
```

Expected: build succeeds. `docs/site/dist/docs/integrations/dify/index.html` exists. Page contains:

- "Install (Dify Cloud)" section with `dify plugin install` command.
- "Install (Self-hosted)" section with `.difypkg` sideload command + a compose-mount snippet for the plugin daemon container.
- Decision matrix comparing the Dify plugin path vs the egress-proxy path vs the LiteLLM-routed path.
- "Limitations" section explicitly listing: no workflow-step gating, no token-by-token cap, Gemini/Bedrock upstreams deferred to v1.1.

### G9 — README index entry present

```bash
grep -F "Dify Model Provider" README.md
```

Expected: exactly one row in the adapter integrations table that includes the `dify plugin install spendguard.difypkg` command.

### G10 — Plugin package builds

```bash
cd plugins/dify && python -m dify_plugin.cli package -o /tmp/spendguard.difypkg
```

Expected: produces `/tmp/spendguard.difypkg` (≥ 200 KB), zero `dify plugin check` warnings on the artefact.

### G11 — Publish workflow lints clean

```bash
gh workflow view dify-plugin-publish.yml --yaml | yq '.jobs' >/dev/null
actionlint .github/workflows/dify-plugin-publish.yml
```

Expected: no lint errors. Workflow runs only on `dify-plugin-v*` tags; not on PR CI. The workflow's marketplace-push step is conditional on `secrets.DIFY_MARKETPLACE_TOKEN` being present, so absence on PR CI is not a failure.

### G12 — No proto / no DB-schema / no Rust changes (purely additive)

```bash
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '\.(proto|rs)$' && echo FAIL || echo OK
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '\.sql$' | grep -v -E '^(deploy/demo/(verify_step_dify_plugin\.sql|dify_plugin/seed_workspace\.sql))$' && echo FAIL || echo OK
```

Expected: both lines print `OK`. The only new SQL files are the demo verify gate and the Dify workspace seed — both in the demo allow-list.

### G13 — No `spendguard-sdk` mutations

```bash
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '^sdk/python/src/spendguard/integrations/' && echo FAIL || echo OK
```

Expected: prints `OK`. D10 lives entirely under `plugins/dify/` + `deploy/demo/` + `docs/site/` + `.github/workflows/` + `README.md`. The SDK's existing integrations folder is untouched.

## 2. Invariants (must never regress)

| ID | Invariant | How verified |
|----|-----------|--------------|
| INV-1 | **DENY never hits the upstream provider.** Counting stub MUST register zero hits across all DENY decisions in the demo. | `verify_step_dify_plugin.sql` `stub_hits` assertion + O02 + E03 + demo step 2 |
| INV-2 | **Pre-call reservation precedes upstream HTTP.** RequestDecision fires before any outbound to `api.openai.com` / `api.anthropic.com`. | O01 + E02 strict-ordering `asyncio.Event` |
| INV-3 | **Fail-closed default.** Sidecar DEGRADE → Dify HTTP 503 (`InvokeServerUnavailableError`). Only `SPENDGUARD_DIFY_FAIL_OPEN=1` permits otherwise. | R06 + R07 + demo step 2 + manual DEGRADE injection |
| INV-4 | **`validate_credentials` is a SpendGuard-side probe, not only an upstream probe.** Catches sidecar misconfig at install time, before the first runtime call. | P02 |
| INV-5 | **End-of-stream commit uses real usage when present.** Upstream's `usage` is what lands in the commit row; estimator-fallback only fires when usage is missing and logs a WARN. | S02 + S03 + demo step 3 |
| INV-6 | **No mutation of operator credentials in logs.** No log line contains `upstream_api_key`, `master_key`, or substrings of secrets. | Linter rule + code-reviewer checklist + manual log scrape in demo |
| INV-7 | **Idempotency across retries.** Two reservations for the same `(workspace_id, app_id, message_id, retry)` tuple dedupe sidecar-side; no double-charge under Dify's retry middleware. | R11 + demo retry probe |
| INV-8 | **Plugin daemon does NOT mutate Dify core or other plugins.** Daemon runs in its own process; its dependencies don't get into Dify core's Python env. | G6 (Dify daemon image hash unchanged) + demo logs absence of import side-effects |
| INV-9 | **No Rust/proto/SDK drift.** D10 is purely additive at the plugin tree. | G12 + G13 |

## 3. Ship checklist

```
[ ] G1 plugin scaffold validates
[ ] G2 plugin daemon imports clean
[ ] G3 unit suite (>=39 tests) passes
[ ] G4 in-tree integration suite (4 tests) passes
[ ] G5 existing demo modes unbroken (decision + litellm_real)
[ ] G6 `make demo-up DEMO_MODE=dify_plugin_real` exits 0 + success lines printed
[ ] G7 `make demo-down` clean
[ ] G8 docs/site build succeeds + new dify page renders + 3 sections present
[ ] G9 README adapter row landed
[ ] G10 `dify plugin package` produces .difypkg artefact, zero warnings
[ ] G11 publish workflow lint clean
[ ] G12 no proto / Rust / SQL drift outside allow-list
[ ] G13 no spendguard-sdk integrations folder mutated
[ ] INV-1 .. INV-9 all green
[ ] All 8 slices merged in order S1 → S8 with R1-R5 review loop per build-plan §1.1
[ ] Memory write-back entry `project_coverage_D10_shipped.md` drafted per build-plan §8
```

## 4. Definition of done (per build-plan §7)

- All 8 slices merged into main.
- Acceptance gates G1..G13 + invariants INV-1..INV-9 green.
- README adapter row landed.
- `docs/site/docs/integrations/dify.md` live.
- `Makefile` `DEMO_MODE=dify_plugin_real` branch live.
- `dify-plugin-publish.yml` workflow exists (marketplace push is a follow-up, not a ship gate).
- Memory entry written per build-plan §8.

## 5. Out-of-scope explicit declarations

D10 does NOT close any of:

- Workflow-step (tool-call, RAG retrieval) cost gating — future Dify plugin slot types.
- Bedrock IAM / GCP service-account auth inside Dify — passthrough only.
- Per-app fine-grained budget keys beyond `workspace_id` / `app_id` reads.
- Gemini upstream + Bedrock upstream implementation — v1 ships stubs that raise `InvokeError`; v1.1 fills these in (tracked as GH issues at merge time).
- Token-by-token cap mid-stream — end-of-stream only.
- Dify Cloud marketplace push automation beyond the lint-clean publish workflow.

These are documented in `docs/site/docs/integrations/dify.md` "Limitations" section so operator expectation matches the ship surface.

## 6. Risk register

| Risk | Mitigation |
|------|-----------|
| Dify plugin SDK 0.2 → 0.3 breaking change between spec-write and ship | Pin `dify-plugin>=0.2.0,<0.3.0` in `requirements.txt`. CI matrix includes the latest 0.2.x patch; 0.3.x release will require a follow-up issue, not a v1 blocker. |
| Self-hosted Dify image (`langgenius/dify-api:1.0`) compose drift | Pin image digest in the overlay compose. CI fails on digest mismatch. |
| Dify Cloud marketplace policy change rejects 3P plugins | The plugin ships as a sideload `.difypkg` regardless; marketplace push is a nice-to-have. INV is the sideload path works. |
| Operator misconfigures `upstream_provider` (e.g. picks `gemini` in v1) | `_invoke` raises `InvokeError` with explicit "not supported in this plugin version" message. Test O08 pins. |
| Demo stack is heavy (~2 GB Dify images) | Compose layer cache in CI; `make demo-clean` documented; `DEMO_MODE=dify_plugin_real` opt-in only, not part of default `make demo-up`. |
