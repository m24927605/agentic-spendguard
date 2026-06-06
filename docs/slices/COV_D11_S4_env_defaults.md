# COV_D11_S4 — D11 LiteLLM proxy plugin: env-driven default factory

> **Deliverable**: D11 LiteLLM `async_pre_call_hook` proxy guardrail plugin
> **Slice**: 4 of 7 (M)
> **Spec set**: [`docs/specs/coverage/D11_litellm_proxy_plugin/`](../specs/coverage/D11_litellm_proxy_plugin/)

## Scope

Wire the env-driven default factory so adapter authors can construct a `SpendGuardGuardrail` from environment variables — no kwargs needed. Mirrors the SLICE 1-3 hooks already shipped (composition over inheritance with `_LoopBoundCallback` delegate); SLICE 4 adds the factory + env-var resolution pipeline.

Concretely:
- `sdk/python/src/spendguard/integrations/litellm_guardrail.py`:
  - Add `@classmethod def from_env(cls) -> "SpendGuardGuardrail":`
    - Reads `SPENDGUARD_TENANT_ID`, `SPENDGUARD_SIDECAR_ADDRESS`, `SPENDGUARD_API_KEY` (or whatever the Python SDK already standardizes for the underlying SpendGuard client config)
    - Defaults: matches the Python SDK's existing `SpendGuardClient.from_env()` pattern if it exists; otherwise raises `ConfigError` with explicit "missing var X" messages
    - Returns a fully-initialized SpendGuardGuardrail with `_LoopBoundCallback` constructed against the env-resolved client
  - Add `@classmethod def from_kwargs(cls, **kwargs)` as the explicit-kwargs constructor (keeps the existing `__init__` path tidy if needed)
  - Add `@classmethod def from_config(cls, config: dict)` factory accepting a parsed config dict (precursor to SLICE 5's proxy_config.yaml entry)
- `sdk/python/src/spendguard/integrations/_loop_bound_callback.py` (or wherever the underlying delegate lives):
  - Confirm it accepts an env-constructed SpendGuardClient or factory function; add a `from_env` constructor if missing
- `sdk/python/tests/integrations/test_litellm_guardrail_factories.py` — NEW:
  - ≥ 10 tests covering:
    - `from_env()` success with all required env vars set
    - `from_env()` raises ConfigError when SPENDGUARD_TENANT_ID is missing
    - `from_env()` raises ConfigError when SPENDGUARD_SIDECAR_ADDRESS is missing
    - `from_env()` propagates SPENDGUARD_DISABLED=true → returns no-op guardrail (mirrors TS disabled-mode)
    - `from_env()` parses SPENDGUARD_PROXY_TIMEOUT_MS to ms duration
    - `from_kwargs(...)` passes through to `_LoopBoundCallback`
    - `from_config({...})` accepts the dict shape that proxy_config.yaml will parse to (SLICE 5 prep)
    - All 3 hooks (pre/success/failure) still delegate correctly after env construction
    - Idempotent: `from_env()` called twice returns separate instances (no module-level singleton)
    - Env var override precedence: kwargs path AND env path are deliberately separate (no env-fallback when kwargs are provided); precedence within env path is env > defaults

## Files touched

| File | Why |
|------|-----|
| `sdk/python/src/spendguard/integrations/litellm_guardrail.py` | from_env / from_kwargs / from_config classmethods |
| `sdk/python/src/spendguard/integrations/_loop_bound_callback.py` | (if needed) from_env on the underlying delegate |
| `sdk/python/tests/integrations/test_litellm_guardrail_factories.py` | NEW factory + env tests |
| `sdk/python/tests/integrations/__init__.py` | (if needed) ensure test dir is discoverable |

## Test/verification plan

1. `cd sdk/python && python -m pytest tests/integrations/test_litellm_guardrail_factories.py -v` — ≥ 10 new tests pass.
2. `cd sdk/python && python -m pytest tests/integrations/` — SLICE 1+2+3 regression all pass (15 + 15 + 17 = 47 baseline + 10 new = 57+).
3. `cd sdk/python && python -m pytest` — full SDK test suite ≥ 886 + 10 = 896+ passing.
4. `python -c "import os; os.environ['SPENDGUARD_TENANT_ID']='t-test'; os.environ['SPENDGUARD_SIDECAR_ADDRESS']='unix:///tmp/sock'; from spendguard.integrations.litellm_guardrail import SpendGuardGuardrail; g = SpendGuardGuardrail.from_env(); print(type(g).__name__)"` — smoke test.

## Anti-scope

- No `proxy_config.yaml` entry — SLICE 5.
- No demo mode — SLICE 6.
- No docs page — SLICE 7.
- No new hook bodies — already wired in SLICE 1-3.

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D11_litellm_proxy_plugin/design.md) §6 slice 4 row, §7 interfaces, §8 env vars
- review-standards: §3.3 factory invariants
- SLICE 1: [`COV_D11_S1_guardrail_class.md`](COV_D11_S1_guardrail_class.md)
- SLICE 2: [`COV_D11_S2_pre_call.md`](COV_D11_S2_pre_call.md)
- SLICE 3: [`COV_D11_S3_commit_release.md`](COV_D11_S3_commit_release.md)
