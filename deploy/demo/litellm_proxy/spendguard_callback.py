"""SpendGuard callback module imported by LiteLLM proxy (Slice 6).

The LiteLLM proxy's `proxy_config.yaml` references this module via
`litellm_settings.callbacks: ["spendguard_callback.spendguard_handler"]`.
LiteLLM imports the module, looks up the attribute, and uses the
instance as a CustomLogger.

The handler is a `_LoopBoundCallback` so that the gRPC/UDS client is
bound to the proxy's ASGI event loop (LiteLLM imports callbacks at
boot, then runs its own loop — `_LoopBoundCallback` defers client
construction until first hook fires per Round 3 P0.3).

Per DESIGN.md §8.2a the operator owns the budget_resolver,
claim_estimator, and claim_reconciler. For the demo (Slice 6) all
three are trivial closures over the demo's seeded budget. Slice 8
documents how a real operator writes these for multi-tenant routing.
"""

from __future__ import annotations

import os
from types import SimpleNamespace
from typing import Any

from spendguard._proto.spendguard.common.v1 import common_pb2
from spendguard.integrations.litellm import (
    BudgetBinding,
    ResolverContext,
    _LoopBoundCallback,
)


def _env(name: str) -> str:
    """Required env var or hard-fail at proxy boot."""
    val = os.environ.get(name)
    if not val:
        raise RuntimeError(
            f"spendguard_callback: env var {name} required at proxy boot"
        )
    return val


_BUDGET_ID = _env("SPENDGUARD_BUDGET_ID")
_WINDOW_INSTANCE_ID = _env("SPENDGUARD_WINDOW_INSTANCE_ID")
_UNIT_ID = _env("SPENDGUARD_UNIT_ID")
_PRICING_VERSION = _env("SPENDGUARD_PRICING_VERSION")
_FX_RATE_VERSION = _env("SPENDGUARD_FX_RATE_VERSION")
_UNIT_CONVERSION_VERSION = _env("SPENDGUARD_UNIT_CONVERSION_VERSION")
_PRICE_SNAPSHOT_HASH_HEX = _env("SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX")
_TENANT_ID = _env("SPENDGUARD_TENANT_ID")
_SIDECAR_UDS = _env("SPENDGUARD_SIDECAR_UDS")


_UNIT_REF = common_pb2.UnitRef(
    unit_id=_UNIT_ID, token_kind="output_token", model_family="gpt-4",
)
_PRICING_FREEZE = common_pb2.PricingFreeze(
    pricing_version=_PRICING_VERSION,
    price_snapshot_hash=bytes.fromhex(_PRICE_SNAPSHOT_HASH_HEX),
    fx_rate_version=_FX_RATE_VERSION,
    unit_conversion_version=_UNIT_CONVERSION_VERSION,
)
# DESIGN.md §8.2a: the BudgetBinding plumbed to SpendGuard must
# expose `pricing` with the four version fields the sidecar audits.
# `SimpleNamespace` is sufficient because `_validate_claim_against_binding`
# only reads .budget_id / .window_instance_id / .unit.unit_id; the
# pricing fields propagate verbatim to `emit_llm_call_post`.
_BINDING = BudgetBinding(
    budget_id=_BUDGET_ID,
    window_instance_id=_WINDOW_INSTANCE_ID,
    unit=_UNIT_REF,
    pricing=_PRICING_FREEZE,
)


def _budget_resolver(ctx: ResolverContext) -> BudgetBinding:
    """Single-budget demo: every call routes to the seeded demo budget.

    Operators with multi-team setups inspect `ctx.user_api_key_dict`
    (LiteLLM team_id / key_alias) to dispatch. Slice 8 documents this
    pattern in PROXY_RECIPE.md.
    """
    return _BINDING


_DEFAULT_ESTIMATE = "50"


def _claim_estimator(ctx: ResolverContext) -> list[Any]:
    """Worst-case pre-call claim.

    Default: 50 atomic units (the ALLOW step's small estimate, well
    below the 1B hard-cap).

    Per-call override: if the request body contains
    `spendguard_estimate_override` (string-form integer), use it
    instead. This lets the demo's DENY step submit a 2B estimate
    that the hard-cap rule will reject. Operators with multi-team
    setups should NOT rely on this override path — Slice 8's
    PROXY_RECIPE.md documents the production pattern (team_id →
    pricing-table lookup).
    """
    data = getattr(ctx, "data", None) or {}
    override = str(data.get("spendguard_estimate_override", "") or "").strip()
    amount = override if override.isdigit() else _DEFAULT_ESTIMATE
    return [
        common_pb2.BudgetClaim(
            budget_id=_BUDGET_ID,
            unit=_UNIT_REF,
            amount_atomic=amount,
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=_WINDOW_INSTANCE_ID,
        ),
    ]


def _claim_reconciler(ctx: ResolverContext, response_obj: Any) -> list[Any]:
    """Post-call reconciler: derive real cost from `response_obj.usage`.

    The counting HTTP listener emits OpenAI-shaped responses with
    `usage.completion_tokens` set deterministically (see run_demo.py
    `_start_counting_provider`). Slice 6 commits 1 atomic unit per
    completion token — Slice 8 documents the real pricing-table
    derivation for operators.
    """
    usage = getattr(response_obj, "usage", None)
    tokens = int(getattr(usage, "completion_tokens", 0) or 0)
    return [
        common_pb2.BudgetClaim(
            budget_id=_BUDGET_ID,
            unit=_UNIT_REF,
            amount_atomic=str(max(tokens, 1)),  # never commit 0
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=_WINDOW_INSTANCE_ID,
        ),
    ]


# Module-level instance referenced by proxy_config.yaml's
# `litellm_settings.callbacks` entry. `_LoopBoundCallback` defers
# client construction until the first hook fires on LiteLLM's
# serving loop (Round 3 P0.3).
spendguard_handler = _LoopBoundCallback(
    socket_path=_SIDECAR_UDS,
    tenant_id=_TENANT_ID,
    budget_resolver=_budget_resolver,
    claim_estimator=_claim_estimator,
    claim_reconciler=_claim_reconciler,
)
