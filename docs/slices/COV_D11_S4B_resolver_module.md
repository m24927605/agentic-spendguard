# COV_D11_S4B — D11 LiteLLM proxy plugin: resolver-module + budget-binding env vars

> **Deliverable**: D11 LiteLLM `async_pre_call_hook` proxy guardrail plugin
> **Slice**: 4b of 7 (M)
> **Spec set**: [`docs/specs/coverage/D11_litellm_proxy_plugin/`](../specs/coverage/D11_litellm_proxy_plugin/)

## Scope

Extends SLICE 4's factory layer with resolver-module wiring that SLICE 5's `proxy_config.yaml` will inherit. Carved off SLICE 4 per its R1 reviewer's recommendation: the SLICE 4 factory plumbing is the prerequisite skeleton; SLICE 4b adds the resolver-module wiring that makes `from_env()` produce hook-callable instances instead of cosmetic ones (otherwise `_LoopBoundCallback.async_pre_call_hook` raises `SpendGuardConfigError("budget_resolver returned None")` at `litellm.py:298-302` on first hook invocation).

Concretely:

- Env vars to wire (1 resolver-module + 3 budget-binding + 4 pricing-version):
  - `SPENDGUARD_RESOLVER_MODULE` (`pkg.mod:fn_name` triple of `(resolver, estimator, reconciler)` factories — operator escape hatch for multi-tenant)
  - `SPENDGUARD_BUDGET_ID`
  - `SPENDGUARD_WINDOW_INSTANCE_ID`
  - `SPENDGUARD_UNIT_ID`
  - 4 pricing-version vars (`SPENDGUARD_PRICING_VERSION` + 3 sibling vars — see `implementation.md` §4 row "Pricing version env vars" and the field-by-field mirror in `examples/litellm-proxy-composite/spendguard_litellm_proxy_callback.py`)
- New helpers in `sdk/python/src/spendguard/integrations/litellm_guardrail.py`:
  - `_load_resolver_from_env() -> BudgetResolver` — when `SPENDGUARD_RESOLVER_MODULE` is set, `importlib.import_module(pkg.mod)` + `getattr(mod, fn_name)`; otherwise build a closure that constructs the single-tenant `BudgetBinding` from the 3 budget-binding + 4 pricing-version env vars
  - `_load_reconciler_from_env() -> ClaimReconciler` — same dispatch pattern; default path reads `response.usage.completion_tokens`
  - `_validate_budget_binding(binding: BudgetBinding) -> None` — raises `SpendGuardConfigError` on empty `budget_id` / `window_instance_id` / `unit_id` (mirror `litellm.py:306-315` field-by-field)
- Modify `from_env()` and `_from_parsed_config()` to consume the new vars and call the loader helpers; both surfaces must produce instances that pass `_validate_budget_binding` before returning
- Tests U06-U10 from `tests.md` §2.2 become implementable; add them with the per-test SLICE-PHASING deferral notes resolved

## Files touched

| File | Why |
|------|-----|
| `sdk/python/src/spendguard/integrations/litellm_guardrail.py` | Loader helpers + `from_env`/`_from_parsed_config` extensions + `_validate_budget_binding` |
| `sdk/python/tests/integrations/test_litellm_guardrail_factories.py` | Tests U06-U10 + budget-binding edge cases + pricing-version parse tests |
| `sdk/python/tests/integrations/fixtures/fake_resolver.py` (NEW if missing) | `make_triple` fixture referenced by U08 |

## Test/verification plan

1. `cd sdk/python && python -m pytest tests/integrations/test_litellm_guardrail_factories.py -v` — 40 SLICE 4 baseline + ~12 new SLICE 4b = 52+ passing.
2. `cd sdk/python && python -m pytest tests/integrations/` — 591 SLICE 3+4 baseline + 12 new = 603+ passing.
3. `cd sdk/python && python -m pytest` — full SDK regression, no flakes.
4. Smoke: `from_env()` instance can call `async_pre_call_hook` without raising `SpendGuardConfigError("budget_resolver returned None")` — the resolver is loaded, `BudgetBinding` is non-empty and valid.
5. Smoke: `SPENDGUARD_RESOLVER_MODULE=tests.fixtures.fake_resolver:make_triple` dispatches to operator factory and ignores the single-tenant env vars (U08 invariant).

## Anti-scope

- No `proxy_config.yaml` entry — SLICE 5.
- No new hook bodies — SLICE 1-3 already wired.
- No demo mode — SLICE 6.
- No docs page — SLICE 7.
- No changes to the 5-var SLICE 4 subset (`SPENDGUARD_TENANT_ID`, `SPENDGUARD_SIDECAR_ADDRESS`, `SPENDGUARD_API_KEY`, `SPENDGUARD_DISABLED`, `SPENDGUARD_PROXY_TIMEOUT_MS`) — they already pass SLICE 4 R1.

## Backlinks

- Spec set: [`implementation.md`](../specs/coverage/D11_litellm_proxy_plugin/implementation.md) §4 (env vars, SLICE 4 / SLICE 4b split), [`review-standards.md`](../specs/coverage/D11_litellm_proxy_plugin/review-standards.md) §4.2-4.6 (SLICE-PHASING notes), [`tests.md`](../specs/coverage/D11_litellm_proxy_plugin/tests.md) §2.2 U06-U10
- SLICE 4: [`COV_D11_S4_env_defaults.md`](COV_D11_S4_env_defaults.md)
- SLICE 4 R1 reviewer carve-off rationale: SLICE 4 R2 implementer notes (2026-06-07)
