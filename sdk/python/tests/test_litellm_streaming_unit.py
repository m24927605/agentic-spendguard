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
        # R1 P1.2: stash now uses snapshot key (immutable primitives)
        "estimator_claims_snapshot": [_claim(estimator_amount)],
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
    real total → emit_llm_call_post with reconciled amount. R1 P2.1
    fix: reconciler now derives amount from resp.usage so the test
    actually validates that pathway."""
    cli = _client()
    cb = _cb_with_stream_stash(
        cli,
        # P2.1: reconciler derives amount FROM response_obj.usage.
        reconciler=lambda ctx, resp: [_claim(
            str(resp.usage.completion_tokens * 2)  # arbitrary fn of usage
        )],
    )
    await cb.async_log_success_event(_kwargs(), _stream_response(completion_tokens=150), 0, 1)
    cli.emit_llm_call_post.assert_called_once()
    kw = cli.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] == "300"  # 150 * 2 — derived from usage
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
    stashed estimator_claims + WARNING log. NOT F7 acceptance.
    R1 P2.1 fix: assert reconciler was NOT called (we used fallback)."""
    import logging as _logging
    reconciler_calls: list = []

    def _reconciler_should_not_be_called(ctx, resp):
        reconciler_calls.append(("called",))
        return [_claim("NEVER_USED")]

    cli = _client()
    cb = _cb_with_stream_stash(
        cli, estimator_amount="800",
        reconciler=_reconciler_should_not_be_called,
    )
    with caplog.at_level(_logging.WARNING, logger="spendguard.integrations.litellm"):
        await cb.async_log_success_event(
            _kwargs(), _stream_response_no_usage(), 0, 1,
        )
    cli.emit_llm_call_post.assert_called_once()
    kw = cli.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] == "800"  # estimator fallback
    assert reconciler_calls == [], "reconciler should NOT fire on missing usage"
    assert any("no .usage" in r.message for r in caplog.records)


@pytest.mark.asyncio
async def test_streaming_snapshot_captures_pre_await_value(monkeypatch):
    """Slice 4 R2 P1: snapshot taken BEFORE request_decision await,
    so if a shared mutable claim mutates DURING the await window
    (operator's claim object reused by another concurrent caller),
    the fallback commits the value the sidecar reserved against, not
    the post-mutation value."""
    from spendguard.integrations.litellm import (
        SpendGuardLiteLLMCallback,
    )
    mutable_claim = _claim("500")  # initial estimate
    cli = _client()

    # Make request_decision mutate the shared claim DURING the await
    # window (simulating a concurrent task touching the same object).
    async def _decision_mutates_claim(**_):
        mutable_claim.amount_atomic = "999999999"  # post-snapshot mutation
        return SimpleNamespace(
            decision="CONTINUE",
            decision_id="dec-1",
            reservation_ids=("res-1",),
            audit_decision_event_id="audit-1",
        )

    cli.request_decision = AsyncMock(side_effect=_decision_mutates_claim)
    cb = SpendGuardLiteLLMCallback(
        client=cli,
        budget_resolver=lambda ctx: _BINDING,
        claim_estimator=lambda ctx: [mutable_claim],
        claim_reconciler=lambda ctx, resp: [_claim("NEVER_USED")],
    )

    # Run the pre-call hook. It snapshots BEFORE await, then sidecar
    # mutates the original. Then trigger streaming fallback path.
    await cb.async_pre_call_hook(
        SimpleNamespace(team_id="t1"),  # user_api_key_dict
        None,
        {"litellm_call_id": "mut-during-await", "model": "gpt", "stream": True,
         "messages": [{"role": "user", "content": "hi"}]},
        "acompletion",
    )
    # Fallback path (no usage on response).
    await cb.async_log_success_event(
        _kwargs("mut-during-await"),
        _stream_response_no_usage(),
        0, 1,
    )
    kw = cli.emit_llm_call_post.call_args.kwargs
    # Snapshot captured pre-await value "500", not the post-mutation
    # "999999999". Without the R2 P1 fix, this assertion would fail.
    assert kw["estimated_amount_atomic"] == "500"


@pytest.mark.asyncio
async def test_streaming_fallback_uses_stashed_snapshot_not_mutated_claim():
    """Slice 4 R1 P1.2 fix: stash should freeze estimator amount, so
    mutating the original claim object after pre-call does NOT change
    what fallback commits."""
    # Build a stash by directly setting estimator_claims_snapshot
    # (mimics post-pre-call state).
    cli = _client()
    original_claim = _claim("500")
    cb = SpendGuardLiteLLMCallback(
        client=cli,
        budget_resolver=lambda ctx: _BINDING,
        claim_estimator=lambda ctx: [original_claim],
        claim_reconciler=lambda ctx, resp: [_claim("NEVER")],
    )
    # Simulate stash population (mimics Slice 2 snapshot logic).
    from types import SimpleNamespace as _SN
    snapshot = _SN(
        amount_atomic="500", budget_id="b1", window_instance_id="w1",
        unit=_SN(unit_id="u1"),
    )
    cb._stash["sim"] = {
        "decision_id": "d", "reservation_ids": ("r1",),
        "llm_call_id": "l", "run_id": "rr", "step_id": "ss",
        "binding": _BINDING,
        "audit_decision_event_id": "a",
        "decision_context": {}, "stream": True,
        "estimator_claims_snapshot": [snapshot],
        "mode": "proxy",
    }
    # MUTATE the original claim post-stash to a wildly different amount.
    original_claim.amount_atomic = "999999999"
    # Fallback path (no usage on response).
    await cb.async_log_success_event(
        _kwargs("sim"), _stream_response_no_usage(), 0, 1,
    )
    kw = cli.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] == "500"  # snapshot, not mutation


@pytest.mark.asyncio
async def test_streaming_commit_unavailable_wraps_as_sidecar_unavailable():
    """R4 P0.6 / NF5: actual SidecarUnavailable at commit boundary
    surfaces as SidecarUnavailable (typed exception contract). R1
    P1.1 narrowing: ONLY SidecarUnavailable (transport) is wrapped;
    semantic errors propagate as-is."""
    cli = _client(emit_side_effect=SidecarUnavailable("UDS gone"))
    cb = _cb_with_stream_stash(cli, call_id="boundary-fail")
    with pytest.raises(SidecarUnavailable, match="commit boundary"):
        await cb.async_log_success_event(
            _kwargs("boundary-fail"), _stream_response(), 0, 1,
        )
    assert "boundary-fail" in cb._stash


@pytest.mark.asyncio
async def test_streaming_commit_semantic_error_propagates_as_is():
    """R1 P1.1 fix: non-availability SpendGuardError (e.g. invariant
    rejection, sidecar ack-rejected) MUST NOT be wrapped as
    SidecarUnavailable — that would mask config/invariant bugs as
    transient outages."""
    semantic_err = SpendGuardError("ack rejected: state conflict")
    cli = _client(emit_side_effect=semantic_err)
    cb = _cb_with_stream_stash(cli, call_id="semantic-err")
    with pytest.raises(SpendGuardError, match="ack rejected") as exc_info:
        await cb.async_log_success_event(
            _kwargs("semantic-err"), _stream_response(), 0, 1,
        )
    assert not isinstance(exc_info.value, SidecarUnavailable)
    assert "semantic-err" in cb._stash  # not popped — retry/TTL backstop


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
    # R1 P1.1 narrowing: semantic SpendGuardError (not SidecarUnavailable)
    # logs "semantic error" path under fail-open.
    assert any("streaming commit" in r.message and (
        "unavailable" in r.message or "semantic error" in r.message
    ) for r in caplog.records)


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
