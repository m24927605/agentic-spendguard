# SpendGuard + LiteLLM Proxy — Operator Recipe

This is the **operator-facing** recipe for running SpendGuard
governance against the LiteLLM proxy. It complements the SDK
integration covered in DESIGN.md §3.4 (v1 Path B) by showing the
production-shape callback module, multi-tenant resolver pattern, and
proxy auth wiring.

> **Demo vs operator.** The demo callback at
> `deploy/demo/litellm_proxy/spendguard_callback.py` has test-mode
> branches (`spendguard_test_fail_mode`, `spendguard_estimate_override`)
> that operators MUST strip before deploying. The template below is the
> stripped version.

## 1. Install

```bash
pip install 'spendguard-sdk[litellm]'
```

The `litellm` extra pulls `litellm[proxy]` (fastapi + uvicorn +
gunicorn) so `python -m litellm.proxy.proxy_cli` works out of the box.

## 2. Operator callback module

Save as `spendguard_litellm_proxy_callback.py`. The module-level
`handler_instance` is what `proxy_config.yaml` references.

```python
"""SpendGuard LiteLLM proxy callback — operator template.

The LiteLLM proxy imports this module at boot via
`litellm_settings.callbacks: ["spendguard_litellm_proxy_callback.handler_instance"]`.
The `_LoopBoundCallback` lazy-instantiates the gRPC/UDS client on
first hook fire so the channel binds to the proxy's serving event
loop (not the import-time loop).
"""

from __future__ import annotations

import os
from typing import Any

from spendguard._proto.spendguard.common.v1 import common_pb2
from spendguard.integrations.litellm import (
    BudgetBinding,
    ResolverContext,
    _LoopBoundCallback,
)


# --- 2.1 read SpendGuard configuration from env vars ---
_SOCKET_PATH = os.environ["SPENDGUARD_SIDECAR_UDS"]
_TENANT_ID = os.environ["SPENDGUARD_TENANT_ID"]
_PRICING_VERSION = os.environ["SPENDGUARD_PRICING_VERSION"]
_FX_RATE_VERSION = os.environ["SPENDGUARD_FX_RATE_VERSION"]
_UNIT_CONVERSION_VERSION = os.environ["SPENDGUARD_UNIT_CONVERSION_VERSION"]
_PRICE_SNAPSHOT_HASH_HEX = os.environ["SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX"]

_PRICING = common_pb2.PricingFreeze(
    pricing_version=_PRICING_VERSION,
    price_snapshot_hash=bytes.fromhex(_PRICE_SNAPSHOT_HASH_HEX),
    fx_rate_version=_FX_RATE_VERSION,
    unit_conversion_version=_UNIT_CONVERSION_VERSION,
)


# --- 2.2 multi-tenant resolver (ADR-001 Option 2) ---
#
# Per-team binding lookup. In production this typically queries
# Postgres / Redis / control-plane. For brevity we map team_id →
# (budget_id, window_instance_id, unit_id) in a literal dict.

_TEAM_TO_BINDING = {
    # team_id from LiteLLM virtual-key auth → SpendGuard budget triple.
    "team-finance": ("44444444-...", "55555555-...", "66666666-..."),
    "team-product": ("44444444-...", "55555555-...", "66666666-..."),
    # ...
}


def _resolve(ctx: ResolverContext) -> BudgetBinding | None:
    """Return the BudgetBinding for the caller's team, or None to
    deny (resolver-side rejection — fail-closed)."""
    api_key_dict = getattr(ctx, "user_api_key_dict", None) or {}
    team_id = (
        getattr(api_key_dict, "team_id", None)
        or (api_key_dict.get("team_id") if isinstance(api_key_dict, dict) else None)
        or "team-default"
    )
    triple = _TEAM_TO_BINDING.get(team_id)
    if triple is None:
        # Unknown team → fail-closed. Logged + 403 to the caller.
        return None
    budget_id, window_instance_id, unit_id = triple
    return BudgetBinding(
        budget_id=budget_id,
        window_instance_id=window_instance_id,
        unit=common_pb2.UnitRef(
            unit_id=unit_id, token_kind="output_token", model_family="gpt-4",
        ),
        pricing=_PRICING,
    )


# --- 2.3 claim estimator (worst-case pre-call cost) ---
def _estimate(ctx: ResolverContext) -> list[Any]:
    """Per-team / per-model worst-case estimate. Operators wire this
    against their pricing table; for v1 a simple `messages` token-count
    estimate is sufficient."""
    binding = _resolve(ctx)
    assert binding is not None  # _resolve None is caught by SDK pre-call
    # 1 atomic unit per character is a coarse-but-safe upper bound.
    messages = (getattr(ctx, "data", None) or {}).get("messages", [])
    char_count = sum(len(str(m.get("content", ""))) for m in messages)
    return [common_pb2.BudgetClaim(
        budget_id=binding.budget_id,
        unit=binding.unit,
        amount_atomic=str(max(char_count, 1)),
        direction=common_pb2.BudgetClaim.DEBIT,
        window_instance_id=binding.window_instance_id,
    )]


# --- 2.4 reconciler (real cost from response.usage) ---
def _reconcile(ctx: ResolverContext, response_obj: Any) -> list[Any]:
    """Post-call commit amount. Operators derive from the provider's
    `usage` block via their pricing table; here we commit 1 atomic
    unit per completion token as a placeholder."""
    binding = _resolve(ctx)
    assert binding is not None
    usage = getattr(response_obj, "usage", None)
    tokens = int(getattr(usage, "completion_tokens", 0) or 0)
    return [common_pb2.BudgetClaim(
        budget_id=binding.budget_id,
        unit=binding.unit,
        amount_atomic=str(max(tokens, 1)),
        direction=common_pb2.BudgetClaim.DEBIT,
        window_instance_id=binding.window_instance_id,
    )]


# --- 2.5 module-level handler instance ---
# `proxy_config.yaml` references this via the dotted-path string
# `spendguard_litellm_proxy_callback.handler_instance`. The
# `_LoopBoundCallback` defers client construction until first hook
# fires on the proxy's serving event loop.
handler_instance = _LoopBoundCallback(
    socket_path=_SOCKET_PATH,
    tenant_id=_TENANT_ID,
    budget_resolver=_resolve,
    claim_estimator=_estimate,
    claim_reconciler=_reconcile,
)
```

## 3. `proxy_config.yaml`

```yaml
model_list:
  - model_name: gpt-4o-mini
    litellm_params:
      model: openai/gpt-4o-mini
      api_key: os.environ/OPENAI_API_KEY
  - model_name: claude-haiku
    litellm_params:
      model: anthropic/claude-3-5-haiku-latest
      api_key: os.environ/ANTHROPIC_API_KEY

litellm_settings:
  # String dotted-path form (Round 4 P1.5 — proxy YAML accepts string;
  # direct-mode Python uses list of instances).
  callbacks: ["spendguard_litellm_proxy_callback.handler_instance"]
  drop_params: true

general_settings:
  master_key: sk-CHANGEME-rotate-this-per-prod-secret
  # Per-team virtual keys → enables _resolve's team-based dispatch.
  database_url: postgresql://... # required for team management
```

## 4. Launch

```bash
# Tell uvicorn where to find the callback module.
export PYTHONPATH="/path/to/callback-dir:$PYTHONPATH"

# Required SpendGuard env vars (your control plane sets these).
export SPENDGUARD_SIDECAR_UDS=/var/run/spendguard/adapter.sock
export SPENDGUARD_TENANT_ID=...
export SPENDGUARD_PRICING_VERSION=...
# ...

python -m litellm.proxy.proxy_cli --config proxy_config.yaml --port 4000
```

## 5. Shape A fallback for sync callers

`litellm.completion()` (sync) is **not** gated by this callback —
ADR-005 explains why (LiteLLM's logging dispatcher swallows
exceptions from sync hooks). Sync callers route via the SpendGuard
egress proxy (Shape A in DESIGN §3.4 v1 Path A):

```python
import litellm
litellm.api_base = "http://spendguard-egress-proxy:9000/v1"
litellm.completion(model="gpt-4o-mini", messages=[...])
```

## 6. TTL tuning + fail-closed posture

- `SPENDGUARD_LITELLM_TTL_SECONDS=300` (default). Tune up if your
  longest stream takes >5 min. Reservations not committed by TTL get
  released by the sweeper.
- `SPENDGUARD_LITELLM_FAIL_OPEN=1` allows calls through when the
  sidecar is unreachable. **Production deployments must leave this
  unset** — fail-closed is the default and the documented contract.

## 7. Audit join story

For proxy-mode callers, SpendGuard's `canonical_events` chain joins
to LiteLLM's `LiteLLM_SpendLogs` table on `litellm_call_id` (= the
`x-litellm-call-id` header your client sends, or a fresh UUID).
DESIGN.md §8.3 + ACCEPTANCE.md §5.1 Q2 cover the SQL. Slice 9
ships a cross-table verify query.

## 8. Stripping the demo-only branches

Before deploying, audit your callback for any `spendguard_test_*`
branches. The demo's `deploy/demo/litellm_proxy/spendguard_callback.py`
has two test-mode injection points (`spendguard_estimate_override`,
`spendguard_test_fail_mode`) that exist purely to drive the deny
demo. Production callbacks MUST NOT honour these — a malicious
caller could otherwise under-claim or self-deny.
