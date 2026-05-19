# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S106
"""Slice 5 — Tier 1 unit tests for async_log_failure_event per
TEST_PLAN.md §2.5. Provider exception → outcome=FAILURE; CancelledError
→ CANCELLED; release RPC failure → swallowed (keep stash); multi-attempt
retry → multi reserve/release pairs (each with distinct call_id).
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


def _claim(amount: str = "50"):
    return SimpleNamespace(
        amount_atomic=amount, budget_id="b1", window_instance_id="w1",
        unit=SimpleNamespace(unit_id="u1"),
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
    cli, *, call_id: str = "litellm-1",
    reservation_ids: tuple = ("res-1",),
):
    cb = SpendGuardLiteLLMCallback(
        client=cli,
        budget_resolver=lambda ctx: _BINDING,
        claim_estimator=lambda ctx: [_claim("50")],
        claim_reconciler=lambda ctx, resp: [_claim("92")],
    )
    cb._stash[call_id] = {
        "decision_id": "dec-1",
        "reservation_ids": reservation_ids,
        "llm_call_id": "llm-1",
        "run_id": "run-1", "step_id": "step-1",
        "binding": _BINDING,
        "audit_decision_event_id": "audit-1",
        "decision_context": {"integration": "litellm", "mode": "proxy"},
        "stream": False,
        "estimator_claims_snapshot": [_claim("50")],
        "mode": "proxy",
    }
    return cb


def _kwargs(call_id: str = "litellm-1", *, exception: object = None):
    k: dict[str, object] = {"litellm_call_id": call_id, "model": "gpt-4o-mini"}
    if exception is not None:
        k["exception"] = exception
    return k


def _response(*, response_id: str = "err-1"):
    return SimpleNamespace(id=response_id, usage=None)


@pytest.mark.asyncio
async def test_failure_event_emits_failure_outcome_on_provider_error():
    """Generic Exception in kwargs → outcome=FAILURE; reservation
    released via emit_llm_call_post with stashed ids."""
    cli = _client()
    cb = _cb_with_stash(cli)
    err = Exception("upstream 500")
    await cb.async_log_failure_event(_kwargs(exception=err), _response(), 0, 1)
    cli.emit_llm_call_post.assert_called_once()
    kw = cli.emit_llm_call_post.call_args.kwargs
    assert kw["decision_id"] == "dec-1"
    assert kw["reservation_id"] == "res-1"
    assert kw["outcome"] == "FAILURE"
    assert kw["provider_reported_amount_atomic"] == "0"
    assert kw["estimated_amount_atomic"] == "0"


@pytest.mark.asyncio
async def test_failure_event_classifies_cancelled_from_exception_obj():
    """asyncio.CancelledError instance → outcome=CANCELLED."""
    cli = _client()
    cb = _cb_with_stash(cli)
    await cb.async_log_failure_event(
        _kwargs(exception=asyncio.CancelledError()), _response(), 0, 1,
    )
    kw = cli.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "CANCELLED"


@pytest.mark.asyncio
async def test_failure_event_classifies_cancelled_from_string():
    """Some LiteLLM versions pass exception as str → 'cancelled'
    substring still classifies as CANCELLED."""
    cli = _client()
    cb = _cb_with_stash(cli)
    await cb.async_log_failure_event(
        _kwargs(exception="Request was Cancelled by client"),
        _response(), 0, 1,
    )
    assert cli.emit_llm_call_post.call_args.kwargs["outcome"] == "CANCELLED"


@pytest.mark.asyncio
async def test_failure_event_silent_noop_when_no_stash():
    """No stash (pre-call hook never fired) → silent no-op; emit NOT
    called; no exception raised."""
    cli = _client()
    cb = SpendGuardLiteLLMCallback(
        client=cli,
        budget_resolver=lambda ctx: _BINDING,
        claim_estimator=lambda ctx: [_claim()],
        claim_reconciler=lambda ctx, resp: [_claim()],
    )
    await cb.async_log_failure_event(
        _kwargs("missing", exception=Exception("x")), _response(), 0, 1,
    )
    cli.emit_llm_call_post.assert_not_called()


@pytest.mark.asyncio
async def test_failure_event_swallows_release_rpc_error_and_keeps_stash(caplog):
    """Release RPC fails → swallowed (do NOT mask original LiteLLM
    exception); stash KEPT so a subsequent retry/sweep can see it."""
    import logging as _logging
    err = SpendGuardError("release RPC boom")
    cli = _client(emit_side_effect=err)
    cb = _cb_with_stash(cli, call_id="keep-me")
    with caplog.at_level(_logging.WARNING, logger="spendguard.integrations.litellm"):
        # Must NOT raise — swallow contract.
        await cb.async_log_failure_event(
            _kwargs("keep-me", exception=Exception("provider")),
            _response(), 0, 1,
        )
    assert "keep-me" in cb._stash
    assert any("release RPC failed" in r.message for r in caplog.records)


@pytest.mark.asyncio
async def test_failure_event_pops_stash_after_successful_release():
    """Successful release → stash popped (mirrors success-path
    contract; pop only on ACK)."""
    cli = _client()
    cb = _cb_with_stash(cli, call_id="release-me")
    await cb.async_log_failure_event(
        _kwargs("release-me", exception=Exception("x")), _response(), 0, 1,
    )
    assert "release-me" not in cb._stash


@pytest.mark.asyncio
async def test_failure_event_warns_on_multi_reservation_releases_first(caplog):
    """Defensive: stash with >1 reservation_ids (shouldn't happen,
    pre-call rejects) → WARN + release first only."""
    import logging as _logging
    cli = _client()
    cb = _cb_with_stash(cli, reservation_ids=("r1", "r2", "r3"))
    with caplog.at_level(_logging.WARNING, logger="spendguard.integrations.litellm"):
        await cb.async_log_failure_event(
            _kwargs(exception=Exception("x")), _response(), 0, 1,
        )
    assert cli.emit_llm_call_post.call_count == 1
    assert cli.emit_llm_call_post.call_args.kwargs["reservation_id"] == "r1"
    assert any("3 reservations" in r.message for r in caplog.records)


@pytest.mark.asyncio
async def test_failure_event_empty_reservations_just_pops_stash():
    """Empty reservation_ids (edge case) → emit NOT called; stash
    popped (nothing to release)."""
    cli = _client()
    cb = _cb_with_stash(cli, call_id="empty-r", reservation_ids=())
    await cb.async_log_failure_event(
        _kwargs("empty-r", exception=Exception("x")), _response(), 0, 1,
    )
    cli.emit_llm_call_post.assert_not_called()
    assert "empty-r" not in cb._stash


@pytest.mark.asyncio
async def test_failure_event_does_not_mask_original_via_release_failure(caplog):
    """Important: even if release RPC raises non-SpendGuardError
    (e.g. asyncio.CancelledError DURING release), we still don't
    propagate — TTL sweep is durable. But SpendGuardError-derived
    errors get the structured warning; anything else still bubbles."""
    # Non-SpendGuardError release exception (e.g. transient OS error)
    # SHOULD propagate per documented contract — only SpendGuardError
    # is swallowed (avoids masking real bugs).
    cli = _client(emit_side_effect=RuntimeError("os-level"))
    cb = _cb_with_stash(cli, call_id="bubble-up")
    with pytest.raises(RuntimeError):
        await cb.async_log_failure_event(
            _kwargs("bubble-up", exception=Exception("provider")),
            _response(), 0, 1,
        )


@pytest.mark.asyncio
async def test_failure_event_retry_storm_releases_each_distinct_call_id():
    """ADR-002: 3 retry attempts → 3 distinct litellm_call_ids → 3
    distinct stashes → 3 release calls. Simulates LiteLLM retry loop:
    each attempt fires pre-call (stashed below directly for test
    isolation) then failure event on provider error."""
    cli = _client()
    cb = SpendGuardLiteLLMCallback(
        client=cli,
        budget_resolver=lambda ctx: _BINDING,
        claim_estimator=lambda ctx: [_claim()],
        claim_reconciler=lambda ctx, resp: [_claim()],
    )
    for i, cid in enumerate(["att-1", "att-2", "att-3"]):
        cb._stash[cid] = {
            "decision_id": f"dec-{i}",
            "reservation_ids": (f"res-{i}",),
            "llm_call_id": f"llm-{i}",
            "run_id": "run-1", "step_id": "step-1",
            "binding": _BINDING,
            "audit_decision_event_id": f"audit-{i}",
            "decision_context": {"integration": "litellm", "mode": "proxy"},
            "stream": False,
            "estimator_claims_snapshot": [_claim()],
            "mode": "proxy",
        }
    for cid in ["att-1", "att-2", "att-3"]:
        await cb.async_log_failure_event(
            _kwargs(cid, exception=Exception("provider 500")),
            _response(), 0, 1,
        )
    assert cli.emit_llm_call_post.call_count == 3
    rids = [c.kwargs["reservation_id"] for c in cli.emit_llm_call_post.call_args_list]
    assert rids == ["res-0", "res-1", "res-2"]
    # All stashes popped after ACK.
    assert "att-1" not in cb._stash
    assert "att-2" not in cb._stash
    assert "att-3" not in cb._stash


@pytest.mark.asyncio
async def test_failure_event_client_none_logs_and_returns(caplog):
    """Stash present but client None (race / config drift) → log
    warning + return (TTL sweep). Mirrors success-branch defensive
    posture but does NOT raise — masking the LiteLLM exception is
    worse than the audit gap, and TTL sweep covers the release."""
    import logging as _logging
    cli = _client()
    cb = _cb_with_stash(cli, call_id="no-client")
    cb._client = None
    with caplog.at_level(_logging.WARNING, logger="spendguard.integrations.litellm"):
        await cb.async_log_failure_event(
            _kwargs("no-client", exception=Exception("x")),
            _response(), 0, 1,
        )
    assert any("no client" in r.message for r in caplog.records)
