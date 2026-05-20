# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S106
"""Slice A2 — unit tests for `SpendGuardDirectAcompletion` (Slice A1).

Covers ALLOW happy path, DENY raises, DEGRADE raises, provider raises
→ FAILURE release fires, commit raises → response still returned,
fail-open dev bypass, stream=True rejected.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock

import pytest

litellm_custom = pytest.importorskip(
    "litellm.integrations.custom_logger",
    reason="LiteLLM not installed; install spendguard-sdk[litellm]",
)

from spendguard.errors import (  # noqa: E402
    DecisionDenied,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardError,
)
from spendguard.integrations.litellm import (  # noqa: E402
    BudgetBinding,
    SpendGuardDirectAcompletion,
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


def _client():
    cli = MagicMock()
    cli.tenant_id = "tenant-1"
    cli.session_id = "session-1"
    cli.request_decision = AsyncMock(return_value=SimpleNamespace(
        decision="CONTINUE",
        decision_id="dec-1",
        reservation_ids=("res-1",),
        audit_decision_event_id="audit-1",
    ))
    cli.emit_llm_call_post = AsyncMock(return_value=None)
    return cli


def _wrapper(cli, *, estimator_amount: str = "50",
             reconciler_amount: str = "92"):
    return SpendGuardDirectAcompletion(
        client=cli,
        budget_resolver=lambda ctx: _BINDING,
        claim_estimator=lambda ctx: [_claim(estimator_amount)],
        claim_reconciler=lambda ctx, resp: [_claim(reconciler_amount)],
    )


def _patch_litellm_acompletion(monkeypatch, *, side_effect=None,
                                response_obj=None):
    """Replace litellm.acompletion with an AsyncMock. Default response
    is a SimpleNamespace with id='chatcmpl-x' and usage.completion_tokens=92."""
    import litellm
    if response_obj is None:
        response_obj = SimpleNamespace(
            id="chatcmpl-x",
            usage=SimpleNamespace(prompt_tokens=10, completion_tokens=92),
        )
    mock = AsyncMock(return_value=response_obj, side_effect=side_effect)
    monkeypatch.setattr(litellm, "acompletion", mock)
    return mock


@pytest.mark.asyncio
async def test_allow_happy_path_returns_response(monkeypatch):
    """ALLOW: pre-call reserves; litellm.acompletion called; post-call
    commits with reconciler amount; response returned."""
    cli = _client()
    cb = _wrapper(cli)
    acompletion_mock = _patch_litellm_acompletion(monkeypatch)
    resp = await cb(model="gpt-4o-mini",
                    messages=[{"role": "user", "content": "hi"}])
    assert resp.id == "chatcmpl-x"
    cli.request_decision.assert_called_once()
    cli.emit_llm_call_post.assert_called_once()
    kw = cli.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "SUCCESS"
    assert kw["estimated_amount_atomic"] == "92"
    assert kw["reservation_id"] == "res-1"
    acompletion_mock.assert_called_once()


@pytest.mark.asyncio
async def test_deny_raises_and_never_calls_litellm(monkeypatch):
    """DENY: request_decision raises DecisionDenied → wrapper re-raises
    UNMODIFIED; litellm.acompletion NEVER called."""
    cli = _client()
    cli.request_decision = AsyncMock(side_effect=DecisionDenied(
        "STOP", decision_id="dec-1", reason_codes=["BUDGET_EXHAUSTED"],
    ))
    cb = _wrapper(cli)
    acompletion_mock = _patch_litellm_acompletion(monkeypatch)
    with pytest.raises(DecisionDenied, match="STOP"):
        await cb(model="gpt-4o-mini",
                 messages=[{"role": "user", "content": "deny"}])
    acompletion_mock.assert_not_called()
    cli.emit_llm_call_post.assert_not_called()


@pytest.mark.asyncio
async def test_degrade_raises_sidecar_unavailable(monkeypatch):
    """DEGRADE: sidecar response → SidecarUnavailable raised; litellm
    NEVER called."""
    cli = _client()
    cli.request_decision = AsyncMock(return_value=SimpleNamespace(
        decision="DEGRADE", decision_id="dec-1", reservation_ids=(),
        audit_decision_event_id="audit-1",
    ))
    cb = _wrapper(cli)
    acompletion_mock = _patch_litellm_acompletion(monkeypatch)
    with pytest.raises(SidecarUnavailable, match="DEGRADE"):
        await cb(model="gpt-4o-mini", messages=[{"role": "user", "content": "x"}])
    acompletion_mock.assert_not_called()


@pytest.mark.asyncio
async def test_pre_call_transport_error_wrapped_as_sidecar_unavailable(monkeypatch):
    """Pre-call generic SpendGuardError → SidecarUnavailable; litellm
    NOT called."""
    cli = _client()
    cli.request_decision = AsyncMock(side_effect=SpendGuardError("rpc boom"))
    cb = _wrapper(cli)
    acompletion_mock = _patch_litellm_acompletion(monkeypatch)
    with pytest.raises(SidecarUnavailable, match="rpc boom"):
        await cb(model="gpt-4o-mini", messages=[{"role": "user", "content": "x"}])
    acompletion_mock.assert_not_called()


@pytest.mark.asyncio
async def test_provider_raises_releases_reservation_and_reraises(monkeypatch):
    """litellm.acompletion raises → emit FAILURE release + re-raise
    original exception."""
    err = RuntimeError("provider 500")
    cli = _client()
    cb = _wrapper(cli)
    _patch_litellm_acompletion(monkeypatch, side_effect=err)
    with pytest.raises(RuntimeError, match="provider 500"):
        await cb(model="gpt-4o-mini", messages=[{"role": "user", "content": "x"}])
    # Release fired with outcome=FAILURE
    cli.emit_llm_call_post.assert_called_once()
    kw = cli.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "FAILURE"
    assert kw["reservation_id"] == "res-1"
    assert kw["estimated_amount_atomic"] == "0"


@pytest.mark.asyncio
async def test_provider_cancelled_classifies_as_cancelled(monkeypatch):
    """asyncio.CancelledError is BaseException — but isinstance check
    inside `except Exception` is dead code. This test documents that
    the wrapper does NOT catch bare CancelledError; cancellation
    propagates without a release. (TTL sweep is durable backstop.)"""
    cli = _client()
    cb = _wrapper(cli)
    _patch_litellm_acompletion(
        monkeypatch, side_effect=asyncio.CancelledError(),
    )
    with pytest.raises(asyncio.CancelledError):
        await cb(model="gpt-4o-mini", messages=[{"role": "user", "content": "x"}])
    # CancelledError is BaseException → except Exception doesn't catch it.
    # Release branch never fires. TTL sweep cleans up.
    cli.emit_llm_call_post.assert_not_called()


@pytest.mark.asyncio
async def test_commit_failure_swallowed_response_still_returned(monkeypatch, caplog):
    """Commit-time SpendGuardError → swallowed with WARN; caller still
    gets provider response (commit failure must NOT mask successful
    provider call). Slice A1 architect-noted contract."""
    import logging as _logging
    cli = _client()
    cli.emit_llm_call_post = AsyncMock(side_effect=SpendGuardError("commit boom"))
    cb = _wrapper(cli)
    _patch_litellm_acompletion(monkeypatch)
    with caplog.at_level(_logging.WARNING, logger="spendguard.integrations.litellm"):
        resp = await cb(model="gpt-4o-mini",
                        messages=[{"role": "user", "content": "x"}])
    assert resp.id == "chatcmpl-x"  # provider response returned
    assert any("commit RPC failed" in r.message for r in caplog.records)


@pytest.mark.asyncio
async def test_stream_true_rejected_before_reserve(monkeypatch):
    """stream=True → SpendGuardConfigError; no reservation, no
    litellm.acompletion call. Slice A1 deferred-streaming contract."""
    cli = _client()
    cb = _wrapper(cli)
    acompletion_mock = _patch_litellm_acompletion(monkeypatch)
    with pytest.raises(SpendGuardConfigError, match="stream=True"):
        await cb(model="gpt-4o-mini", stream=True,
                 messages=[{"role": "user", "content": "x"}])
    cli.request_decision.assert_not_called()
    acompletion_mock.assert_not_called()


@pytest.mark.asyncio
async def test_fail_open_bypasses_pre_call_error(monkeypatch):
    """SPENDGUARD_LITELLM_FAIL_OPEN=1 + pre-call SpendGuardError →
    bypass to litellm.acompletion; NO reservation, NO commit."""
    monkeypatch.setenv("SPENDGUARD_LITELLM_FAIL_OPEN", "1")
    cli = _client()
    cli.request_decision = AsyncMock(side_effect=SpendGuardError("rpc down"))
    # IMPORTANT: must construct wrapper AFTER env var set (read at __init__)
    cb = _wrapper(cli)
    acompletion_mock = _patch_litellm_acompletion(monkeypatch)
    resp = await cb(model="gpt-4o-mini",
                    messages=[{"role": "user", "content": "x"}])
    assert resp.id == "chatcmpl-x"
    acompletion_mock.assert_called_once()
    cli.emit_llm_call_post.assert_not_called()


@pytest.mark.asyncio
async def test_fail_open_bypasses_degrade(monkeypatch):
    """SPENDGUARD_LITELLM_FAIL_OPEN=1 + DEGRADE outcome → bypass."""
    monkeypatch.setenv("SPENDGUARD_LITELLM_FAIL_OPEN", "1")
    cli = _client()
    cli.request_decision = AsyncMock(return_value=SimpleNamespace(
        decision="DEGRADE", decision_id="dec-1", reservation_ids=(),
        audit_decision_event_id="audit-1",
    ))
    cb = _wrapper(cli)
    _patch_litellm_acompletion(monkeypatch)
    resp = await cb(model="gpt-4o-mini",
                    messages=[{"role": "user", "content": "x"}])
    assert resp.id == "chatcmpl-x"


@pytest.mark.asyncio
async def test_resolver_none_rejected_before_reserve(monkeypatch):
    """budget_resolver returns None → SpendGuardConfigError; no
    reservation, no litellm call."""
    cli = _client()
    cb = SpendGuardDirectAcompletion(
        client=cli,
        budget_resolver=lambda ctx: None,
        claim_estimator=lambda ctx: [_claim()],
        claim_reconciler=lambda ctx, resp: [_claim()],
    )
    acompletion_mock = _patch_litellm_acompletion(monkeypatch)
    with pytest.raises(SpendGuardConfigError, match="budget_resolver"):
        await cb(model="gpt-4o-mini",
                 messages=[{"role": "user", "content": "x"}])
    cli.request_decision.assert_not_called()
    acompletion_mock.assert_not_called()


@pytest.mark.asyncio
async def test_estimator_wrong_cardinality_rejected(monkeypatch):
    """claim_estimator must return EXACTLY 1 claim."""
    cli = _client()
    cb = SpendGuardDirectAcompletion(
        client=cli,
        budget_resolver=lambda ctx: _BINDING,
        claim_estimator=lambda ctx: [_claim(), _claim()],
        claim_reconciler=lambda ctx, resp: [_claim()],
    )
    acompletion_mock = _patch_litellm_acompletion(monkeypatch)
    with pytest.raises(SpendGuardConfigError, match="2 claims"):
        await cb(model="gpt-4o-mini",
                 messages=[{"role": "user", "content": "x"}])
    acompletion_mock.assert_not_called()


@pytest.mark.asyncio
async def test_reconciler_binding_mismatch_rejected(monkeypatch):
    """claim_reconciler returning a different budget_id → reject AFTER
    provider call succeeded → commit NOT emitted (audit row missing
    BUT no double-charge). Reservation will TTL-sweep."""
    cli = _client()
    bad_claim = SimpleNamespace(
        amount_atomic="92", budget_id="OTHER",
        window_instance_id="w1", unit=SimpleNamespace(unit_id="u1"),
    )
    cb = SpendGuardDirectAcompletion(
        client=cli,
        budget_resolver=lambda ctx: _BINDING,
        claim_estimator=lambda ctx: [_claim()],
        claim_reconciler=lambda ctx, resp: [bad_claim],
    )
    _patch_litellm_acompletion(monkeypatch)
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        await cb(model="gpt-4o-mini",
                 messages=[{"role": "user", "content": "x"}])
    # request_decision happened; emit did NOT (reconciler validation
    # ran before emit_llm_call_post).
    cli.emit_llm_call_post.assert_not_called()


@pytest.mark.asyncio
async def test_concurrent_calls_get_distinct_call_ids(monkeypatch):
    """asyncio.gather of N concurrent calls produce N distinct
    litellm_call_ids (no signature collision via urandom mix-in)."""
    cli = _client()
    cb = _wrapper(cli)
    _patch_litellm_acompletion(monkeypatch)
    await asyncio.gather(*(
        cb(model="gpt-4o-mini",
           messages=[{"role": "user", "content": f"msg-{i}"}])
        for i in range(8)
    ))
    # 8 distinct request_decision calls
    assert cli.request_decision.call_count == 8
    # 8 distinct litellm_call_id values
    call_ids = [c.kwargs["llm_call_id"]
                for c in cli.request_decision.call_args_list]
    assert len(set(call_ids)) == 8, f"collisions found: {call_ids}"


@pytest.mark.asyncio
async def test_litellm_call_id_caller_supplied_honored(monkeypatch):
    """If caller passes `litellm_call_id`, the wrapper uses it (does
    NOT generate a fresh one). Enables external correlation."""
    cli = _client()
    cb = _wrapper(cli)
    _patch_litellm_acompletion(monkeypatch)
    await cb(model="gpt-4o-mini",
             litellm_call_id="caller-supplied-id",
             messages=[{"role": "user", "content": "x"}])
    # The same litellm_call_id flows through to llm_call_id derivation,
    # which uses a derived UUID. But the source signature is the same
    # → idempotent retries with the same caller-id yield the same llm_call_id.
    cid_1 = cli.request_decision.call_args.kwargs["llm_call_id"]
    # Re-run with same caller id → same derived UUID
    cli2 = _client()
    cb2 = _wrapper(cli2)
    _patch_litellm_acompletion(monkeypatch)
    await cb2(model="gpt-4o-mini",
              litellm_call_id="caller-supplied-id",
              messages=[{"role": "user", "content": "x"}])
    cid_2 = cli2.request_decision.call_args.kwargs["llm_call_id"]
    assert cid_1 == cid_2, "idempotent retries should yield same llm_call_id"
