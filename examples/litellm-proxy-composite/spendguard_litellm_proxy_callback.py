"""Operator-facing SpendGuard ↔ LiteLLM proxy callback (Slice B3).

This file is the **stripped** template — fork as-is and customise the
three operator hooks (`_resolve`, `_estimate`, `_reconcile`) for your
production binding/pricing/usage logic. The in-tree demo callback
(`deploy/demo/litellm_proxy/spendguard_callback.py`) extends this with
`spendguard_test_fail_mode` + `spendguard_estimate_override` branches
that exist purely to drive the deny-mode demo — operators must NOT
honour those in production.

See `docs/specs/litellm-integration/PROXY_RECIPE.md` §2 for the full
explanation of each hook.
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


def _require_env(name: str) -> str:
    val = os.environ.get(name)
    if not val:
        raise RuntimeError(
            f"example_callback: env var {name} required at LiteLLM proxy boot. "
            "See docker-compose.yml for the standard env-var set."
        )
    return val


_SOCKET_PATH = _require_env("SPENDGUARD_SIDECAR_UDS")
_TENANT_ID = _require_env("SPENDGUARD_TENANT_ID")
_BUDGET_ID = _require_env("SPENDGUARD_BUDGET_ID")
_WINDOW_ID = _require_env("SPENDGUARD_WINDOW_INSTANCE_ID")
_UNIT_ID = _require_env("SPENDGUARD_UNIT_ID")
_PRICING_VERSION = _require_env("SPENDGUARD_PRICING_VERSION")
_FX_RATE_VERSION = _require_env("SPENDGUARD_FX_RATE_VERSION")
_UNIT_CONVERSION_VERSION = _require_env("SPENDGUARD_UNIT_CONVERSION_VERSION")
_PRICE_SNAPSHOT_HASH_HEX = _require_env("SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX")


_UNIT_REF = common_pb2.UnitRef(
    unit_id=_UNIT_ID, token_kind="output_token", model_family="gpt-4",
)
_PRICING = common_pb2.PricingFreeze(
    pricing_version=_PRICING_VERSION,
    price_snapshot_hash=bytes.fromhex(_PRICE_SNAPSHOT_HASH_HEX),
    fx_rate_version=_FX_RATE_VERSION,
    unit_conversion_version=_UNIT_CONVERSION_VERSION,
)
_BINDING = BudgetBinding(
    budget_id=_BUDGET_ID, window_instance_id=_WINDOW_ID,
    unit=_UNIT_REF, pricing=_PRICING,
)


# --------------------------------------------------------------------
# Operator hooks — customise these for your production setup.
# --------------------------------------------------------------------


def _resolve(ctx: ResolverContext) -> BudgetBinding | None:
    """Return the BudgetBinding for the caller's tenant/team.

    For multi-team production: inspect `ctx.user_api_key_dict.team_id`
    (LiteLLM proxy's virtual-key auth populates this) and look up the
    binding in your control plane (Postgres/Redis/etc).

    Return None to deny the request (resolver-side fail-closed). The
    SDK raises `SpendGuardConfigError` which the proxy maps to HTTP
    500 — for a policy-shaped 403 use the budget's hard-cap rule
    instead (see DESIGN §3.4).
    """
    # SINGLE-TENANT EXAMPLE — replace with your real team→binding map.
    return _BINDING


def _estimate(ctx: ResolverContext) -> list[Any]:
    """Worst-case pre-call cost. Inspect `ctx.data` for the request
    body (model + messages) and consult your pricing table.

    For the example we return a flat 50-atomic-unit estimate per call.
    Production: derive from token-count × per-token pricing.
    """
    return [common_pb2.BudgetClaim(
        budget_id=_BUDGET_ID, unit=_UNIT_REF,
        amount_atomic="50",
        direction=common_pb2.BudgetClaim.DEBIT,
        window_instance_id=_WINDOW_ID,
    )]


def _reconcile(ctx: ResolverContext, response_obj: Any) -> list[Any]:
    """Real cost from the provider response. Read
    `response_obj.usage.completion_tokens` (OpenAI shape) — LiteLLM
    normalises Anthropic/Bedrock/Gemini responses into the same shape.

    Production: multiply by your per-token pricing.
    """
    usage = getattr(response_obj, "usage", None)
    tokens = int(getattr(usage, "completion_tokens", 0) or 0)
    return [common_pb2.BudgetClaim(
        budget_id=_BUDGET_ID, unit=_UNIT_REF,
        amount_atomic=str(max(tokens, 1)),  # commit at least 1
        direction=common_pb2.BudgetClaim.DEBIT,
        window_instance_id=_WINDOW_ID,
    )]


# --------------------------------------------------------------------
# Module-level handler instance — referenced by proxy_config.yaml as
# `example_callback.handler_instance`. `_LoopBoundCallback` defers
# gRPC client construction until first hook fires on the proxy's
# serving event loop (Round 3 P0.3 — required for loop-affinity).
# --------------------------------------------------------------------

handler_instance = _LoopBoundCallback(
    socket_path=_SOCKET_PATH,
    tenant_id=_TENANT_ID,
    budget_resolver=_resolve,
    claim_estimator=_estimate,
    claim_reconciler=_reconcile,
)
