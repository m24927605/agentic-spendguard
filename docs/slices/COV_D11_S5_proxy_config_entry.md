# COV_D11_S5 — D11 LiteLLM proxy plugin: proxy_config.yaml entry + PyPI extras

> **Deliverable**: D11 LiteLLM `async_pre_call_hook` proxy guardrail plugin
> **Slice**: 5 of 7 (S)
> **Spec set**: [`docs/specs/coverage/D11_litellm_proxy_plugin/`](../specs/coverage/D11_litellm_proxy_plugin/)

## Scope

Land the operator-facing `guardrails:` registry entry that lets a single-tenant deployment drop SpendGuard into proxy_config.yaml WITHOUT forking the callback module. Operators set `SPENDGUARD_RESOLVER_MODULE` + budget-binding env vars (SLICE 4 + 4b) and copy the yaml stanza below.

Plus: PyPI extras so `pip install spendguard-sdk[litellm-proxy]` pulls in `litellm` as a peer dep without forcing every user to install LiteLLM.

Concretely:
- `sdk/python/src/spendguard/integrations/litellm_guardrail.py`:
  - Add a top-level `spendguard_guardrail_factory(litellm_params: dict) -> SpendGuardGuardrail` factory function that LiteLLM's guardrail registry calls during proxy boot
  - Inside: parse litellm_params for SpendGuard config keys (tenant_id, sidecar_address, resolver_module, etc.); fall back to env vars for missing keys
  - Call SpendGuardGuardrail.from_config(parsed) — leverages the SLICE 4 / SLICE 4b dispatch
- `sdk/python/pyproject.toml`:
  - Add `[project.optional-dependencies]` section with `litellm-proxy = ["litellm>=1.40.0"]` (or current LiteLLM min version)
  - Add `[project.entry-points."litellm.guardrails"]` registry entry pointing at the factory (verify LiteLLM uses this entry-point pattern; if not, fall back to module-path resolution in proxy_config.yaml)
- `examples/litellm-proxy/proxy_config.yaml` — NEW:
  - Minimal yaml snippet operators can copy-paste:
    ```yaml
    guardrails:
      - guardrail_name: spendguard
        litellm_params:
          guardrail: spendguard.integrations.litellm_guardrail:spendguard_guardrail_factory
          mode: pre_call
          default_on: true
          # Optional inline config (env vars also supported):
          # tenant_id: my-tenant
          # sidecar_address: unix:///var/run/spendguard.sock
          # resolver_module: myapp.spendguard:make_resolver_triple
    ```
- `sdk/python/tests/integrations/test_litellm_guardrail_proxy_config.py` — NEW (≥8 tests):
  - spendguard_guardrail_factory with full inline config → constructs guardrail
  - spendguard_guardrail_factory with env-var fallback → uses env
  - spendguard_guardrail_factory raises ConfigError on missing tenant_id (no env, no inline)
  - spendguard_guardrail_factory returns distinct instances on repeated invocations (no module-level singleton)
  - Inline `disabled: true` → no-op guardrail
  - Inline `resolver_module:` → loaded via the SLICE 4b dispatch
  - Smoke test: load example proxy_config.yaml as a dict, pass litellm_params to factory, verify guardrail constructed
- `sdk/python/tests/integrations/test_litellm_guardrail_pyproject_extras.py` — NEW (≥3 tests):
  - pyproject.toml [project.optional-dependencies] has 'litellm-proxy' key
  - 'litellm-proxy' extras include 'litellm' with version pin
  - entry-points configuration matches the litellm guardrail registry pattern

## Files touched

| File | Why |
|------|-----|
| `sdk/python/src/spendguard/integrations/litellm_guardrail.py` | spendguard_guardrail_factory function |
| `sdk/python/pyproject.toml` | [project.optional-dependencies] + entry-points |
| `examples/litellm-proxy/proxy_config.yaml` | NEW — operator-facing yaml stanza |
| `sdk/python/tests/integrations/test_litellm_guardrail_proxy_config.py` | NEW — factory + yaml tests |
| `sdk/python/tests/integrations/test_litellm_guardrail_pyproject_extras.py` | NEW — packaging tests |

## Test/verification plan

1. `python -m pytest tests/integrations/test_litellm_guardrail_proxy_config.py` — ≥8 passing
2. `python -m pytest tests/integrations/` — 609 SLICE 4b baseline + ~11 new = 620+ passing
3. `python -m pytest` — full SDK no regressions (944 + 11 = 955+)
4. `python -m ruff check src tests` clean
5. `pip install -e .[litellm-proxy]` succeeds (verifies the extras entry)
6. Smoke: `python -c "from spendguard.integrations.litellm_guardrail import spendguard_guardrail_factory; g = spendguard_guardrail_factory({'tenant_id': 't-test', 'sidecar_address': 'unix:///tmp/sock'}); print(type(g).__name__)"`

## Anti-scope

- No demo mode — SLICE 6
- No docs page — SLICE 7
- No NPM publish equivalent for TS — out of D11 scope (D05 handles that)

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D11_litellm_proxy_plugin/design.md) §6 slice 5 row, §7 yaml interface
- SLICE 4: [`COV_D11_S4_env_defaults.md`](COV_D11_S4_env_defaults.md)
- SLICE 4b: [`COV_D11_S4B_resolver_module.md`](COV_D11_S4B_resolver_module.md)
