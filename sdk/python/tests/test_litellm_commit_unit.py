# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S106
"""Slice 3 — Tier 1 unit tests per TEST_PLAN.md §2.3 for
async_log_success_event non-streaming branch.
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

from spendguard.errors import (  # noqa: E402
    SpendGuardConfigError,
    SpendGuardError,
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
    unit=SimpleNamespace(unit_id="u1"),
    pricing=_FakePricing(),
)


def _claim(amount: str = "92", budget_id: str = "b1", window: str = "w1"):
    return SimpleNamespace(
        amount_atomic=amount, budget_id=budget_id, window_instance_id=window,
    )


def _client(*, emit_side_effect=None):
    cli = MagicMock()
    cli.tenant_id = "tenant-1"
    cli.session_id = "session-1"
    cli.request_decision = AsyncMock()
    cli.emit_llm_call_post = AsyncMock(
        return_value=None, side_effect=emit_side_effect,
    )
    return cli


def _cb_with_stash(
    cli,
    *,
    call_id: str = "litellm-1",
    stream: bool = False,
    reservation_ids: tuple = ("res-1",),
    reconciler=None,
):
    cb = SpendGuardLiteLLMCallback(
        client=cli,
        budget_resolver=lambda ctx: _BINDING,
        claim_estimator=lambda ctx: [_claim("50")],
        claim_reconciler=reconciler or (lambda ctx, resp: [_claim("92")]),
    )
    cb._stash[call_id] = {
        "decision_id": "dec-1",
        "reservation_ids": reservation_ids,
        "llm_call_id": "llm-1",
        "run_id": "run-1", "step_id": "step-1",
        "binding": _BINDING,
        "audit_decision_event_id": "audit-1",
        "decision_context": {"integration": "litellm", "mode": "proxy"},
        "stream": stream,
        "estimator_claims": [_claim("50")],
        "mode": "proxy",
    }
    return cb


def _kwargs(call_id: str = "litellm-1"):
    return {"litellm_call_id": call_id, "model": "gpt-4o-mini"}


def _response(*, completion_tokens: int = 92, response_id: str = "chatcmpl-x"):
    return SimpleNamespace(
        id=response_id,
        usage=SimpleNamespace(prompt_tokens=10, completion_tokens=completion_tokens),
    )


@pytest.mark.asyncio
async def test_success_event_calls_emit_with_stashed_ids():
    """Happy path: success event commits via emit_llm_call_post with
    decision_id + reservation_id from stash + reconciled amount."""
    cli = _client()
    cb = _cb_with_stash(cli)
    await cb.async_log_success_event(_kwargs(), _response(), 0, 1)
    cli.emit_llm_call_post.assert_called_once()
    kw = cli.emit_llm_call_post.call_args.kwargs
    assert kw["decision_id"] == "dec-1"
    assert kw["reservation_id"] == "res-1"
    # Slice 3 R1 P0 fix: emit via estimated_amount_atomic (v1 path)
    # not provider_reported_amount_atomic (deferred Phase 2B Step 8).
    assert kw["estimated_amount_atomic"] == "92"
    assert kw["provider_reported_amount_atomic"] == ""
    assert kw["outcome"] == "SUCCESS"
    assert kw["provider_event_id"] == "chatcmpl-x"


@pytest.mark.asyncio
async def test_success_event_pops_stash_after_ack():
    """Stash entry popped after sidecar ACK so memory is bounded."""
    cli = _client()
    cb = _cb_with_stash(cli, call_id="pop-me")
    assert "pop-me" in cb._stash
    await cb.async_log_success_event(_kwargs("pop-me"), _response(), 0, 1)
    assert "pop-me" not in cb._stash


@pytest.mark.asyncio
async def test_success_event_silently_noop_if_no_stash():
    """No stash → silent no-op (pre-call hook didn't fire)."""
    cli = _client()
    cb = SpendGuardLiteLLMCallback(
        client=cli,
        budget_resolver=lambda ctx: _BINDING,
        claim_estimator=lambda ctx: [_claim()],
        claim_reconciler=lambda ctx, resp: [_claim()],
    )
    await cb.async_log_success_event(_kwargs("missing"), _response(), 0, 1)
    cli.emit_llm_call_post.assert_not_called()


@pytest.mark.asyncio
async def test_success_event_streaming_branch_routes_to_slice4():
    """If stash['stream'] is True → NotImplementedError (Slice 4)."""
    cli = _client()
    cb = _cb_with_stash(cli, stream=True)
    with pytest.raises(NotImplementedError, match="Slice 4"):
        await cb.async_log_success_event(_kwargs(), _response(), 0, 1)
    cli.emit_llm_call_post.assert_not_called()


@pytest.mark.asyncio
async def test_success_event_rejects_multi_claim_reconciler():
    """v1 contract: reconciler returns exactly 1 claim → 0 / ≥2 = SpendGuardConfigError."""
    cli = _client()
    cb = _cb_with_stash(cli, reconciler=lambda ctx, resp: [_claim(), _claim()])
    with pytest.raises(SpendGuardConfigError, match="2 claims"):
        await cb.async_log_success_event(_kwargs(), _response(), 0, 1)
    cli.emit_llm_call_post.assert_not_called()


@pytest.mark.asyncio
async def test_success_event_rejects_zero_claim_reconciler():
    cli = _client()
    cb = _cb_with_stash(cli, reconciler=lambda ctx, resp: [])
    with pytest.raises(SpendGuardConfigError, match="0 claims"):
        await cb.async_log_success_event(_kwargs(), _response(), 0, 1)


@pytest.mark.asyncio
async def test_success_event_keeps_stash_on_sidecar_error_when_fail_closed():
    """Sidecar commit fails + fail_closed=True → raise; stash KEPT
    so retry can find it; sidecar idempotency dedupes."""
    err = SpendGuardError("emit RPC boom")
    cli = _client(emit_side_effect=err)
    cb = _cb_with_stash(cli, call_id="retry-me")
    with pytest.raises(SpendGuardError):
        await cb.async_log_success_event(_kwargs("retry-me"), _response(), 0, 1)
    assert "retry-me" in cb._stash  # not popped


@pytest.mark.asyncio
async def test_success_event_fail_open_swallows_error_keeps_stash(monkeypatch, caplog):
    """fail_open=1: log WARNING, return silently, KEEP stash (TTL sweep durable)."""
    import logging as _logging
    monkeypatch.setenv("SPENDGUARD_LITELLM_FAIL_OPEN", "1")
    err = SpendGuardError("emit RPC boom")
    cli = _client(emit_side_effect=err)
    cb = _cb_with_stash(cli, call_id="fail-open-1")
    with caplog.at_level(_logging.WARNING, logger="spendguard.integrations.litellm"):
        await cb.async_log_success_event(_kwargs("fail-open-1"), _response(), 0, 1)
    assert "fail-open-1" in cb._stash  # not popped under fail-open either
    assert any("commit failed" in r.message for r in caplog.records)


@pytest.mark.asyncio
async def test_success_event_rejects_reconciler_budget_mismatch():
    """Slice 3 R1 P1 fix: reconciler budget_id ≠ stash binding →
    SpendGuardConfigError (mirror of pre-call check at commit time)."""
    cli = _client()
    cb = _cb_with_stash(
        cli, reconciler=lambda ctx, resp: [_claim(budget_id="OTHER")],
    )
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        await cb.async_log_success_event(_kwargs(), _response(), 0, 1)
    cli.emit_llm_call_post.assert_not_called()


@pytest.mark.asyncio
async def test_success_event_rejects_reconciler_window_mismatch():
    cli = _client()
    cb = _cb_with_stash(
        cli, reconciler=lambda ctx, resp: [_claim(window="OTHER")],
    )
    with pytest.raises(SpendGuardConfigError, match="window_instance_id"):
        await cb.async_log_success_event(_kwargs(), _response(), 0, 1)
    cli.emit_llm_call_post.assert_not_called()


@pytest.mark.asyncio
async def test_success_event_client_none_with_stash_raises():
    """Slice 3 R1 P2 fix: stash present + client None → fail-closed
    (not silent no-op). Should be impossible after pre-call hook
    succeeded, but defensive contract."""
    cli = _client()
    cb = _cb_with_stash(cli)
    cb._client = None  # simulate corruption
    with pytest.raises(SpendGuardConfigError, match="self._client is None"):
        await cb.async_log_success_event(_kwargs(), _response(), 0, 1)


@pytest.mark.asyncio
async def test_success_event_rejects_multi_reservation_stash():
    """Defensive: if stash somehow has >1 reservation_ids (Slice 2
    pre-rejects, but spec drift defense) → SpendGuardConfigError."""
    cli = _client()
    cb = _cb_with_stash(cli, reservation_ids=("r1", "r2"))
    with pytest.raises(SpendGuardConfigError, match="2 reservation"):
        await cb.async_log_success_event(_kwargs(), _response(), 0, 1)
