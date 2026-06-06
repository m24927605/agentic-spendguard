# COV_D11_S6 — D11 LiteLLM proxy plugin: DEMO_MODE=litellm_guardrail

> **Deliverable**: D11 LiteLLM `async_pre_call_hook` proxy guardrail plugin
> **Slice**: 6 of 7 (M)
> **Spec set**: [`docs/specs/coverage/D11_litellm_proxy_plugin/`](../specs/coverage/D11_litellm_proxy_plugin/)

## Scope

Land `make demo-up DEMO_MODE=litellm_guardrail` per design §1 line 16 + §2 line 26: boots `postgres + sidecar + litellm-proxy + counting-stub`, issues 3 calls (ALLOW + DENY + STREAM), asserts pre-call sidecar reservation BEFORE the upstream stub is hit on each ALLOW, verifies the stub counter does NOT increment on DENY.

This is the "demo as quality gate" — Codex ✅ insufficient; the SLICE 5 proxy_config.yaml entry must actually work in a running compose stack against a counting stub.

Concretely:
- `deploy/demo/litellm_guardrail/proxy_config.yaml` — NEW:
  - Adapted copy of `examples/litellm-proxy/proxy_config.yaml` with `model_list:` pointing at the counting-stub
  - guardrails: stanza using `spendguard.integrations.litellm_guardrail.spendguard_guardrail_factory`
  - mode: pre_call, default_on: true
- `deploy/demo/litellm_guardrail/docker-compose.yaml` — NEW (or extend existing demo compose):
  - postgres + sidecar + litellm-proxy + counting-stub services
  - SPENDGUARD_TENANT_ID / SPENDGUARD_SIDECAR_ADDRESS / SPENDGUARD_BUDGET_ID / WINDOW / UNIT injected via env on litellm-proxy service
  - SPENDGUARD_RESOLVER_MODULE pointed at a deploy-side resolver (single-tenant default)
- `deploy/demo/litellm_guardrail/Makefile.partial` — NEW:
  - `make demo-up DEMO_MODE=litellm_guardrail` recipe
  - Brings up the compose stack
  - Calls the driver script
  - On success: tears down compose; on failure: leaves logs
- `deploy/demo/litellm_guardrail/driver.py` (or .sh) — NEW:
  - Issues 3 LiteLLM proxy calls:
    1. ALLOW: small prompt within budget → sidecar reserves → stub serves → audit OK
    2. DENY: prompt over budget cap → sidecar denies → stub NOT hit → 403 returned
    3. STREAM: streaming call within budget → sidecar reserves once + commits at end
  - Asserts via SQL on sidecar postgres:
    - reservations table has 2 entries (ALLOW + STREAM); NOT 3
    - stub_call_count table (or stub log) shows 2 calls; NOT 3
    - DENY emitted RUN_ABORTED audit row
- `deploy/demo/litellm_guardrail/verify.sql` — NEW SQL gates:
  - Strict-order INV-2 check (review-standards §6.4): sidecar reservation timestamp < stub call timestamp on each ALLOW
- `Makefile` — top-level: route `DEMO_MODE=litellm_guardrail` to the new demo bundle
- (optional) `deploy/demo/litellm_guardrail/README.md` — operator-facing 1-page guide

## Files touched

| File | Why |
|------|-----|
| `deploy/demo/litellm_guardrail/proxy_config.yaml` | NEW — demo yaml |
| `deploy/demo/litellm_guardrail/docker-compose.yaml` | NEW — compose stack |
| `deploy/demo/litellm_guardrail/Makefile.partial` | NEW — demo recipe |
| `deploy/demo/litellm_guardrail/driver.py` | NEW — 3-call driver |
| `deploy/demo/litellm_guardrail/verify.sql` | NEW — strict-order INV-2 |
| `Makefile` | DEMO_MODE routing |
| (optional) `deploy/demo/litellm_guardrail/README.md` | Operator guide |

## Test/verification plan

1. `make demo-up DEMO_MODE=litellm_guardrail` succeeds end-to-end (compose up + 3 calls + assertions)
2. Sidecar postgres shows exactly 2 reservations (ALLOW + STREAM), not 3
3. Counting-stub call_count shows exactly 2, not 3
4. DENY case: sidecar emits RUN_ABORTED audit; LiteLLM proxy returns 403 to client; stub_call_count unchanged
5. INV-2 strict-order: for each ALLOW, sidecar reserve timestamp < stub call timestamp (asserts the pre-call hook fired before upstream)

## Anti-scope

- No docs page — SLICE 7
- No new factory paths — SLICE 5 owns
- No new hook bodies — SLICE 1-3 wired

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D11_litellm_proxy_plugin/design.md) §1 line 16, §2 line 26, §6 slice 6 row, §6.4 INV-2 strict-order
- SLICE 5: [`COV_D11_S5_proxy_config_entry.md`](COV_D11_S5_proxy_config_entry.md)
- Demo-as-quality-gate per [[feedback_demo_quality_gate]]: Codex ✅ insufficient
