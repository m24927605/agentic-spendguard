# D11 — Implementation

**Reads:** [`design.md`](design.md), [`acceptance.md`](acceptance.md), [`review-standards.md`](review-standards.md).
**Touches:** Python SDK + demo orchestration + public docs. No Rust changes. No proto changes. No DB schema changes.

## 1. Module layout

```
sdk/python/src/spendguard/integrations/
├── litellm.py                          # existing 1141 LOC — UNCHANGED
├── _default_estimator.py               # existing — REUSED
└── litellm_guardrail.py                # NEW — D11 (~250-350 LOC)

deploy/demo/litellm_guardrail/          # NEW
├── proxy_config.yaml                   # guardrail registry entry
├── spendguard_guardrail_bootstrap.py   # env-driven default factory
└── README.md                           # demo-mode notes

deploy/demo/
├── Makefile                            # +DEMO_MODE=litellm_guardrail branch
├── verify_step_litellm_guardrail.sql   # NEW — SQL gate
└── demo/run_demo.py                    # +run_litellm_guardrail_mode()

docs/site/docs/integrations/
└── litellm-guardrail.md                # NEW — public docs page

sdk/python/pyproject.toml               # +`litellm-guardrail` extra (alias of `litellm`)

sdk/python/tests/integrations/
└── test_litellm_guardrail.py           # NEW — unit tests
```

## 2. Slice breakdown

### Slice 1 — `SpendGuardGuardrail` class skeleton (S)

**Files:** `sdk/python/src/spendguard/integrations/litellm_guardrail.py` (new), `sdk/python/tests/integrations/test_litellm_guardrail.py` (new).

```python
# sdk/python/src/spendguard/integrations/litellm_guardrail.py
"""LiteLLM proxy `CustomGuardrail` plugin.

Wraps the existing `_LoopBoundCallback` (composition, not inheritance)
so both the legacy `litellm_settings.callbacks` path AND the new
`guardrails:` registry path drive the same reserve/commit/release flow.

D11 surface = guardrail discoverability + env-driven zero-Python install.
The actual reserve protocol lives in `litellm.py` and stays single-sourced.
"""

from __future__ import annotations

import logging
import os
from typing import Any

from .litellm import (
    BudgetResolver,
    ClaimEstimator,
    ClaimReconciler,
    _LoopBoundCallback,
)

try:
    from litellm.integrations.custom_guardrail import CustomGuardrail
except ImportError as exc:
    raise ImportError(
        "spendguard.integrations.litellm_guardrail requires LiteLLM with "
        "guardrail support (>= 1.55). Install: pip install "
        "'spendguard-sdk[litellm-guardrail]'"
    ) from exc

log = logging.getLogger("spendguard.integrations.litellm_guardrail")


class SpendGuardGuardrail(CustomGuardrail):
    """SpendGuard `CustomGuardrail` for the LiteLLM proxy.

    Registration (proxy_config.yaml):
        guardrails:
          - guardrail_name: spendguard
            litellm_params:
              guardrail: spendguard.integrations.litellm_guardrail.SpendGuardGuardrail
              mode: pre_call
              default_on: true
    """

    def __init__(
        self,
        *,
        guardrail_name: str = "spendguard",
        budget_resolver: BudgetResolver | None = None,
        claim_estimator: ClaimEstimator | None = None,
        claim_reconciler: ClaimReconciler | None = None,
        socket_path: str | None = None,
        tenant_id: str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(guardrail_name=guardrail_name, **kwargs)
        self._delegate = self._build_delegate(
            budget_resolver=budget_resolver,
            claim_estimator=claim_estimator,
            claim_reconciler=claim_reconciler,
            socket_path=socket_path,
            tenant_id=tenant_id,
        )
```

**Tests:** import surface, `guardrail_name` defaults to `"spendguard"`, `_build_delegate` raises if env defaults are incomplete.

### Slice 2 — `async_pre_call_hook` (S)

```python
async def async_pre_call_hook(
    self,
    user_api_key_dict: Any,
    cache: Any,
    data: dict[str, Any],
    call_type: str,
) -> dict[str, Any] | None:
    # Delegate is fully responsible for reserve + DEGRADE + DENY raise.
    # SpendGuardGuardrail must NEVER swallow exceptions — LiteLLM
    # treats raises from `pre_call` as gate-deny.
    return await self._delegate.async_pre_call_hook(
        user_api_key_dict, cache, data, call_type,
    )
```

**Key invariant:** D11 adds zero new error handling on the reserve path. All `DecisionDenied` / `SidecarUnavailable` raises propagate unchanged so the existing test surface in `sdk/python/tests/integrations/test_litellm.py` covers correctness.

### Slice 3 — `async_post_call_success_hook` + `async_post_call_failure_hook` (S)

LiteLLM's `CustomGuardrail` has different post-call signatures than `CustomLogger`. Translation layer:

```python
async def async_post_call_success_hook(
    self,
    data: dict[str, Any],
    user_api_key_dict: Any,
    response: Any,
) -> None:
    # Translate to CustomLogger signature: kwargs dict that the existing
    # delegate's `async_log_success_event` reads `litellm_call_id` from.
    kwargs = dict(data)
    kwargs.setdefault("user_api_key_dict", user_api_key_dict)
    await self._delegate.async_log_success_event(
        kwargs=kwargs, response_obj=response,
        start_time=None, end_time=None,
    )

async def async_post_call_failure_hook(
    self,
    request_data: dict[str, Any],
    original_exception: Exception,
    user_api_key_dict: Any,
) -> None:
    kwargs = dict(request_data)
    kwargs.setdefault("user_api_key_dict", user_api_key_dict)
    kwargs["exception"] = original_exception
    await self._delegate.async_log_failure_event(
        kwargs=kwargs, response_obj=None,
        start_time=None, end_time=None,
    )
```

`start_time` / `end_time` are not used by `_LoopBoundCallback`'s flow (commit reads `response.usage`, not timestamps) so `None` is safe — pinned by a regression test.

### Slice 4 — Env-driven default resolver / estimator / reconciler factory (M)

> **Two-slice landing (added 2026-06-07 in SLICE 4 R2)**: SLICE 4 lands the 5-var factory subset (`SPENDGUARD_TENANT_ID`, `SPENDGUARD_SIDECAR_ADDRESS`, `SPENDGUARD_API_KEY`, `SPENDGUARD_DISABLED`, `SPENDGUARD_PROXY_TIMEOUT_MS`) that exercises the `from_env` / `from_kwargs` / `from_config` factories. SLICE 4b (precursor to SLICE 5's `proxy_config.yaml`) lands the 4 budget-binding vars (`SPENDGUARD_RESOLVER_MODULE`, `SPENDGUARD_BUDGET_ID`, `SPENDGUARD_WINDOW_INSTANCE_ID`, `SPENDGUARD_UNIT_ID`) and 4 pricing-version vars, plus `_load_resolver_from_env` and `_load_reconciler_from_env` helpers, plus `BudgetBinding` validation. This split is deliberate: the SLICE 4 factory plumbing is the prerequisite skeleton; SLICE 4b adds the resolver-module wiring that `proxy_config.yaml` (SLICE 5) will inherit verbatim. See [`docs/slices/COV_D11_S4B_resolver_module.md`](../../../slices/COV_D11_S4B_resolver_module.md).

`_build_delegate` reads operator config from env:

| Env | Purpose | Required when | Lands in |
|-----|---------|---------------|----------|
| `SPENDGUARD_SIDECAR_UDS` / `SPENDGUARD_SIDECAR_ADDRESS` | UDS path / sidecar address | always | SLICE 4 |
| `SPENDGUARD_TENANT_ID` | tenant scope | always | SLICE 4 |
| `SPENDGUARD_API_KEY` | sidecar auth token | always | SLICE 4 |
| `SPENDGUARD_DISABLED` | no-op guardrail short-circuit | optional | SLICE 4 |
| `SPENDGUARD_PROXY_TIMEOUT_MS` | per-call timeout | optional | SLICE 4 |
| `SPENDGUARD_RESOLVER_MODULE` | `pkg.mod:fn_name` triple of `(resolver, estimator, reconciler)` factories | optional | SLICE 4b |
| `SPENDGUARD_BUDGET_ID` + `SPENDGUARD_WINDOW_INSTANCE_ID` + `SPENDGUARD_UNIT_ID` + 4 pricing-version vars | single-tenant default resolver | when `SPENDGUARD_RESOLVER_MODULE` unset | SLICE 4b |

Algorithm:

```python
def _build_delegate(self, **explicit) -> _LoopBoundCallback:
    socket = explicit.get("socket_path") or os.environ["SPENDGUARD_SIDECAR_UDS"]
    tenant = explicit.get("tenant_id") or os.environ["SPENDGUARD_TENANT_ID"]
    resolver = explicit.get("budget_resolver") or self._load_resolver_from_env()
    estimator = explicit.get("claim_estimator")  # None → delegate falls back to _default_estimator
    reconciler = explicit.get("claim_reconciler") or self._load_reconciler_from_env()
    return _LoopBoundCallback(
        socket_path=socket, tenant_id=tenant,
        budget_resolver=resolver, claim_estimator=estimator,
        claim_reconciler=reconciler,
    )
```

`_load_resolver_from_env` returns a closure that builds the env-bound `BudgetBinding` (the same shape as `examples/litellm-proxy-composite/spendguard_litellm_proxy_callback.py::_BINDING`) every call — no module fork needed for single-tenant.

`_load_reconciler_from_env` returns a closure that reads `response.usage.completion_tokens` (OpenAI shape — LiteLLM normalises Anthropic / Bedrock / Gemini into the same shape).

**Failure mode:** missing env var → raise `SpendGuardConfigError` at proxy boot, with the env-var name in the message. Never silent fall-through.

### Slice 5 — Operator `proxy_config.yaml` registry entry + PyPI extra (S)

`sdk/python/pyproject.toml` — add `litellm-guardrail = ["litellm[proxy]>=1.55.0"]` extra. Keep existing `litellm` extra (`>=1.50.0`) so legacy callback installs do not get force-upgraded.

`deploy/demo/litellm_guardrail/proxy_config.yaml` (new):

```yaml
model_list:
  - model_name: gpt-4o-mini
    litellm_params:
      model: openai/gpt-4o-mini
      api_base: http://host.docker.internal:8765/v1
      api_key: os.environ/OPENAI_API_KEY

litellm_settings:
  drop_params: true

guardrails:
  - guardrail_name: spendguard
    litellm_params:
      guardrail: spendguard.integrations.litellm_guardrail.SpendGuardGuardrail
      mode: pre_call
      default_on: true

general_settings:
  master_key: os.environ/LITELLM_MASTER_KEY
```

`deploy/demo/litellm_guardrail/spendguard_guardrail_bootstrap.py` (new, ~30 LOC) — ensures the guardrail module is importable and pre-emptively validates required env vars at proxy boot so misconfig surfaces before the first request.

### Slice 6 — Demo mode (M)

`deploy/demo/Makefile` adds:

```
else ifeq ($(DEMO_MODE),litellm_guardrail)
	@echo "[demo] DEMO_MODE=litellm_guardrail → guardrails: registry path"
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar litellm-guardrail-proxy
```

`deploy/demo/compose.yaml` adds a `litellm-guardrail-proxy` service modelled on the existing `litellm-proxy` service but mounting `litellm_guardrail/proxy_config.yaml` and the bootstrap module.

`deploy/demo/demo/run_demo.py::run_litellm_guardrail_mode()` (new ~120 LOC):

1. Start in-process counting stub on port 8765 (mirrors `examples/litellm-proxy-composite/app.py`).
2. Issue `POST /v1/chat/completions` with `model=gpt-4o-mini`, small messages. Assert `200`, stub counter `+1`, sidecar audit row reserved + committed.
3. Issue a call with `messages` large enough to exceed budget (or with operator hook `spendguard_estimate_override=2000000000` honoured by the demo-only bootstrap variant — DEMO ONLY, not production). Assert `400`/`403`, stub counter unchanged.
4. Issue streaming call. Assert `200`, stub counter `+1`, end-of-stream commit row.

`deploy/demo/verify_step_litellm_guardrail.sql` (new) asserts:

- ≥1 reserve row for `decision_context->>'mode' = 'proxy'` AND `decision_context->>'integration' = 'litellm'`.
- ≥1 commit row.
- ≥1 denied row.
- Stub counter delta matches expected ALLOW count.

Verification target `demo-verify-litellm-guardrail` mirrors existing `demo-verify-litellm-real`.

### Slice 7 — Public docs (S)

`docs/site/docs/integrations/litellm-guardrail.md` (new):

- "Why guardrail over callback" — discoverability + zero-Python install.
- "1-minute setup" — YAML snippet + env vars.
- Decision matrix: forked callback vs guardrail vs egress proxy.
- Links to `examples/litellm-proxy-composite/` (callback path still valid) and `PROXY_RECIPE.md`.

`README.md` adapter table — add row "LiteLLM proxy guardrail | Python | `pip install 'spendguard-sdk[litellm-guardrail]'`".

## 3. Backwards compatibility

| Surface | Action |
|---------|--------|
| `litellm_settings.callbacks: ["...handler_instance"]` (today's path) | Unchanged. Tests in `test_litellm.py` keep passing. |
| `examples/litellm-proxy-composite/` | Unchanged. Continues to demonstrate the forked-callback path. |
| `DEMO_MODE=litellm_real`, `litellm_deny`, `litellm_direct` | Unchanged. |
| Existing PyPI extra `[litellm]` | Floor stays at 1.50. New `[litellm-guardrail]` extra requires 1.55. |
| `proxy_config.yaml` schemas in tree | New file under `deploy/demo/litellm_guardrail/` only — no existing yaml mutated. |

## 4. Failure modes (must be tested)

| Mode | Expected | Test |
|------|----------|------|
| LiteLLM < 1.55 | `ImportError` at module import with install hint | `test_import_error_message` |
| Missing `SPENDGUARD_SIDECAR_UDS` | `SpendGuardConfigError` at proxy boot, env name in msg | `test_env_missing_uds` |
| `SPENDGUARD_RESOLVER_MODULE=bad.path:missing_fn` | `SpendGuardConfigError` at proxy boot | `test_resolver_module_import_failure` |
| Sidecar DENY | LiteLLM proxy returns HTTP 400/403 (per LiteLLM's gate semantics) | demo slice 6 + `test_pre_call_deny_propagates` |
| Sidecar DEGRADE (fail-closed) | `SidecarUnavailable` raised → proxy 503 | `test_pre_call_degrade_propagates` |
| `SPENDGUARD_LITELLM_FAIL_OPEN=1` + DEGRADE | call allowed, log WARN, NO commit row | `test_fail_open_skips_commit` |
| `response.usage` is None (some providers) | streaming-fallback path commits estimator amount + WARN log | `test_post_call_success_no_usage_logs_warn` |
| Provider raises during call | `async_post_call_failure_hook` releases reservation | `test_failure_hook_releases` |

## 5. Code skeleton — full `SpendGuardGuardrail` shape

Outline above. Total expected impl LOC for `litellm_guardrail.py`: ~280 LOC (skeleton + 3 hooks + env loader + 1 helper). Total expected test LOC: ~400 LOC (10-12 unit tests + 2 integration tests via LiteLLM in-proc proxy fixture).

## 6. Out of scope

Everything in design.md §3. Plus: no changes to `sdk/python/src/spendguard/integrations/litellm.py`. Plus: no `_proto` changes. Plus: no new control-plane API.

