# ruff: noqa: ANN401, S106
# ANN401: BudgetResolver / ClaimReconciler signatures accept Any per the
# integration's typing contract; the test fixture must match.
# S106: `token_kind="output_token"` is a proto field value, not a
# credential.
"""Test fixture: importable triple-factory for SLICE 4b ``SPENDGUARD_RESOLVER_MODULE``
dispatch (U08).

The ``make_triple`` zero-arg factory returns a tuple
``(BudgetResolver, ClaimEstimator | None, ClaimReconciler)`` where:

* The resolver yields a deterministic ``BudgetBinding`` whose fields
  are distinguishable from the single-tenant env-var defaults so
  tests can assert dispatch went through the operator factory rather
  than falling back to the env-var path.
* The estimator is ``None`` (delegate falls back to
  ``_default_estimator``) — matches the impl.md §4 contract that
  estimator is the only ``None``-able slot.
* The reconciler returns a single-element list with a known sentinel
  amount so tests can assert the commit path also routes through the
  operator factory.

This module is loaded by setting
``SPENDGUARD_RESOLVER_MODULE=tests.integrations.fixtures.fake_resolver:make_triple``
in the test environment and then calling
``SpendGuardGuardrail.from_env()``. ``importlib.import_module`` resolves
the path relative to the SDK ``tests`` rootdir which pytest places on
``sys.path`` automatically.
"""

from __future__ import annotations

from typing import Any

from spendguard._proto.spendguard.common.v1 import common_pb2
from spendguard.integrations.litellm import (
    BudgetBinding,
    BudgetResolver,
    ClaimEstimator,
    ClaimReconciler,
)

# Sentinel values distinguishable from the env-var defaults so the U08
# "single-tenant vars are NOT consulted" invariant can be asserted on
# the resulting binding fields.
FIXTURE_BUDGET_ID = "fixture-budget"
FIXTURE_WINDOW_ID = "fixture-window"
FIXTURE_UNIT_ID = "fixture-unit"
FIXTURE_PRICING_VERSION = "fixture-pricing-v1"
FIXTURE_FX_RATE_VERSION = "fixture-fx-v1"
FIXTURE_UNIT_CONVERSION_VERSION = "fixture-uc-v1"
# 32-byte snapshot hash (matches the production SHA-256 width).
FIXTURE_PRICE_SNAPSHOT_HASH = bytes(range(32))

_FIXTURE_UNIT = common_pb2.UnitRef(
    unit_id=FIXTURE_UNIT_ID,
    token_kind="output_token",
    model_family="gpt-4",
)
_FIXTURE_PRICING = common_pb2.PricingFreeze(
    pricing_version=FIXTURE_PRICING_VERSION,
    price_snapshot_hash=FIXTURE_PRICE_SNAPSHOT_HASH,
    fx_rate_version=FIXTURE_FX_RATE_VERSION,
    unit_conversion_version=FIXTURE_UNIT_CONVERSION_VERSION,
)
_FIXTURE_BINDING = BudgetBinding(
    budget_id=FIXTURE_BUDGET_ID,
    window_instance_id=FIXTURE_WINDOW_ID,
    unit=_FIXTURE_UNIT,
    pricing=_FIXTURE_PRICING,
)


def _resolver(_ctx: Any) -> BudgetBinding:
    return _FIXTURE_BINDING


def _reconciler(_ctx: Any, _response: Any) -> list[Any]:
    # Sentinel amount distinguishable from the single-tenant
    # `max(tokens, 1)` default.
    return [
        common_pb2.BudgetClaim(
            budget_id=FIXTURE_BUDGET_ID,
            unit=_FIXTURE_UNIT,
            amount_atomic="42",
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=FIXTURE_WINDOW_ID,
        ),
    ]


def make_triple() -> tuple[BudgetResolver, ClaimEstimator | None, ClaimReconciler]:
    """Zero-arg factory called by ``_load_resolver_triple`` at boot."""
    return (_resolver, None, _reconciler)


def not_callable() -> str:
    """Helper that returns a non-callable; used to exercise the
    triple-factory invariant check (factory must return a tuple of
    callables)."""
    return "not a callable triple"


# Module-level non-callable attribute used by the
# "attr is not callable" branch of `_load_resolver_triple`.
not_a_function = 42
