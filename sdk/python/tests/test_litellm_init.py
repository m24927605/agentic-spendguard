# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S106
"""Slice 2 — SpendGuardLiteLLMCallback __init__ tests
(env-var reads, WARNING semantics, TTL validation). Split from
test_litellm_precall_unit.py per R2 P2.1 (≤200 LOC per file budget).
"""

from __future__ import annotations

from dataclasses import dataclass
from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock

import pytest

litellm = pytest.importorskip(
    "litellm.integrations.custom_logger",
    reason="LiteLLM not installed; install spendguard-sdk[litellm]",
)

from spendguard.errors import SpendGuardConfigError  # noqa: E402
from spendguard.integrations.litellm import (  # noqa: E402
    BudgetBinding,
    SpendGuardLiteLLMCallback,
)


@dataclass(frozen=True)
class _FakePricing:
    pricing_version: str = "v1"
    price_snapshot_hash_hex: str = "deadbeef"
    fx_rate_version: str = "fxv1"
    unit_conversion_version: str = "uv1"


def _make_callback():
    client = MagicMock()
    client.tenant_id = "tenant-1"
    client.session_id = "session-1"
    client.request_decision = AsyncMock()
    return SpendGuardLiteLLMCallback(
        client=client,
        budget_resolver=lambda ctx: BudgetBinding(
            budget_id="b1", window_instance_id="w1",
            unit=SimpleNamespace(unit_id="u1"),
            pricing=_FakePricing(),
        ),
        claim_estimator=lambda ctx: [],
        claim_reconciler=lambda ctx, resp: [],
    )


def test_init_reads_fail_open_env_at_construction(monkeypatch, caplog):
    """S6: FAIL_OPEN=1 logs WARNING at construction (not just at use)."""
    monkeypatch.setenv("SPENDGUARD_LITELLM_FAIL_OPEN", "1")
    import logging as _logging
    with caplog.at_level(_logging.WARNING, logger="spendguard.integrations.litellm"):
        _make_callback()
    assert any("FAIL_OPEN=1" in rec.message for rec in caplog.records)


def test_init_no_warning_when_fail_open_unset(monkeypatch, caplog):
    monkeypatch.delenv("SPENDGUARD_LITELLM_FAIL_OPEN", raising=False)
    import logging as _logging
    with caplog.at_level(_logging.WARNING, logger="spendguard.integrations.litellm"):
        cb = _make_callback()
    assert cb._fail_open_dev is False
    assert not any("FAIL_OPEN=1" in rec.message for rec in caplog.records)


def test_init_rejects_negative_ttl_seconds(monkeypatch):
    monkeypatch.setenv("SPENDGUARD_LITELLM_TTL_SECONDS", "-1")
    with pytest.raises(SpendGuardConfigError, match="non-negative"):
        _make_callback()


def test_init_rejects_non_integer_ttl_seconds(monkeypatch):
    monkeypatch.setenv("SPENDGUARD_LITELLM_TTL_SECONDS", "notanumber")
    with pytest.raises(SpendGuardConfigError, match="non-negative"):
        _make_callback()


def test_init_default_ttl_seconds_is_300(monkeypatch):
    monkeypatch.delenv("SPENDGUARD_LITELLM_TTL_SECONDS", raising=False)
    cb = _make_callback()
    assert cb._ttl_seconds == 300


def test_init_custom_ttl_seconds(monkeypatch):
    monkeypatch.setenv("SPENDGUARD_LITELLM_TTL_SECONDS", "600")
    cb = _make_callback()
    assert cb._ttl_seconds == 600


def test_init_fail_open_only_one_flips_to_true(monkeypatch):
    """SDK convention: FAIL_OPEN=1 only, not =true/=yes."""
    for val in ["true", "yes", "TRUE", "1.0", "True", ""]:
        monkeypatch.setenv("SPENDGUARD_LITELLM_FAIL_OPEN", val)
        cb = _make_callback()
        assert cb._fail_open_dev is False, f"unexpected fail-open for {val!r}"
    monkeypatch.setenv("SPENDGUARD_LITELLM_FAIL_OPEN", "1")
    cb = _make_callback()
    assert cb._fail_open_dev is True
