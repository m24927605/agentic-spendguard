# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S106
"""Slice 2 R1 follow-up — DEGRADE / multi-reservation / claim-binding
mismatch outcomes. Per TEST_PLAN §2.2 extensions.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock

import pytest

litellm = pytest.importorskip(
    "litellm.integrations.custom_logger",
    reason="LiteLLM not installed; install spendguard-sdk[litellm]",
)

from spendguard.errors import (  # noqa: E402
    SidecarUnavailable,
    SpendGuardConfigError,
)
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


_BINDING = BudgetBinding(
    budget_id="b1",
    window_instance_id="w1",
    unit=SimpleNamespace(unit_id="u1", token_kind="output_token"),
    pricing=_FakePricing(),
)


def _claim(*, budget_id: str = "b1", window: str = "w1", amount: str = "100",
           unit_id: str = "u1"):
    return SimpleNamespace(
        budget_id=budget_id, window_instance_id=window, amount_atomic=amount,
        unit=SimpleNamespace(unit_id=unit_id),
    )


def _client(
    *,
    decision: str = "CONTINUE",
    reservation_ids: tuple = ("res-1",),
):
    cli = MagicMock()
    cli.tenant_id = "tenant-1"
    cli.session_id = "session-1"
    outcome = SimpleNamespace(
        decision=decision,
        decision_id="dec-1",
        reservation_ids=reservation_ids,
        audit_decision_event_id="audit-1",
    )
    cli.request_decision = AsyncMock(return_value=outcome)
    cli.emit_llm_call_post = AsyncMock(return_value=None)
    return cli


def _cb(client, *, estimator_claim=None):
    return SpendGuardLiteLLMCallback(
        client=client,
        budget_resolver=lambda ctx: _BINDING,
        claim_estimator=lambda ctx: [estimator_claim or _claim()],
        claim_reconciler=lambda ctx, resp: [_claim()],
    )


def _data(call_id: str = "c-1"):
    return {"litellm_call_id": call_id, "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": "hi"}]}


@pytest.mark.asyncio
async def test_degrade_outcome_raises_sidecar_unavailable():
    """Slice 2 R1 P0.1: DEGRADE under fail-closed → SidecarUnavailable
    (DESIGN §5 ledger-down row). LiteLLM diverges from agt.py."""
    cli = _client(decision="DEGRADE")
    cb = _cb(cli)
    with pytest.raises(SidecarUnavailable, match="DEGRADE"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")


@pytest.mark.asyncio
async def test_degrade_outcome_allowed_under_fail_open(monkeypatch, caplog):
    """Slice 2 R1 P0.1: DEGRADE + FAIL_OPEN=1 → log WARNING + return
    data. Operator-explicit dev-only opt-out."""
    import logging as _logging
    monkeypatch.setenv("SPENDGUARD_LITELLM_FAIL_OPEN", "1")
    cli = _client(decision="DEGRADE")
    cb = _cb(cli)
    data = _data()
    with caplog.at_level(_logging.WARNING, logger="spendguard.integrations.litellm"):
        result = await cb.async_pre_call_hook(None, None, data, "acompletion")
    assert result is data
    assert any("DEGRADE" in r.message for r in caplog.records)


@pytest.mark.asyncio
async def test_multi_reservation_outcome_releases_then_raises():
    """Slice 2 R1 P1.2: multi-reservation outcome → best-effort release
    each reservation BEFORE raising; TTL is the durable backstop."""
    cli = _client(reservation_ids=("r-a", "r-b", "r-c"))
    cb = _cb(cli)
    with pytest.raises(SpendGuardConfigError, match=r"3 reservations"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")
    # Each reservation should have been released best-effort.
    assert cli.emit_llm_call_post.call_count == 3
    released = [
        c.kwargs["reservation_id"] for c in cli.emit_llm_call_post.call_args_list
    ]
    assert sorted(released) == ["r-a", "r-b", "r-c"]
    for c in cli.emit_llm_call_post.call_args_list:
        assert c.kwargs["outcome"] == "FAILURE"
        assert c.kwargs["provider_reported_amount_atomic"] == "0"


@pytest.mark.asyncio
async def test_multi_reservation_release_errors_are_swallowed():
    """If best-effort release fails, the original
    SpendGuardConfigError still surfaces (TTL is the backstop)."""
    cli = _client(reservation_ids=("r-x", "r-y"))
    cli.emit_llm_call_post = AsyncMock(side_effect=RuntimeError("net err"))
    cb = _cb(cli)
    with pytest.raises(SpendGuardConfigError, match=r"2 reservations"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")
    # All releases attempted despite errors.
    assert cli.emit_llm_call_post.call_count == 2


@pytest.mark.asyncio
async def test_estimator_claim_budget_mismatch_with_binding():
    """Slice 2 R1 P1.1: claim.budget_id mismatch with binding →
    SpendGuardConfigError BEFORE the wire. Audit context can't say
    budget X while we charge budget Y."""
    cli = _client()
    cb = _cb(cli, estimator_claim=_claim(budget_id="OTHER_BUDGET"))
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")
    cli.request_decision.assert_not_called()  # never reaches sidecar


@pytest.mark.asyncio
async def test_estimator_claim_window_mismatch_with_binding():
    """Slice 2 R1 P1.1: claim.window_instance_id mismatch with binding
    → SpendGuardConfigError BEFORE the wire."""
    cli = _client()
    cb = _cb(cli, estimator_claim=_claim(window="OTHER_WINDOW"))
    with pytest.raises(SpendGuardConfigError, match="window_instance_id"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")
    cli.request_decision.assert_not_called()


@pytest.mark.asyncio
async def test_estimator_claim_missing_attrs_rejected():
    """Slice 2 R3 P2.1 regression: claim WITHOUT budget_id/window_instance_id
    attrs (e.g. amount-only SimpleNamespace) → SpendGuardConfigError
    because attrs normalize to "" and binding.budget_id is non-empty."""
    cli = _client()
    cb = _cb(cli, estimator_claim=SimpleNamespace(amount_atomic="100"))
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")
    cli.request_decision.assert_not_called()


@pytest.mark.asyncio
async def test_estimator_claim_missing_unit_rejected():
    """Slice 3 R3 P1: claim with budget+window but NO unit attr is
    rejected. Without this guard the helper would silently pass and
    amount would be committed under wrong unit semantics."""
    cli = _client()
    no_unit_claim = SimpleNamespace(
        amount_atomic="100", budget_id="b1", window_instance_id="w1",
    )  # no unit attr
    cb = _cb(cli, estimator_claim=no_unit_claim)
    with pytest.raises(SpendGuardConfigError, match="unit.unit_id"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")
    cli.request_decision.assert_not_called()


@pytest.mark.asyncio
async def test_estimator_claim_empty_unit_id_rejected():
    cli = _client()
    cb = _cb(cli, estimator_claim=_claim(unit_id=""))
    with pytest.raises(SpendGuardConfigError, match="unit.unit_id"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")
    cli.request_decision.assert_not_called()


@pytest.mark.asyncio
async def test_estimator_claim_empty_budget_id_rejected():
    cli = _client()
    cb = _cb(cli, estimator_claim=_claim(budget_id=""))
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")
    cli.request_decision.assert_not_called()  # R4 P2: pre-wire reject


@pytest.mark.asyncio
async def test_estimator_claim_empty_window_id_rejected():
    cli = _client()
    cb = _cb(cli, estimator_claim=_claim(window=""))
    with pytest.raises(SpendGuardConfigError, match="window_instance_id"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")
    cli.request_decision.assert_not_called()  # R4 P2: pre-wire reject


@pytest.mark.asyncio
async def test_empty_binding_budget_id_rejected():
    """Slice 2 R3 P1 regression: BudgetBinding with empty budget_id →
    SpendGuardConfigError BEFORE the wire (defensive: even matching
    empty claim would silently pass equality check)."""
    cli = _client()
    cb = SpendGuardLiteLLMCallback(
        client=cli,
        budget_resolver=lambda ctx: BudgetBinding(
            budget_id="",  # invalid
            window_instance_id="w1",
            unit=SimpleNamespace(unit_id="u1"),
            pricing=_FakePricing(),
        ),
        claim_estimator=lambda ctx: [_claim(budget_id="")],
        claim_reconciler=lambda ctx, resp: [],
    )
    with pytest.raises(SpendGuardConfigError, match="budget_id is empty"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")
    cli.request_decision.assert_not_called()  # R4 P2: pre-wire reject


@pytest.mark.asyncio
async def test_empty_binding_window_id_rejected():
    """Slice 2 R3 P1: BudgetBinding with empty window_instance_id rejected."""
    cli = _client()
    cb = SpendGuardLiteLLMCallback(
        client=cli,
        budget_resolver=lambda ctx: BudgetBinding(
            budget_id="b1",
            window_instance_id="",  # invalid
            unit=SimpleNamespace(unit_id="u1"),
            pricing=_FakePricing(),
        ),
        claim_estimator=lambda ctx: [_claim(window="")],
        claim_reconciler=lambda ctx, resp: [],
    )
    with pytest.raises(SpendGuardConfigError, match="window_instance_id is empty"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")
    cli.request_decision.assert_not_called()  # R4 P2: pre-wire reject


@pytest.mark.asyncio
async def test_ensure_client_deadline_bounds_handshake_via_remaining_time(
    monkeypatch,
):
    """Slice 2 R3 P2.2 regression: _ensure_client MUST pass
    timeout=min(attempt_timeout, remaining) to wait_for. Patch
    asyncio.wait_for to record timeouts; assert the LAST call has
    timeout ≤ remaining time, not the full attempt timeout."""
    from spendguard.integrations.litellm import _LoopBoundCallback

    monkeypatch.setattr(_LoopBoundCallback, "_ENSURE_CLIENT_DEADLINE_S", 0.3)
    monkeypatch.setattr(
        _LoopBoundCallback, "_ENSURE_CLIENT_ATTEMPT_TIMEOUT_S", 1.0,
    )

    recorded_timeouts: list[float] = []
    original_wait_for = asyncio.wait_for

    async def _recording_wait_for(awaitable, *, timeout):
        recorded_timeouts.append(timeout)
        # Cancel the awaitable to avoid coroutine warnings.
        try:
            return await original_wait_for(awaitable, timeout=min(timeout, 0.01))
        except (asyncio.TimeoutError, Exception):
            raise

    import asyncio as _asyncio
    monkeypatch.setattr(_asyncio, "wait_for", _recording_wait_for)

    cb = _LoopBoundCallback(
        socket_path="/tmp/nonexistent-spendguard-r3-test",  # noqa: S108
        tenant_id="t1",
        budget_resolver=lambda ctx: None,
        claim_estimator=lambda ctx: [],
        claim_reconciler=lambda ctx, resp: [],
    )
    with pytest.raises(SidecarUnavailable):
        await cb._ensure_client()
    # Each recorded timeout MUST be ≤ ATTEMPT_TIMEOUT (1.0) AND
    # ≤ DEADLINE (0.3); the bounding by min(attempt, remaining)
    # means most are well under 1.0.
    assert recorded_timeouts, "expected at least one wait_for call"
    for t in recorded_timeouts:
        assert t <= 0.3 + 0.01, (
            f"timeout {t} > deadline; recompute-remaining is not enforced"
        )


@pytest.mark.asyncio
async def test_loop_bound_callback_ensure_client_respects_deadline(monkeypatch):
    """Slice 2 R1 P0.2: _ensure_client honors absolute deadline.
    Even if individual handshake attempts hang, the total time is
    bounded by _ENSURE_CLIENT_DEADLINE_S."""
    from spendguard.integrations.litellm import _LoopBoundCallback

    # Force a tiny deadline so the test runs fast.
    monkeypatch.setattr(_LoopBoundCallback, "_ENSURE_CLIENT_DEADLINE_S", 0.2)
    monkeypatch.setattr(
        _LoopBoundCallback, "_ENSURE_CLIENT_ATTEMPT_TIMEOUT_S", 0.05,
    )

    cb = _LoopBoundCallback(
        socket_path="/tmp/nonexistent-spendguard-deadline-test",  # noqa: S108
        tenant_id="t1",
        budget_resolver=lambda ctx: None,
        claim_estimator=lambda ctx: [],
        claim_reconciler=lambda ctx, resp: [],
    )
    import time
    start = time.monotonic()
    with pytest.raises(SidecarUnavailable, match="deadline"):
        await cb._ensure_client()
    elapsed = time.monotonic() - start
    # Should NOT exceed the deadline by much (allow 0.3s slack).
    assert elapsed < 0.5, f"_ensure_client exceeded deadline: {elapsed:.2f}s"
