# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S106
"""Slice 2 — Tier 1 unit tests per TEST_PLAN.md §2.2.

Tests `async_pre_call_hook` against a mock SpendGuardClient. Per TEST_PLAN
the mocking line: at Tier 1 it's acceptable to mock SpendGuardClient
(Tier 2/3 ban this). The unit tests pin the callback contract.
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
    DecisionDenied,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardError,
)
from spendguard.integrations.litellm import (  # noqa: E402
    BudgetBinding,
    LiteLLMRunContext,
    ResolverContext,
    SpendGuardLiteLLMCallback,
    run_context,
)


@dataclass(frozen=True)
class _FakePricing:
    pricing_version: str = "v1"
    price_snapshot_hash_hex: str = "deadbeef"
    fx_rate_version: str = "fxv1"
    unit_conversion_version: str = "uv1"


_FAKE_BINDING = BudgetBinding(
    budget_id="b1",
    window_instance_id="w1",
    unit=SimpleNamespace(unit_id="u1", token_kind="output_token"),
    pricing=_FakePricing(),
)


def _make_client_mock(
    *,
    decision_id: str = "dec-1",
    reservation_ids: tuple = ("res-1",),
    audit_event_id: str = "audit-1",
    request_decision_side_effect=None,
):
    client = MagicMock()
    client.tenant_id = "tenant-1"
    client.session_id = "session-1"

    outcome = SimpleNamespace(
        decision_id=decision_id,
        reservation_ids=reservation_ids,
        audit_decision_event_id=audit_event_id,
    )
    if request_decision_side_effect is not None:
        client.request_decision = AsyncMock(
            side_effect=request_decision_side_effect
        )
    else:
        client.request_decision = AsyncMock(return_value=outcome)
    return client


def _make_callback(
    *,
    client=None,
    resolver=lambda ctx: _FAKE_BINDING,
    estimator=lambda ctx: [SimpleNamespace(amount_atomic="100")],
    reconciler=lambda ctx, resp: [SimpleNamespace(amount_atomic="100")],
    fail_closed: bool = True,
):
    return SpendGuardLiteLLMCallback(
        client=client or _make_client_mock(),
        budget_resolver=resolver,
        claim_estimator=estimator,
        claim_reconciler=reconciler,
        fail_closed=fail_closed,
    )


def _data(call_id: str = "litellm-call-1", **extra):
    return {"litellm_call_id": call_id, "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": "hi"}], **extra}


@pytest.mark.asyncio
async def test_pre_call_hook_builds_resolver_context_with_user_api_key_dict():
    """P0.2 fix: ResolverContext.user_api_key_dict from hook arg, not data."""
    captured = []

    def _resolver(ctx: ResolverContext):
        captured.append(ctx)
        return _FAKE_BINDING

    cb = _make_callback(resolver=_resolver)
    uak = SimpleNamespace(team_id="t1")
    await cb.async_pre_call_hook(uak, None, _data(), "acompletion")
    assert len(captured) == 1
    assert captured[0].user_api_key_dict is uak
    assert captured[0].call_type == "acompletion"


@pytest.mark.asyncio
async def test_pre_call_hook_uses_claim_estimator_output_single_claim():
    """v1 contract: estimator returns exactly 1 claim; it lands in
    request_decision(projected_claims=[...])."""
    estimator_claim = SimpleNamespace(amount_atomic="500")
    client = _make_client_mock()
    cb = _make_callback(client=client, estimator=lambda ctx: [estimator_claim])
    await cb.async_pre_call_hook(None, None, _data(), "acompletion")
    call_kwargs = client.request_decision.call_args.kwargs
    assert call_kwargs["projected_claims"] == [estimator_claim]


@pytest.mark.asyncio
async def test_pre_call_hook_stashes_reservation_ids_tuple():
    """R0 P0.7 fix: reservation_ids stashed as a TUPLE (plural)."""
    client = _make_client_mock(reservation_ids=("r-a",))
    cb = _make_callback(client=client)
    await cb.async_pre_call_hook(None, None, _data("c-99"), "acompletion")
    stash = cb._stash["c-99"]
    assert isinstance(stash["reservation_ids"], tuple)
    assert stash["reservation_ids"] == ("r-a",)
    assert stash["decision_id"] == "dec-1"


@pytest.mark.asyncio
async def test_pre_call_hook_does_not_mutate_data_with_spendguard_key():
    """P1.5 fix: returned data has NO `spendguard` key — stash lives on
    self._stash, NEVER on data (would leak to provider wire)."""
    cb = _make_callback()
    data = _data("c-mut")
    result = await cb.async_pre_call_hook(None, None, data, "acompletion")
    assert "spendguard" not in (result or {})
    assert "spendguard" not in data


@pytest.mark.asyncio
async def test_pre_call_hook_raises_on_resolver_returns_none():
    """P0.10: resolver returning None → SpendGuardConfigError. No
    global default fallback (ADR-001)."""
    cb = _make_callback(resolver=lambda ctx: None)
    with pytest.raises(SpendGuardConfigError, match="resolver returned None"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")


@pytest.mark.asyncio
async def test_pre_call_hook_raises_when_litellm_call_id_missing():
    """Pivot R0 P1.1: missing litellm_call_id is fail-closed (would
    break commit lookup + LiteLLM_SpendLogs reconciliation)."""
    cb = _make_callback()
    with pytest.raises(SpendGuardConfigError, match="litellm_call_id.* missing"):
        # no litellm_call_id key
        await cb.async_pre_call_hook(
            None, None, {"model": "gpt-4o-mini", "messages": []}, "acompletion"
        )


@pytest.mark.asyncio
async def test_pre_call_hook_rejects_multi_claim_estimator():
    """R3 P1.2: estimator returning ≠1 claim fails BEFORE the wire."""
    multi = [SimpleNamespace(), SimpleNamespace()]
    cb = _make_callback(estimator=lambda ctx: multi)
    with pytest.raises(SpendGuardConfigError, match=r"returned 2 claims"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")


@pytest.mark.asyncio
async def test_pre_call_hook_rejects_multi_reservation_outcome():
    """R4 P0.2: validate reservation_ids cardinality BEFORE returning
    (proxy would otherwise contact provider before error surfaces)."""
    client = _make_client_mock(reservation_ids=("r1", "r2"))
    cb = _make_callback(client=client)
    with pytest.raises(SpendGuardConfigError, match=r"2 reservations"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")


@pytest.mark.asyncio
async def test_pre_call_hook_propagates_decision_denied():
    """DENY path: DecisionDenied propagates (proxy blocks the call)."""
    deny = DecisionDenied(
        "over budget", decision_id="d-x", reason_codes=["BUDGET_EXCEEDED"],
    )
    client = _make_client_mock(request_decision_side_effect=deny)
    cb = _make_callback(client=client)
    with pytest.raises(DecisionDenied) as exc_info:
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")
    assert exc_info.value.reason_codes == ["BUDGET_EXCEEDED"]


@pytest.mark.asyncio
async def test_pre_call_hook_wraps_sidecar_error_as_sidecar_unavailable():
    """fail_closed (default): SpendGuardError → SidecarUnavailable."""
    err = SpendGuardError("sidecar boom")
    client = _make_client_mock(request_decision_side_effect=err)
    cb = _make_callback(client=client)
    with pytest.raises(SidecarUnavailable, match="sidecar pre-call failed"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")


@pytest.mark.asyncio
async def test_pre_call_hook_fail_open_env_returns_data(monkeypatch, caplog):
    """SPENDGUARD_LITELLM_FAIL_OPEN=1: sidecar error → log WARNING +
    return data; no exception; no stash entry. S6 says WARNING must
    fire at both construction AND each fail-open path taken."""
    import logging as _logging
    monkeypatch.setenv("SPENDGUARD_LITELLM_FAIL_OPEN", "1")
    err = SpendGuardError("sidecar boom")
    client = _make_client_mock(request_decision_side_effect=err)
    cb = _make_callback(client=client)
    data = _data("real-call-id-77")  # Slice 2 R1 P2.2 fix: use real call id
    with caplog.at_level(_logging.WARNING, logger="spendguard.integrations.litellm"):
        result = await cb.async_pre_call_hook(None, None, data, "acompletion")
    assert result is data
    assert "real-call-id-77" not in cb._stash  # no stash on fail-open
    # S6: runtime WARNING must fire on the fail-open path taken.
    assert any("FAIL_OPEN=1" in r.message and "allowing call" in r.message
               for r in caplog.records), "fail-open runtime WARNING missing"


@pytest.mark.asyncio
async def test_pre_call_hook_raises_when_client_is_none():
    """SpendGuardLiteLLMCallback constructed without a client + called
    directly (not via _LoopBoundCallback wrapper) → SpendGuardConfigError."""
    cb = _make_callback(client=None)
    # Override: client=None bypasses our default
    cb._client = None
    with pytest.raises(SpendGuardConfigError, match="has no client"):
        await cb.async_pre_call_hook(None, None, _data(), "acompletion")


@pytest.mark.asyncio
async def test_pre_call_hook_decision_context_has_12_fields():
    """DESIGN §8.2a: decision_context_json carries 12 named fields.
    Verify via the stashed copy."""
    cb = _make_callback()
    await cb.async_pre_call_hook(
        SimpleNamespace(team_id="t-7"),
        None,
        _data("c-12fields"),
        "acompletion",
    )
    ctx = cb._stash["c-12fields"]["decision_context"]
    expected = {
        "integration", "litellm_call_id", "model",
        "pricing_version", "price_snapshot_hash_hex",
        "fx_rate_version", "unit_conversion_version",
        "prompt_hash", "call_type", "stream",
        "mode", "team_id",
    }
    assert set(ctx.keys()) == expected
    assert ctx["integration"] == "litellm"
    assert ctx["mode"] == "proxy"  # v1 always proxy
    assert ctx["team_id"] == "t-7"


@pytest.mark.asyncio
async def test_pre_call_hook_uses_run_context_run_id():
    """When `run_context` is active, hook uses ctx.run_id; else
    derives from litellm_call_id deterministically (P1.6 fix)."""
    cb = _make_callback()
    async with run_context(LiteLLMRunContext(run_id="my-run", step_id="my-step")):
        await cb.async_pre_call_hook(None, None, _data("c-rc"), "acompletion")
    assert cb._stash["c-rc"]["run_id"] == "my-run"
    assert cb._stash["c-rc"]["step_id"] == "my-step"


@pytest.mark.asyncio
async def test_pre_call_hook_derives_distinct_decision_id_per_call_id():
    """ADR-002 retry contract: distinct litellm_call_id → distinct
    decision_id (so LiteLLM num_retries doesn't double-reserve).
    Inspect the kwarg passed to request_decision (not the mock's
    outcome, which is fixed)."""
    client = _make_client_mock()
    cb = _make_callback(client=client)
    await cb.async_pre_call_hook(None, None, _data("retry-call-1"), "acompletion")
    d1 = client.request_decision.call_args.kwargs["decision_id"]
    await cb.async_pre_call_hook(None, None, _data("retry-call-2"), "acompletion")
    d2 = client.request_decision.call_args.kwargs["decision_id"]
    assert d1 != d2  # distinct litellm_call_id → distinct decision_id
    # Same litellm_call_id → same decision_id (deterministic).
    await cb.async_pre_call_hook(None, None, _data("retry-call-1"), "acompletion")
    d1_again = client.request_decision.call_args.kwargs["decision_id"]
    assert d1 == d1_again


def test_init_reads_fail_open_env_at_construction(monkeypatch, caplog):
    """S6: FAIL_OPEN=1 logs WARNING at construction (not just at use)."""
    monkeypatch.setenv("SPENDGUARD_LITELLM_FAIL_OPEN", "1")
    import logging as _logging
    with caplog.at_level(_logging.WARNING, logger="spendguard.integrations.litellm"):
        _make_callback()
    assert any("FAIL_OPEN=1" in rec.message for rec in caplog.records)


def test_init_rejects_negative_ttl_seconds(monkeypatch):
    monkeypatch.setenv("SPENDGUARD_LITELLM_TTL_SECONDS", "-1")
    with pytest.raises(SpendGuardConfigError, match="non-negative"):
        _make_callback()


def test_init_default_ttl_seconds_is_300(monkeypatch):
    monkeypatch.delenv("SPENDGUARD_LITELLM_TTL_SECONDS", raising=False)
    cb = _make_callback()
    assert cb._ttl_seconds == 300
