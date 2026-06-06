# D31 — Coze Studio Model Provider — Tests

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`acceptance.md`](acceptance.md).

## §1. Test taxonomy

D31 is a recipe + docs + demo deliverable. There is no new Rust or Python plugin code to unit-test. The test surface concentrates on (a) recipe validity, (b) end-to-end smoke against a real Coze stack, (c) demo regression, (d) docs build.

| Layer | Framework | Where | Slice |
|-------|-----------|-------|-------|
| Recipe lint (YAML / Markdown) | `yamllint`, `python -c "yaml.safe_load(...)"`, `markdownlint` | shell one-liners | Slice 1 |
| Header cheatsheet completeness | `grep` assertions | shell | Slice 1 |
| Coze stack smoke (docker-compose) | `bash examples/coze-studio/smoke.sh` | shell + curl + psql | Slice 2 |
| Demo regression (existing modes still pass) | `make demo-up DEMO_MODE=decision` + `litellm_real` | Makefile | Slice 3 |
| Demo-mode E2E | `make demo-up DEMO_MODE=coze_studio_real` | Makefile | Slice 3 |
| Audit chain SQL gate | `verify_step_coze_studio_real.sql` | psql | Slice 3 |
| Docs build (Starlight) | `cd docs/site && npm run build` | npm | Slice 4 |
| README adapter row | `grep` assertion | shell | Slice 4 |

There are **no unit tests** under `tests/` for D31 — all verification is via lint, smoke, demo, and docs build.

## §2. Per-slice test plan

### Slice 1 — Coze workspace config snippet

**Recipe lint gates** (run from repo root):

```bash
# T01: workspace YAML is parseable
python -c "import yaml,sys; yaml.safe_load(open('examples/coze-studio/coze-workspace-config.yaml'))"

# T02: YAML uses safe types only (no Python-specific tags)
yamllint examples/coze-studio/coze-workspace-config.yaml

# T03: snippet declares the three required X-SpendGuard-* headers
grep -q "X-SpendGuard-Tenant-Id" examples/coze-studio/coze-workspace-config.yaml
grep -q "X-SpendGuard-Budget-Id" examples/coze-studio/coze-workspace-config.yaml
grep -q "X-SpendGuard-Window-Instance-Id" examples/coze-studio/coze-workspace-config.yaml

# T04: snippet does NOT hardcode a real api_key (env-var ref only)
! grep -E 'api_key:\s*"?sk-' examples/coze-studio/coze-workspace-config.yaml

# T05: snippet base_url targets sidecar HTTP companion path
grep -q "/v1/openai" examples/coze-studio/coze-workspace-config.yaml

# T06: README walks through all three required headers
grep -c "X-SpendGuard-" examples/coze-studio/README.md  # >= 3

# T07: README has a "Troubleshooting" section
grep -q "Troubleshooting" examples/coze-studio/README.md

# T08: headers-cheatsheet documents the three headers with format rules
grep -c "X-SpendGuard-" examples/coze-studio/headers-cheatsheet.md  # >= 3
```

**Anti-regression**:

- No `OPENAI_API_KEY` literal anywhere in `examples/coze-studio/`.
- No `fail_open` flag in the snippet (Coze v1 is fail-closed only — §3.4 of design).

### Slice 2 — HTTP companion smoke

**Smoke harness** (`examples/coze-studio/smoke.sh`):

```bash
#!/usr/bin/env bash
# T-SMOKE-01..06
set -euo pipefail

# Boot
docker compose -f examples/coze-studio/docker-compose.coze.yaml up -d --wait

# T-SMOKE-01: sidecar HTTP companion answers /v1/openai/chat/completions
curl --cacert ${SIDECAR_CA} --cert ${COZE_CERT} --key ${COZE_KEY} \
  -X POST https://localhost:8443/v1/openai/chat/completions \
  -H "Authorization: Bearer ${OPENAI_API_KEY}" \
  -H "X-SpendGuard-Tenant-Id: coze-smoke" \
  -H "X-SpendGuard-Budget-Id: bud_smoke" \
  -H "X-SpendGuard-Window-Instance-Id: win_smoke" \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}' \
  -o /tmp/coze-smoke.json -w "%{http_code}\n" | grep -q "^200$"

# T-SMOKE-02: response shape is OpenAI-compatible
jq -e '.choices[0].message.content' /tmp/coze-smoke.json

# T-SMOKE-03: audit chain has reserve + commit with matching reservation_id
psql -h localhost -U spendguard -d spendguard_ledger -At -c "
  SELECT COUNT(DISTINCT reservation_id) FROM audit_outbox
   WHERE decision_context->>'integration' = 'coze_studio'
     AND created_at > now() - interval '1 minute';" | grep -q "^[1-9]"

# T-SMOKE-04: missing tenant header → 400 with structured error
RESP=$(curl --cacert ${SIDECAR_CA} --cert ${COZE_CERT} --key ${COZE_KEY} \
  -X POST https://localhost:8443/v1/openai/chat/completions \
  -H "Authorization: Bearer ${OPENAI_API_KEY}" \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}' \
  -o /tmp/coze-smoke-err.json -w "%{http_code}\n")
[ "$RESP" = "400" ]
jq -e '.error.code | test("MISSING_TENANT")' /tmp/coze-smoke-err.json

# T-SMOKE-05: Coze Studio container is healthy (we did not bring it down)
docker compose -f examples/coze-studio/docker-compose.coze.yaml ps --format json | \
  jq -e '.[] | select(.Service=="coze-studio") | .Health == "healthy"'

# T-SMOKE-06: tear down cleanly
docker compose -f examples/coze-studio/docker-compose.coze.yaml down -v
```

**Notes**:

- Smoke uses a **real** OpenAI key (`OPENAI_API_KEY` must be set). Per `feedback_demo_quality_gate`, fake upstream is insufficient — wire assumptions only surface against real upstreams.
- Smoke does **not** drive Coze Studio's UI; it replays Coze's "test connection" probe shape directly against the companion. UI-driven flow is covered in Slice 3 demo.

### Slice 3 — Demo mode

**Demo regression gate** (run before D31 demo to prove non-regression):

```bash
make demo-down && make demo-up DEMO_MODE=decision && make demo-down
make demo-up DEMO_MODE=litellm_real && make demo-down
```

Both must exit 0. D31 is purely additive; existing demo modes' compose files were not edited.

**Demo-mode E2E** (`make demo-up DEMO_MODE=coze_studio_real`):

| Step | Body | Asserts |
|------|------|---------|
| 1 ALLOW | small prompt fits budget; Coze chat-flow API POST | Coze HTTP 200 + sidecar reserve + commit + real OpenAI hit visible in `provider_event_id` |
| 2 DENY | pre-exhausted budget OR oversized projection | Coze surfaces 502 from companion; DENY audited; **zero** upstream stub hits |
| 3 STREAM | Coze chat-flow with `stream=true` | Coze receives SSE chunks; end-of-stream commit row in SpendGuard with `decision_context.stream=true` |

Driver writes `[demo] coze_studio_real ALL 3 steps PASS (ALLOW + DENY + STREAM)` on success; exits 9 on gate failure.

**SQL gate** (`verify_step_coze_studio_real.sql`): all 7 assertions in `implementation.md` §2 Slice 3 SQL block must return success / no exception.

### Slice 4 — Docs page

**Docs build gate**:

```bash
cd docs/site && npm run build  # T-DOC-01: build succeeds
test -s docs/site/dist/docs/integrations/coze-studio/index.html  # T-DOC-02: page rendered
```

**Docs content gates** (grep against `docs/site/docs/integrations/coze-studio.md`):

```bash
grep -q "Install (self-hosted Coze)" docs/site/docs/integrations/coze-studio.md  # T-DOC-03
grep -q "Decision matrix" docs/site/docs/integrations/coze-studio.md             # T-DOC-04
grep -q "Limitations" docs/site/docs/integrations/coze-studio.md                 # T-DOC-05
grep -q "Troubleshooting" docs/site/docs/integrations/coze-studio.md             # T-DOC-06
grep -q "X-SpendGuard-Tenant-Id" docs/site/docs/integrations/coze-studio.md      # T-DOC-07
grep -q "Coze Cloud" docs/site/docs/integrations/coze-studio.md                  # T-DOC-08 (must mention as unsupported)
grep -q "Anthropic" docs/site/docs/integrations/coze-studio.md                   # T-DOC-09 (under Limitations / v1.1)
```

**README gate**:

```bash
grep -q "Coze Studio" README.md          # T-DOC-10
grep -q "examples/coze-studio" README.md # T-DOC-11
```

## §3. Negative test surface (must-not-regress)

| What | Why | Where |
|------|-----|-------|
| Upstream hit on DENY | Worst correctness bug — wires DENY into a fake-allow | Demo driver step 2 + verify SQL `stub_hits` assertion |
| Reserve fires AFTER upstream | Cancels D31 thesis | Slice 2 smoke T-SMOKE-03 (reservation row created before any upstream OpenAI billable) |
| Snippet contains real `sk-*` API key | Secret leak via the repo | Slice 1 T04 |
| Coze stack's Postgres collides with `spendguard_ledger` schema | Demo cross-contamination | Slice 3: Coze uses separate `coze_db` database name |
| Tenant header missing → silent allow | Wires no-tenant calls into a default tenant bucket | Slice 2 T-SMOKE-04 (400 required) |
| Audit chain hash break across the demo | Audit integrity regression | Slice 3 SQL `spendguard_verify_chain('coze_studio_real')` |
| `fail_open=true` smuggled into the snippet | Violates §3.4 fail-closed default | Slice 1 anti-regression grep |
| Docs page silent on Coze Cloud unsupported | Operator wastes time | Slice 4 T-DOC-08 |

## §4. Performance budgets (informational)

| Op | Target | Notes |
|----|--------|-------|
| Sidecar HTTP companion overhead vs raw OpenAI call | < 30 ms p95 added latency | D09 SLICE 1 already enforces; D31 inherits |
| Smoke harness end-to-end (boot → 1 call → teardown) | < 90s on a workstation | informational; not a CI gate |
| `make demo-up DEMO_MODE=coze_studio_real` boot | < 180s including Coze image pull (cached) | informational |

Not CI-gated; verified manually post-merge.

## §5. CI integration

D31 hooks into the existing CI matrix at three points:

1. **`plugins-recipes-lint`** (new lightweight job): runs the Slice 1 lint gates T01-T08 on every PR touching `examples/coze-studio/**`.
2. **`e2e-demo-coze`** (new matrix cell): runs `make demo-up DEMO_MODE=coze_studio_real` on tagged builds (`coverage-d31-*` tag prefix). Not on every PR — Coze image is large (~1.5 GB), keeps PR CI fast. Triggered on D31 slice branches via path filter.
3. **`docs-build`** (existing job): naturally picks up the new docs page on every PR.

The `examples/coze-studio/smoke.sh` is **not** in PR CI (requires a real `OPENAI_API_KEY`); it's runnable locally by reviewers and runs in the tagged `e2e-demo-coze` job.

## §6. Test ownership

| Slice | Owner runs gates on R1-R5 | Reviewer (`superpowers:code-reviewer`) verifies |
|-------|---------------------------|-------------------------------------------------|
| 1 | Implementer | T01-T08 + anti-regression greps |
| 2 | Implementer + manual reviewer with `OPENAI_API_KEY` | T-SMOKE-01..06 |
| 3 | Implementer | Demo regression + demo-mode E2E + SQL gate |
| 4 | Implementer | T-DOC-01..11 + Starlight build |

Per build-plan §1.2, reviewer re-runs every gate without privileged access — the only "privileged" gate is the OpenAI key for Slice 2 smoke + Slice 3 demo, called out explicitly in the acceptance doc.
