# D09 — Kong AI Gateway Plugin — Acceptance Gates

**Companion to:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md)

A SpendGuard deliverable is "shipped" only when **every** gate listed below runs green on a clean checkout of `main`. Reviewer (`superpowers:code-reviewer`) must be able to re-run every gate without privileged access beyond what's in the repo.

---

## §1. Headline acceptance gate (the demo proof)

```bash
export OPENAI_API_KEY=sk-...                     # real key, real upstream
make demo-up DEMO_MODE=kong_gateway_real
```

This command must:

1. Boot Postgres + SpendGuard sidecar (with HTTP companion enabled) + Kong DataPlane (with the SpendGuard Go plugin loaded via `plugin-server`).
2. Configure Kong declaratively to proxy `POST /v1/chat/completions` upstream to `https://api.openai.com/v1/chat/completions` via the `ai-proxy` plugin, with the `spendguard` plugin attached as a route-level guardrail.
3. Run a 3-call client script:
   - **ALLOW**: budget headroom = $5.00, request "Say hi", expect HTTP 200 + valid OpenAI usage block + sidecar audit row with `PRE_LLM_CALL.RESERVE` (ALLOW) + `LLM_CALL_POST.SUCCESS`.
   - **DENY**: budget headroom = $0.00, same request, expect HTTP **429** with body containing `SPENDGUARD_DENY` + sidecar audit row with `PRE_LLM_CALL.RESERVE` (DENY) + **no** `LLM_CALL_POST.*` follow-up.
   - **COMMIT-IDEMPOTENCY**: replay the ALLOW request with the same `Idempotency-Key` header, expect HTTP 200 from cache and exactly one `LLM_CALL_POST.SUCCESS` row total.
4. Execute `verify_step_kong_gateway_real.sql` against the demo Postgres; all 5 assertions must return `t`.
5. Print `[demo] D09 kong_gateway_real PASS` on success; exit non-zero on any assertion failure.

Reviewer re-runs:

```bash
make demo-up DEMO_MODE=kong_gateway_real && make demo-down DEMO_MODE=kong_gateway_real
```

## §2. Sidecar HTTP companion gates (SLICE 1)

| ID | Gate | Command |
|----|------|---------|
| D09-A-01 | HTTP companion unit tests pass | `cargo test -p spendguard-sidecar http_companion` |
| D09-A-02 | mTLS required (non-mTLS rejected) | `cargo test -p spendguard-sidecar http_companion::mtls_required` |
| D09-A-03 | SVID SAN URI validates tenant | `cargo test -p spendguard-sidecar http_companion::svid_san_validates_tenant` |
| D09-A-04 | Loopback-only by default | `cargo test -p spendguard-sidecar http_companion::loopback_only_by_default` |
| D09-A-05 | 4MiB body cap enforced | `cargo test -p spendguard-sidecar http_companion::tokenize_handler_4mib_cap` |
| D09-A-06 | Integration: 100 concurrent /v1/decision, no chain break | `cargo test -p spendguard-sidecar --test http_companion_integration` |

## §3. Go plugin gates (SLICE 2-4)

| ID | Gate | Command |
|----|------|---------|
| D09-B-01 | Go plugin builds to ELF .so | `cd plugins/kong/spendguard-go && go build ./... && file ../../../target/kong/spendguard` (must contain `ELF 64-bit LSB executable`) |
| D09-B-02 | Go unit tests with `-race` pass | `cd plugins/kong/spendguard-go && go test -race -cover ./...` |
| D09-B-03 | Go line coverage ≥ 85% (excluding `main.go`) | inspect `go test -coverprofile` output |
| D09-B-04 | kong-pongo `[D09-ACCESS-*]` scenarios pass | `cd plugins/kong/spendguard-go && pongo run` |
| D09-B-05 | kong-pongo `[D09-COMMIT-*]` scenarios pass | (same as B-04) |
| D09-B-06 | fail-closed default verified (no `fail_open` flag → DEGRADE = 503) | `pongo run -- --grep fail_closed_default` |

## §4. Lua fallback gates (SLICE 5)

| ID | Gate | Command |
|----|------|---------|
| D09-C-01 | Lua plugin schema validates | `cd plugins/kong/spendguard-lua && pongo run -- --grep schema` |
| D09-C-02 | kong-pongo `[D09-LUA-*]` scenarios pass | `cd plugins/kong/spendguard-lua && pongo run` |
| D09-C-03 | Docs page marks Lua "experimental" | grep `experimental` in `docs/site/docs/integrations/kong-ai-gateway.md` |

## §5. Helm chart gates (SLICE 6)

| ID | Gate | Command |
|----|------|---------|
| D09-D-01 | `helm template` succeeds with `kongPlugin.enabled=true` | `helm template ./charts/spendguard --set kongPlugin.enabled=true` |
| D09-D-02 | `helm template` fails closed when `kongPlugin.svidIssuer` unset | `! helm template ./charts/spendguard --set kongPlugin.enabled=true 2>/dev/null` |
| D09-D-03 | NetworkPolicy ingress selector matches `app.kubernetes.io/name=kong` | `helm template ./charts/spendguard --set kongPlugin.enabled=true,kongPlugin.svidIssuer=test \| yq 'select(.kind=="NetworkPolicy")' \| grep "kong"` |
| D09-D-04 | ServiceMonitor rendered when enabled | `helm template ... \| yq 'select(.kind=="ServiceMonitor")'` non-empty |
| D09-D-05 | Kind smoke test passes (kong-companion pod Ready, /v1/tokenize reachable) | `bash charts/spendguard/tests/kong_plugin_kind.sh` |
| D09-D-06 | All Helm chart unit assertions pass | `bash charts/spendguard/tests/kong_plugin_test.sh` |

## §6. Demo gates (SLICE 7)

| ID | Gate | Command |
|----|------|---------|
| D09-E-01 | `DEMO_MODE=kong_gateway_real` target exists in `deploy/demo/Makefile` | `grep "DEMO_MODE.*kong_gateway_real" deploy/demo/Makefile` |
| D09-E-02 | `compose.kong.yaml` parses | `docker compose -f deploy/demo/compose.kong.yaml config` |
| D09-E-03 | Demo boots and ALLOW returns OpenAI 200 | `make demo-up DEMO_MODE=kong_gateway_real` exits 0 |
| D09-E-04 | DENY returns 429 with `SPENDGUARD_DENY` body | observed in demo client log (`grep SPENDGUARD_DENY deploy/demo/.demo-out/client.log`) |
| D09-E-05 | `verify_step_kong_gateway_real.sql` all 5 assertions PASS | demo make target re-runs the SQL on demand |
| D09-E-06 | Audit chain hash continuity intact | `spendguard_verify_chain('kong_gateway_real')` returns `t` |
| D09-E-07 | Demo tears down cleanly | `make demo-down DEMO_MODE=kong_gateway_real` exits 0 |

## §7. Documentation gates (SLICE 7)

| ID | Gate | Command |
|----|------|---------|
| D09-F-01 | `docs/site/docs/integrations/kong-ai-gateway.md` exists and is non-empty | `test -s docs/site/docs/integrations/kong-ai-gateway.md` |
| D09-F-02 | Page covers both Go and Lua install paths | `grep -q "Go plugin" docs/site/docs/integrations/kong-ai-gateway.md && grep -q "Lua" docs/site/docs/integrations/kong-ai-gateway.md` |
| D09-F-03 | README.md adapter integrations table has D09 row | `grep -q "Kong AI Gateway" README.md` |
| D09-F-04 | Page renders in Starlight build | `cd docs/site && npm run build` exits 0 |
| D09-F-05 | 10-line `curl` recipe present | `grep -A 10 "curl recipe" docs/site/docs/integrations/kong-ai-gateway.md` |

## §8. Cross-cutting / hygiene

| ID | Gate | Command |
|----|------|---------|
| D09-G-01 | Workspace cargo build clean | `cargo build --workspace` |
| D09-G-02 | Workspace cargo test clean | `cargo test --workspace` |
| D09-G-03 | No clippy warnings in `services/sidecar/src/server/http_companion.rs` | `cargo clippy -p spendguard-sidecar -- -D warnings` |
| D09-G-04 | `go vet` clean | `cd plugins/kong/spendguard-go && go vet ./...` |
| D09-G-05 | No fabricated audit columns / no parallel audit lane | `grep -r "audit_outbox" plugins/kong/ services/sidecar/src/server/http_companion.rs` — must show only routed-through-decision-transaction writes, no direct INSERT |
| D09-G-06 | No `fail_open=true` in default Helm values | `grep "failOpen.*false" charts/spendguard/values.yaml` |
| D09-G-07 | Adapter v1alpha1 proto unchanged | `git diff main -- 'services/sidecar/proto/**'` empty |

## §9. Release gates (deliverable definition-of-done)

1. All slices merged to `main`.
2. All gates in §2-§8 run green.
3. Memory write-back at `~/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/project_coverage_D09_shipped.md` recording: merge commits, round count, arbitration y/n.
4. `make demo-up DEMO_MODE=kong_gateway_real` PASS recorded in the slice memory as the headline acceptance proof (per `feedback_demo_quality_gate`).

## §10. Non-acceptance (explicit exclusions)

The following are **not** acceptance gates for D09 and must not block merge:

- Streaming SSE budget enforcement mid-stream (anti-scope §5 of design.md).
- Kong Konnect SaaS integration.
- Co-install validation with `ai-rate-limiting-advanced`.
- Bedrock SigV4 mutation (Kong's `ai-proxy` handles upstream auth).
- Lua plugin coverage ≥ 85% (Lua is experimental tier).
- Customer plugin contract Strategy C integration.
