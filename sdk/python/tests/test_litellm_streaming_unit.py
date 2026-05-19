# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S106
"""Slice 4 — Tier 1 unit tests for streaming reconciler per
TEST_PLAN.md §2.4. Streaming branch fires when stash['stream']=True;
reconciles end-of-stream usage; falls back to estimator if usage
missing; wraps commit failures as SidecarUnavailable.
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
    SidecarUnavailable,
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


def _claim(amount: str = "92", budget_id: str = "b1", window: str = "w1",
           unit_id: str = "u1"):
    return SimpleNamespace(
        amount_atomic=amount, budget_id=budget_id, window_instance_id=window,
        unit=SimpleNamespace(unit_id=unit_id),
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


def _cb_with_stream_stash(cli, *, call_id: str = "litellm-1",
                          reconciler=None,
                          estimator_amount: str = "1000"):
    cb = SpendGuardLiteLLMCallback(
        client=cli,
        budget_resolver=lambda ctx: _BINDING,
        claim_estimator=lambda ctx: [_claim(estimator_amount)],
        claim_reconciler=reconciler or (lambda ctx, resp: [_claim("200")]),
    )
    cb._stash[call_id] = {
        "decision_id": "dec-1",
        "reservation_ids": ("res-1",),
        "llm_call_id": "llm-1",
        "run_id": "run-1", "step_id": "step-1",
        "binding": _BINDING,
        "audit_decision_event_id": "audit-1",
        "decision_context": {"integration": "litellm", "mode": "proxy"},
        "stream": True,  # Slice 4 streaming branch
        "estimator_claims": [_claim(estimator_amount)],
        "mode": "proxy",
    }
    return cb


def _kwargs(call_id: str = "litellm-1"):
    return {"litellm_call_id": call_id, "model": "gpt-4o-mini"}


def _stream_response(*, completion_tokens: int = 200, response_id: str = "stream-1"):
    return SimpleNamespace(
        id=response_id,
        usage=SimpleNamespace(prompt_tokens=10, completion_tokens=completion_tokens),
    )


def _stream_response_no_usage(*, response_id: str = "stream-no-usage"):
    """End-of-stream response with no .usage frame (degraded path)."""
    return SimpleNamespace(id=response_id, usage=None)


@pytest.mark.asyncio
async def test_streaming_branch_commits_real_usage_when_present():
    """F7 happy path: response.usage present → reconciler computes
    real total → emit_llm_call_post with reconciled amount."""
    cli = _client()
    cb = _cb_with_stream_stash(cli)
    await cb.async_log_success_event(_kwargs(), _stream_response(), 0, 1)
    cli.emit_llm_call_post.assert_called_once()
    kw = cli.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] == "200"  # reconciled
    assert kw["outcome"] == "SUCCESS"


@pytest.mark.asyncio
async def test_streaming_commit_amount_not_equal_to_estimator():
    """F7 acceptance assertion: commit amount ≠ estimator worst-case
    (proves reconciler ran on actual usage)."""
    cli = _client()
    cb = _cb_with_stream_stash(
        cli, estimator_amount="1000",
        reconciler=lambda ctx, resp: [_claim("200")],
    )
    await cb.async_log_success_event(_kwargs(), _stream_response(), 0, 1)
    kw = cli.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] == "200"
    assert kw["estimated_amount_atomic"] != "1000"  # not the estimator


@pytest.mark.asyncio
async def test_streaming_fallback_to_estimator_when_usage_missing(monkeypatch, caplog):
    """R3 P0.7: degraded path — response has no .usage → fall back to
    stashed estimator_claims + WARNING log. NOT F7 acceptance."""
    import logging as _logging
    cli = _client()
    cb = _cb_with_stream_stash(cli, estimator_amount="800")
    with caplog.at_level(_logging.WARNING, logger="spendguard.integrations.litellm"):
        await cb.async_log_success_event(
            _kwargs(), _stream_response_no_usage(), 0, 1,
        )
    cli.emit_llm_call_post.assert_called_once()
    kw = cli.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] == "800"  # estimator fallback
    assert any("no .usage" in r.message for r in caplog.records)


@pytest.mark.asyncio
async def test_streaming_commit_boundary_error_wraps_as_sidecar_unavailable():
    """R4 P0.6 / NF5: commit-boundary sidecar failure surfaces as
    SidecarUnavailable (typed exception contract)."""
    err = SpendGuardError("commit RPC boom")
    cli = _client(emit_side_effect=err)
    cb = _cb_with_stream_stash(cli, call_id="boundary-fail")
    with pytest.raises(SidecarUnavailable, match="commit boundary"):
        await cb.async_log_success_event(
            _kwargs("boundary-fail"), _stream_response(), 0, 1,
        )
    # Stash kept for retry/TTL.
    assert "boundary-fail" in cb._stash


@pytest.mark.asyncio
async def test_streaming_commit_fail_open_keeps_stash(monkeypatch, caplog):
    """fail_open=1: sidecar error logged + return; stash kept."""
    import logging as _logging
    monkeypatch.setenv("SPENDGUARD_LITELLM_FAIL_OPEN", "1")
    err = SpendGuardError("commit RPC boom")
    cli = _client(emit_side_effect=err)
    cb = _cb_with_stream_stash(cli, call_id="stream-fo")
    with caplog.at_level(_logging.WARNING, logger="spendguard.integrations.litellm"):
        await cb.async_log_success_event(
            _kwargs("stream-fo"), _stream_response(), 0, 1,
        )
    assert "stream-fo" in cb._stash
    assert any("streaming commit failed" in r.message for r in caplog.records)


@pytest.mark.asyncio
async def test_streaming_rejects_multi_claim_reconciler():
    cli = _client()
    cb = _cb_with_stream_stash(
        cli, reconciler=lambda ctx, resp: [_claim(), _claim()],
    )
    with pytest.raises(SpendGuardConfigError, match="2 claims"):
        await cb.async_log_success_event(_kwargs(), _stream_response(), 0, 1)


@pytest.mark.asyncio
async def test_streaming_rejects_reconciler_binding_mismatch():
    """Same binding validation as non-streaming: reconciler claim
    budget/window/unit must match binding."""
    cli = _client()
    cb = _cb_with_stream_stash(
        cli,
        reconciler=lambda ctx, resp: [_claim(budget_id="OTHER")],
    )
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        await cb.async_log_success_event(_kwargs(), _stream_response(), 0, 1)


@pytest.mark.asyncio
async def test_streaming_pops_stash_after_ack():
    cli = _client()
    cb = _cb_with_stream_stash(cli, call_id="stream-pop")
    await cb.async_log_success_event(_kwargs("stream-pop"), _stream_response(), 0, 1)
    assert "stream-pop" not in cb._stash


@pytest.mark.asyncio
async def test_streaming_client_none_raises_config_error():
    """Stash present + client=None defensive check (same as non-stream)."""
    cli = _client()
    cb = _cb_with_stream_stash(cli)
    cb._client = None
    with pytest.raises(SpendGuardConfigError, match="self._client is None"):
        await cb.async_log_success_event(_kwargs(), _stream_response(), 0, 1)
