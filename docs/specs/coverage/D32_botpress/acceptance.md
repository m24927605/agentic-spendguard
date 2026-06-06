# D32 — Acceptance

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`review-standards.md`](review-standards.md).

D32 ships when **every gate** in §1 is green AND the §2 invariants are unbreakable AND the §3 ship checklist is fully checked. Per build-plan §3 "100% feasible" rule: every gate below is runnable in the repo's current state, no third-party action required (Botpress Cloud marketplace push is gated behind a secret-availability check, not a hard ship gate).

## 1. Hard gates

### G1 — Integration scaffold validates

```bash
cd integrations/botpress && pnpm install && pnpm lint
```

Expected: exits 0. `botpress.integration.yaml` valid YAML, `package.json` declares peer-deps for `@spendguard/sdk@^0.1.0` and `@botpress/sdk@^0.7.0`.

### G2 — Integration builds

```bash
cd integrations/botpress && pnpm build
```

Expected: produces `dist/index.js` + `dist/index.d.ts`. No type errors. Bundle size < 100 KB (peer-deps externalised).

### G3 — Unit suite

```bash
cd integrations/botpress && pnpm test
```

Expected: 37 unit tests pass (count from `tests.md` §2.1-§2.5; final count may rise during implementation but never fall below 37).

### G4 — Integration suite (real Botpress v12 in testcontainers)

```bash
cd integrations/botpress && pnpm test:integration
```

Expected: 4 tests pass. Botpress v12 container boots, mock sidecar + mock upstream both observe correct ordering. Sufficient if at least 1 ALLOW + 1 DENY + 1 validateConfiguration probe shows in the mock sidecar event log.

### G5 — Existing demos still pass (regression)

```bash
make demo-down
make demo-up DEMO_MODE=decision
make demo-down
make demo-up DEMO_MODE=litellm_real
make demo-down
make demo-up DEMO_MODE=dify_plugin_real
```

Expected: all three existing demos still exit 0. D32 is purely additive — the existing demo modes' compose files were not edited (Slice 5 lives in an overlay file).

### G6 — Botpress demo boots and passes

```bash
make demo-down
make demo-up DEMO_MODE=botpress_real
```

Expected:

- All compose services (Botpress v12 + SpendGuard sidecar + Postgres + ledger + canonical ingest + outbox forwarder + the upstream counting stub) reach healthy.
- Demo driver exits 0.
- stdout contains `[demo] botpress_real ALL 3 steps PASS (ALLOW + DENY + STREAM)`.
- stdout contains `D32_BOTPRESS OK: decisions=N` for `N >= 2`.
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

Expected: build succeeds. `docs/site/dist/docs/integrations/botpress/index.html` exists. Page contains:

- "Install (self-hosted Botpress v12)" section with `botpress integrations push` command.
- "Install (Botpress Cloud)" section with marketplace submission path (clearly noted as "pending approval").
- Decision matrix comparing the Botpress integration path vs the egress-proxy path.
- "Limitations" section explicitly listing: no workflow-node gating beyond AI hook, no token-by-token cap, no Botpress channel plugin coverage.

### G9 — README index entry present

```bash
grep -F "Botpress" README.md
```

Expected: exactly one row in the adapter integrations table that includes `npm i @spendguard/botpress-integration && botpress integrations push`.

### G10 — Integration package publish dry-run

```bash
cd integrations/botpress && pnpm pack --dry-run
```

Expected: produces a tarball manifest; package files include `dist/`, `botpress.integration.yaml`, `README.md`, `package.json`. Tarball estimated size < 200 KB.

### G11 — Publish workflow lints clean

```bash
gh workflow view botpress-integration-publish.yml --yaml | yq '.jobs' >/dev/null
actionlint .github/workflows/botpress-integration-publish.yml
```

Expected: no lint errors. Workflow runs only on `botpress-integration-v*` tags; not on PR CI. The workflow's marketplace-push step is conditional on `secrets.BOTPRESS_MARKETPLACE_TOKEN` so absence on PR CI is not a failure.

### G12 — No proto / no DB-schema / no Rust changes (purely additive)

```bash
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '\.(proto|rs)$' && echo FAIL || echo OK
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '\.sql$' | grep -v -E '^deploy/demo/(verify_step_botpress\.sql|botpress/.*\.sql)$' && echo FAIL || echo OK
```

Expected: both lines print `OK`. The only new SQL files are the demo verify gate and any Botpress seed scripts in the demo allow-list.

### G13 — No `@spendguard/sdk` mutations

```bash
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '^sdk/typescript/' && echo FAIL || echo OK
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '^sdk/python/' && echo FAIL || echo OK
```

Expected: both print `OK`. D32 lives entirely under `integrations/botpress/` + `deploy/demo/` + `docs/site/` + `.github/workflows/` + `README.md`. The TS and Python SDK trees are untouched.

### G14 — No D09 HTTP companion mutation

```bash
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '^services/sidecar/.*http_companion' && echo FAIL || echo OK
```

Expected: `OK`. D32 reuses, does not extend, D09 SLICE 1's HTTP companion contract.

## 2. Invariants (must never regress)

| ID | Invariant | How verified |
|----|-----------|--------------|
| INV-1 | **DENY never hits the upstream provider.** Counting stub MUST register zero hits across all DENY decisions in the demo. | `verify_step_botpress.sql` `stub_hits` assertion + B03 + I02 + demo step 2 |
| INV-2 | **Pre-call reservation precedes upstream HTTP.** Sidecar `/v1/decision` POST fires before Botpress's outbound to the mock upstream. | B07 + I01 strict-ordering event |
| INV-3 | **Fail-closed default.** Sidecar DEGRADE → Botpress `RuntimeError` (`code: BUDGET_DEGRADED`). Only `SPENDGUARD_BOTPRESS_FAIL_OPEN=1` permits otherwise. | R06 + R07 + B04 + manual DEGRADE injection |
| INV-4 | **`validateConfiguration` is a SpendGuard-side probe, not only an upstream probe.** Catches sidecar misconfig at integration register time. | L01 + I04 |
| INV-5 | **End-of-hook commit uses real usage when present.** `event.payload.usage` is what lands in the commit row; estimator-fallback only fires when usage is missing and logs a WARN. | A01 + A02 + demo step 3 |
| INV-6 | **No mutation of operator credentials in logs.** No log line contains `sidecarUrl`, `tlsKeyPath`, or any substring of an mTLS key. | Linter rule + code-reviewer checklist + manual log scrape in demo |
| INV-7 | **Idempotency across retries.** Two reservations for the same `(botId, conversationId, runId, retry)` tuple dedupe sidecar-side; no double-charge under Botpress's retry behaviour. | R11 + demo retry probe |
| INV-8 | **Integration does NOT mutate Botpress core or other integrations.** Bundle is mounted read-only; its dependencies don't get into Botpress core's Node env. | G6 (Botpress image digest unchanged) + demo logs absence of import side-effects |
| INV-9 | **No TS/Python SDK or D09 companion drift.** D32 is purely additive at the integration tree. | G12 + G13 + G14 |
| INV-10 | **Hook re-entrancy safety.** Two messages on the same conversation get distinct handles; no shared mutable state. | A08 + B05 |

## 3. Ship checklist

```
[ ] G1 integration scaffold validates
[ ] G2 integration builds
[ ] G3 unit suite (>=37 tests) passes
[ ] G4 integration suite (4 tests, real Botpress v12) passes
[ ] G5 existing demo modes unbroken (decision + litellm_real + dify_plugin_real)
[ ] G6 `make demo-up DEMO_MODE=botpress_real` exits 0 + success lines printed
[ ] G7 `make demo-down` clean
[ ] G8 docs/site build succeeds + new botpress page renders + 3 sections present
[ ] G9 README adapter row landed
[ ] G10 `pnpm pack --dry-run` produces a clean tarball manifest
[ ] G11 publish workflow lint clean
[ ] G12 no proto / Rust / SQL drift outside allow-list
[ ] G13 no @spendguard/sdk (TS or Python) mutations
[ ] G14 no D09 HTTP companion mutation
[ ] INV-1 .. INV-10 all green
[ ] All 5 slices merged in order S1 → S5 with R1-R5 review loop per build-plan §1.1
[ ] Memory write-back entry `project_coverage_D32_shipped.md` drafted per build-plan §8
```

## 4. Definition of done (per build-plan §7)

- All 5 slices merged into main.
- Acceptance gates G1..G14 + invariants INV-1..INV-10 green.
- README adapter row landed.
- `docs/site/docs/integrations/botpress.md` live.
- `Makefile` `DEMO_MODE=botpress_real` branch live.
- `botpress-integration-publish.yml` workflow exists (marketplace push is a follow-up, not a ship gate).
- Memory entry written per build-plan §8.

## 5. Out-of-scope explicit declarations

D32 does NOT close any of:

- Workflow-node-level cost gating (RAG nodes, tool-call nodes, knowledge-base retrieval) — future Botpress hook surfaces.
- Botpress channel plugin coverage (WhatsApp / Slack / Web Chat) — orthogonal axis.
- Token-by-token mid-stream cap — end-of-hook only.
- Botpress Cloud marketplace push automation beyond the lint-clean publish workflow.
- Per-conversation Strategy C customer plugin contract — v1.1.
- `@botpress/sdk` 0.8.x compatibility — pinned `^0.7.0` only.
- Botpress v11 or earlier — different hook surface.

These are documented in `docs/site/docs/integrations/botpress.md` "Limitations" section so operator expectation matches the ship surface.

## 6. Risk register

| Risk | Mitigation |
|------|-----------|
| `@botpress/sdk` 0.7 → 0.8 breaking change between spec-write and ship | Pin `@botpress/sdk@^0.7.0,<0.8.0` in `peerDependencies`. Integration CI matrix includes the latest 0.7.x; 0.8.x release will require a follow-up issue, not a v1 blocker. |
| Self-hosted Botpress image (`botpress/server:v12.30.x`) compose drift | Pin image digest in the overlay compose. CI fails on digest mismatch. |
| Botpress Cloud marketplace policy change rejects 3P integrations | The integration ships as a `botpress integrations push` sideload regardless; marketplace push is a nice-to-have. INV is the sideload path works. |
| Operator misconfigures `upstreamProvider` (e.g. picks unsupported) | Zod enum rejection at register time + clear error. L04 pins. |
| Demo stack is heavy (~800 MB Botpress image) | Compose layer cache in CI; `make demo-clean` documented; `DEMO_MODE=botpress_real` opt-in only, not part of default `make demo-up`. |
| D09 SLICE 1 HTTP companion not yet landed when D32 starts | D32 SLICE 1 has a fallback clause: extracts the companion endpoint inline behind a feature flag, removed when D09 lands. Documented in `implementation.md` §6. |
| Botpress runtime's hook re-entrancy under high conversation concurrency | INV-10 + B05 + A08 lock the per-call handle isolation. |
