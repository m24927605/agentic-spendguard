"""SpendGuard guardrail resolver module for `DEMO_MODE=litellm_guardrail`.

`proxy_config.yaml`'s guardrail entry points its `resolver_module:`
field at this module via the dotted spec
`spendguard_guardrail_resolver:make_triple`. The SLICE 4b loader
calls `make_triple()` once at proxy boot and unpacks the returned
`(resolver, estimator, reconciler)` triple into the underlying
`_LoopBoundCallback` (composition delegate of the guardrail).

The resolver / estimator / reconciler mirror the shape of the
`litellm_real` callback (`deploy/demo/litellm_proxy/spendguard_callback.py`)
so the SLICE 6 demo can reuse the same:

  * single-tenant budget binding (env-bound)
  * per-call `spendguard_estimate_override` hook for the DENY step
  * `response.usage.completion_tokens` reconciler

This is demo-only — operators ship their own triple via PyPI or a
mounted PYTHONPATH module (per the SLICE 7 docs page).
"""

from __future__ import annotations

import os
from typing import Any

from spendguard._proto.spendguard.common.v1 import common_pb2
from spendguard.errors import SidecarUnavailable
from spendguard.integrations.litellm import (
    BudgetBinding,
    ResolverContext,
)


def _env(name: str) -> str:
    """Required env var or hard-fail at proxy boot."""
    val = os.environ.get(name)
    if not val:
        raise RuntimeError(
            f"spendguard_guardrail_resolver: env var {name} required at proxy boot"
        )
    return val


def make_triple() -> tuple[Any, Any, Any]:
    """Zero-arg triple-factory called by `_load_resolver_triple`.

    Returns `(resolver, estimator, reconciler)` per SLICE 4b
    contract. All three close over env-bound identity / pricing /
    binding state captured at proxy boot — matches the
    `spendguard_callback.py` (legacy callback) shape exactly so the
    `DEMO_MODE=litellm_guardrail` driver can reuse the same DENY
    override pattern (`spendguard_estimate_override=2000000000`).
    """
    budget_id = _env("SPENDGUARD_BUDGET_ID")
    window_instance_id = _env("SPENDGUARD_WINDOW_INSTANCE_ID")
    unit_id = _env("SPENDGUARD_UNIT_ID")
    pricing_version = _env("SPENDGUARD_PRICING_VERSION")
    fx_rate_version = _env("SPENDGUARD_FX_RATE_VERSION")
    unit_conversion_version = _env("SPENDGUARD_UNIT_CONVERSION_VERSION")
    price_snapshot_hash_hex = _env("SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX")

    unit_ref = common_pb2.UnitRef(
        unit_id=unit_id, token_kind="output_token", model_family="gpt-4",
    )
    pricing_freeze = common_pb2.PricingFreeze(
        pricing_version=pricing_version,
        price_snapshot_hash=bytes.fromhex(price_snapshot_hash_hex),
        fx_rate_version=fx_rate_version,
        unit_conversion_version=unit_conversion_version,
    )
    binding = BudgetBinding(
        budget_id=budget_id,
        window_instance_id=window_instance_id,
        unit=unit_ref,
        pricing=pricing_freeze,
    )

    default_estimate = "50"

    def _resolver(ctx: ResolverContext) -> BudgetBinding | None:
        """Single-budget demo resolver.

        Demo-only test injection: a per-request
        `spendguard_test_fail_mode=sidecar_offline` raises
        `SidecarUnavailable` to exercise fail-closed path. Operators
        MUST strip this from production resolvers.
        """
        data = getattr(ctx, "data", None) or {}
        fail_mode = str(data.get("spendguard_test_fail_mode", "") or "").lower()
        if fail_mode == "sidecar_offline":
            raise SidecarUnavailable(
                "demo-only test injection: simulated sidecar UDS unreachable"
            )
        return binding

    def _estimator(ctx: ResolverContext) -> list[Any]:
        """Worst-case pre-call claim (mirrors the litellm_real
        callback `_claim_estimator`).

        Default: 50 atomic units (small ALLOW well under hard-cap).
        Per-call `spendguard_estimate_override` (string digit) bumps
        the claim — used by the demo driver's DENY step to push above
        the seeded 1B hard-cap so the contract evaluator emits a
        SPENDGUARD_DENY pre-call.
        """
        data = getattr(ctx, "data", None) or {}
        override = str(data.get("spendguard_estimate_override", "") or "").strip()
        amount = override if override.isdigit() else default_estimate
        return [
            common_pb2.BudgetClaim(
                budget_id=budget_id,
                unit=unit_ref,
                amount_atomic=amount,
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id=window_instance_id,
            ),
        ]

    def _reconciler(ctx: ResolverContext, response_obj: Any) -> list[Any]:
        """Post-call reconciler from `response_obj.usage.completion_tokens`.

        Matches `litellm_real`: 1 atomic unit per completion token,
        floor of 1 (never commit 0).
        """
        usage = getattr(response_obj, "usage", None)
        tokens = int(getattr(usage, "completion_tokens", 0) or 0)
        return [
            common_pb2.BudgetClaim(
                budget_id=budget_id,
                unit=unit_ref,
                amount_atomic=str(max(tokens, 1)),
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id=window_instance_id,
            ),
        ]

    return _resolver, _estimator, _reconciler
