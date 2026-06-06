# D11 — LiteLLM `async_pre_call_hook` Proxy Guardrail Plugin

**Status:** Spec — Tier 2, build plan `framework-coverage-build-plan-2026-06.md` §2.2.
**Parent strategy:** [`docs/strategy/framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md), "LiteLLM SDK gap" + "Should integrate, not compete".
**Owner sub-agent:** Backend Architect.
**Sibling deliverable:** D12 (SDK monkey-patch shim for direct `litellm.acompletion`).

## 1. Problem

`sdk/python/src/spendguard/integrations/litellm.py` already ships a `CustomLogger`-based proxy callback (`SpendGuardLiteLLMCallback` + `_LoopBoundCallback`) that uses `async_pre_call_hook` to reserve, `async_log_success_event` to commit, `async_log_failure_event` to release. It works against `litellm[proxy]` ≥ 1.50 and is exercised by `examples/litellm-proxy-composite/` + `make demo-up DEMO_MODE=litellm_real|litellm_deny`.

What it does **not** ship:

1. A first-class **`CustomGuardrail`** entry point (LiteLLM's newer guardrail registration surface, distinct from the legacy `litellm_settings.callbacks` list). The guardrail surface is what LiteLLM Cloud, Portkey-as-PANW-property, and most 2026 docs steer operators at. SpendGuard not being present there is a discoverability gap.
2. A `guardrails:` registry entry the operator can copy into `proxy_config.yaml` with **zero callback-module fork**. Today operators must fork `spendguard_litellm_proxy_callback.py`, write `_resolve` / `_estimate` / `_reconcile`, and mount it on `PYTHONPATH`. For 80% of single-tenant deployments that fork is overhead; the guardrail registry plus env-driven defaults should be enough.
3. A demo mode (`DEMO_MODE=litellm_guardrail`) that proves SpendGuard fires **before** the OpenAI HTTP request leaves the proxy, against a real upstream stub, with a hard-cap deny short-circuit.

D11 closes those three gaps. D11 is **additive** to the existing callback — both paths must keep working — and is the foundation D12 (direct-SDK shim) re-uses.

## 2. Goals

1. New `SpendGuardGuardrail(CustomGuardrail)` class registered via `guardrails: [{guardrail_name: spendguard, litellm_params: {guardrail: spendguard_litellm_proxy_plugin.SpendGuardGuardrail, mode: pre_call, default_on: true}}]` in `proxy_config.yaml`.
2. Guardrail's `async_pre_call_hook(user_api_key_dict, cache, data, call_type)` re-uses the **existing** reserve flow in `SpendGuardLiteLLMCallback` (composition, not re-implementation) so identity validation / idempotency / DEGRADE handling stay single-sourced.
3. Env-driven defaults: when the operator does not supply `_resolve` / `_estimate` / `_reconcile`, the guardrail loads the same `_LoopBoundCallback`-style env-bound binding + the `_default_estimator` already in `sdk/python/src/spendguard/integrations/_default_estimator.py`. Operators with one tenant + one budget get a working install with zero Python.
4. Streaming + commit path: `async_post_call_success_hook` and `async_post_call_failure_hook` from `CustomGuardrail` route to the same commit/release flow.
5. Demo: `make demo-up DEMO_MODE=litellm_guardrail` boots `postgres + sidecar + litellm-proxy + counting-stub`, issues 3 calls (ALLOW + DENY + STREAM), asserts pre-call sidecar reservation **before** the upstream stub is hit on each ALLOW + verifies the stub counter does **not** increment on DENY.
6. Public docs page `docs/site/docs/integrations/litellm-guardrail.md` covering both paths (zero-Python guardrail vs forked callback) with a decision matrix.

## 3. Non-goals

- Direct `litellm.acompletion()` SDK gating — D12.
- LiteLLM Issue #8842 upstream fix — separate workstream ("LiteLLM upstream re-engagement" in the strategy memo).
- Replacing the existing `SpendGuardLiteLLMCallback`. D11 wraps it.
- LiteLLM virtual-key / team management plumbing beyond reading `user_api_key_dict.team_id` in the resolver.
- Streaming token-by-token gating mid-stream. End-of-stream commit only (same as today).

## 4. Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│ litellm proxy                                                    │
│   ↓ POST /v1/chat/completions                                    │
│   guardrails registry → SpendGuardGuardrail                      │
│     async_pre_call_hook(data, call_type)                         │
│       → _delegate.async_pre_call_hook(...)  ← composition        │
│           └→ SpendGuardLiteLLMCallback (existing reserve flow)   │
│                ↓ UDS gRPC                                        │
│                sidecar.RequestDecision                           │
│                ↓ ALLOW → return data (proxy fires upstream)      │
│                ↓ DENY  → raise → proxy returns 400/403           │
│                ↓ DEGRADE → raise SidecarUnavailable → 503        │
│     async_post_call_success_hook(response)                       │
│       → _delegate.async_log_success_event(...)  → commit         │
│     async_post_call_failure_hook(exception)                      │
│       → _delegate.async_log_failure_event(...) → release         │
└──────────────────────────────────────────────────────────────────┘
```

Composition (not inheritance) is locked: `SpendGuardGuardrail` holds a `_delegate: _LoopBoundCallback` and forwards every hook. Reasons: (a) `CustomGuardrail` and `CustomLogger` may inherit conflicting state in future LiteLLM releases; (b) keeps the existing 1141-LOC test surface intact; (c) lets D12 wrap `_delegate` the same way without duplicating logic.

## 5. Key decisions

- **`mode: pre_call`** locked. `during_call` returns a transformed `data` mid-flight and is not the gate point we need. `logging_only` skips the reserve entirely.
- **Env-driven zero-Python install** when `SPENDGUARD_LITELLM_RESOLVER_MODULE` is unset. When set (e.g. `myteam.callbacks:resolve_estimate_reconcile`), the guardrail imports and dispatches to the operator's module.
- **`default_on: true`** locked for the registry entry. Operators who want opt-in per virtual key flip to `false` and set per-key `metadata.guardrails: ["spendguard"]`.
- **Fail-closed default** carries over. `SPENDGUARD_LITELLM_FAIL_OPEN=1` remains dev-only escape hatch.
- **Direct-SDK gap NOT closed by D11.** Path B in the existing `docs/site/docs/integrations/litellm.md` still routes direct callers through D12 (or egress proxy Path C).

## 6. Slice plan

| Slice | Title | Size |
|-------|-------|------|
| `COV_D11_S1_guardrail_class` | `SpendGuardGuardrail` skeleton + composition wiring | S |
| `COV_D11_S2_pre_call` | `async_pre_call_hook` reserve via delegate + raise-on-deny | S |
| `COV_D11_S3_commit_release` | success / failure hooks → delegate commit / release | S |
| `COV_D11_S4_env_defaults` | Env-driven default resolver / estimator / reconciler factory | M |
| `COV_D11_S5_proxy_config_entry` | Operator-facing `guardrails:` registry yaml + PyPI extras | S |
| `COV_D11_S6_demo_mode` | `DEMO_MODE=litellm_guardrail` Makefile + verify SQL + 3-step driver | M |
| `COV_D11_S7_docs_page` | `docs/site/docs/integrations/litellm-guardrail.md` + README index entry | S |

7 slices, all S/M, ~1500 LOC total (~700 impl + 500 test + 300 docs / yaml).

## 7. Interfaces

```python
# sdk/python/src/spendguard/integrations/litellm_guardrail.py
class SpendGuardGuardrail(CustomGuardrail):
    def __init__(self, *, guardrail_name: str = "spendguard", **kwargs: Any) -> None: ...
    async def async_pre_call_hook(self, user_api_key_dict, cache, data, call_type) -> dict | None: ...
    async def async_post_call_success_hook(self, data, user_api_key_dict, response) -> None: ...
    async def async_post_call_failure_hook(self, request_data, original_exception, user_api_key_dict) -> None: ...
```

Proxy entry: `guardrails: [{guardrail_name: "spendguard", litellm_params: {guardrail: "spendguard.integrations.litellm_guardrail.SpendGuardGuardrail", mode: "pre_call", default_on: true}}]`.

## 8. Open questions (locked at spec write)

1. **`CustomGuardrail` vs `CustomLogger` co-registration:** both classes are referenced from the same `proxy_config.yaml` without conflict — verified by LiteLLM source. Locked: D11 ships the guardrail; the legacy callback path remains supported.
2. **Pre-call hook signature drift between LiteLLM 1.50 and 1.65+:** LiteLLM kept the `(user_api_key_dict, cache, data, call_type)` shape across both. Locked: support 1.50 floor.
3. **`during_call` mode revisit:** rejected. Gate must fire before any provider HTTP, not mid-stream.

