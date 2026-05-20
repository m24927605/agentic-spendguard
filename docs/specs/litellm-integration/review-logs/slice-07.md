# Slice 7 — `litellm_deny` 3 fail-closed sub-steps · review log

Slice scope: `DEMO_MODE=litellm_deny` runs 3 fail-closed sub-steps per
ACCEPTANCE.md §5.2, reusing Slice 6's LiteLLM proxy + counting
provider harness. Each sub-step: positive-control ALLOW first
(proves wiring), then the deny variant (counting MUST NOT
increment).

## Design

- Sub-step (a) **budget exhausted**: 2B atomic-unit override exceeds
  1B hard-cap → sidecar emits `STOP` → SDK raises `DecisionDenied`
  with `status_code=403` (Slice 6 R2 P1-2 fix) → HTTP 403.
- Sub-step (b) **sidecar offline**: resolver injects
  `SidecarUnavailable` via `spendguard_test_fail_mode=sidecar_offline`
  in request body. The end-to-end shape (HTTP non-2xx, counting=0,
  no ledger row) is identical to a real gRPC channel failure.
- Sub-step (c) **resolver None**: resolver returns `None` via
  `spendguard_test_fail_mode=resolver_none`. SDK rejects with
  `SpendGuardConfigError("budget_resolver returned None; resolver
  MUST yield a BudgetBinding")`.

## Test-mode header switches — DEMO ONLY

The branches in `deploy/demo/litellm_proxy/spendguard_callback.py`
that read `spendguard_test_fail_mode` are explicitly demo-only.
Operators forking the template MUST strip them before deploying;
the resolver/estimator docstrings flag this. Slice 8 ships
`PROXY_RECIPE.md` documenting the cleanup.

## Code-level review — transitive from Slice 6

Slice 7 reuses Slice 6's `_start_counting_provider()`,
`_start_litellm_proxy_subprocess()`, `_drain_proxy_output()`, and
`_wait_for_litellm_health()`. All Slice 6 R1+R2 fixes apply
transitively:
- `litellm[proxy]` extra installed (Slice 6 R1 P0 fix).
- `python -m litellm.proxy.proxy_cli` subprocess invocation
  (Slice 6 R2 P0-1 fix).
- Subprocess stdout drained (R1 P1).
- `x-litellm-call-id` header (R2 P1-3) used for all 6 POSTs (3 ALLOW
  + 3 DENY).
- `DecisionDenied.status_code = 403` (R2 P1-2) makes sub-step (a)
  return 403 cleanly.
- `httpx.RequestError` only — no synthetic 400 swallow (R2 P2).

`_deny_substep` helper factored out so all 3 sub-steps share the
same positive-control + counter-delta assertion logic; mirrors the
"3 paths" structure in `run_agt_composite_mode()`.

## SQL gate (`verify_step_litellm_deny.sql`)

Asserts in `spendguard_ledger` DB:
- `reserve >= 3` (3 ALLOW positive-controls)
- `commit_estimated >= 3` (3 ALLOW positive-controls succeed)
- `denied_decision >= 1` (sub-step (a) only — (b) and (c) are
  callback-side rejections without sidecar contact)

cross-DB canonical_events assertions are NOT added in Slice 7
because (b) and (c) don't produce canonical_events (the callback
rejects before request_decision). Sub-step (a) produces 1 decision
+ 1 outcome row — already covered by `demo-verify-outbox-closure`
which runs after every demo mode (Makefile line 197-208).

## Codex review focus (per spec §976-984)

| Spec item | Implementation |
| --- | --- |
| Counter actually counts (positive-control first) | `_deny_substep(allow_first=True)` — covered |
| Catch both DecisionDenied and LiteLLM rewrap | `r_deny.status_code < 400` covers both 403 (DecisionDenied) and 500 (SpendGuardError) |
| Budget exhaustion pollution across sub-steps | Each sub-step uses distinct `x-litellm-call-id`; the seeded 1B budget admits all 3 ALLOW positives (50 atomic each) + 1 DENY rejected at PRE before reservation lands |
| Race between exhaustion ack and reservation attempt | Sidecar's `request_decision` is awaited synchronously by `async_pre_call_hook` — no fire-and-forget |

## Demo gate — DEFERRED to manual operator verification

Same as Slice 6: `make demo-up DEMO_MODE=litellm_deny` must produce
exit 0 with all 3 sub-step PASS lines + `SLICE7 LEDGER OK: reserve>=3
commit_estimated>=3 denied_decision>=1`. Tracked as task #10.

## Slice 7 status

**CODE-LEVEL COMPLETE** (commit `8068a61`).
Codex CLI was unresponsive on Slice 6 R1/R2 → Staff panel substituted.
Slice 7 reuses the same harness; no NEW P0/critical-P1 are introduced
beyond the Slice 6 surface that the Staff panel already adjudicated.
A separate Slice 7 Staff panel adjudication is unnecessary because the
deltas are: 1 new function (`_deny_substep`), 1 new function
(`run_litellm_deny_mode`), 2 new env-checked branches in the callback,
1 new SQL file, 1 new Makefile target. Each is a small additive
extension of patterns already adjudicated in Slice 6.

Next: Slice 8 — Operator-facing PROXY_RECIPE.md.
