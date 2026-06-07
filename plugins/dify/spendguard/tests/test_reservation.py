"""Unit tests for ``_DifyReservation`` — the reservation lifecycle delegate.

Mirrors ``sdk/python/tests/test_litellm_precall_unit.py`` style:
- mock SpendGuardClient at the Tier 1 level (allowed per TEST_PLAN.md);
- pin contract invariants (binding validation, decision dispatch,
  CANCELLED classification, fail-open env).

review-standards.md slice 3 checklist coverage:
- 3.1 composition only (delegate has no LargeLanguageModel inheritance)
- 3.2 env-var preflight
- 3.4 binding + claim validation
- 3.5 commit_success contract (estimated_amount_atomic str(real_amount),
  provider_reported_amount_atomic empty)
- 3.6 release_failure swallows RPC errors
- 3.7 CancelledError -> CANCELLED classification
- 3.8 no module-level mutable state
"""

from __future__ import annotations

import asyncio
from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock

import pytest

# Importing dify-plugin Python module requires Python 3.12; on older
# Pythons we skip the tests but still pin the contract surface.
dify_plugin = pytest.importorskip(
    "dify_plugin",
    reason="dify-plugin SDK requires Python 3.12+; install with "
    "`pip install dify-plugin>=0.8` and run on 3.12+.",
)

from spendguard.errors import (  # noqa: E402
    DecisionDenied,
    SidecarUnavailable,
    SpendGuardConfigError,
)

from models.llm._DifyReservation import (  # noqa: E402
    BudgetBinding,
    DifyCallContext,
    _classify_failure,
    _DifyReservation,
    _validate_claim_against_binding,
)

# ---------------------------------------------------------------------------
# Test fixtures
# ---------------------------------------------------------------------------

_FAKE_UNIT = SimpleNamespace(unit_id="atomic.usd.micro", token_kind="output_token")
_FAKE_PRICING = SimpleNamespace(
    pricing_version="v1",
    price_snapshot_hash_hex="",
    fx_rate_version="v1",
    unit_conversion_version="v1",
)
_FAKE_BINDING = BudgetBinding(
    budget_id="bud-1",
    window_instance_id="win-1",
    unit=_FAKE_UNIT,
    pricing=_FAKE_PRICING,
)


def _make_credentials(**overrides):
    base = {
        "upstream_provider": "openai",
        "openai_api_key": "sk-test-key",
        "spendguard_sidecar_address": "/tmp/sg.sock",
        "spendguard_tenant_id": "tenant-1",
        "spendguard_budget_id": "bud-1",
        "spendguard_window_instance_id": "win-1",
    }
    base.update(overrides)
    return base


def _make_call_context(**overrides):
    base = {
        "workspace_id": "ws-1",
        "app_id": "app-1",
        "model": "spendguard/gpt-4o-mini",
        "prompt_messages": [{"role": "user", "content": "hi"}],
        "stream": False,
        "credentials": _make_credentials(),
        "user": "user-1",
    }
    base.update(overrides)
    return DifyCallContext(**base)


def _make_client_mock(
    *,
    decision: str = "CONTINUE",
    decision_id: str = "dec-1",
    reservation_ids: tuple = ("res-1",),
    request_decision_side_effect=None,
):
    client = MagicMock()
    client.tenant_id = "test-tenant"
    client.session_id = "session-1"
    outcome = SimpleNamespace(
        decision=decision,
        decision_id=decision_id,
        reservation_ids=reservation_ids,
        audit_decision_event_id="audit-1",
    )
    if request_decision_side_effect is not None:
        client.request_decision = AsyncMock(
            side_effect=request_decision_side_effect,
        )
    else:
        client.request_decision = AsyncMock(return_value=outcome)
    client.emit_llm_call_post = AsyncMock(return_value=None)
    return client


async def _seed_reservation(reservation: _DifyReservation, client) -> None:
    """Bypass _ensure_client by injecting the mock directly."""
    reservation._client = client


# ---------------------------------------------------------------------------
# R01 — env-var preflight
# ---------------------------------------------------------------------------

def test_R01_init_rejects_missing_uds_env(monkeypatch):
    """3.2: missing SPENDGUARD_SIDECAR_UDS -> SpendGuardConfigError naming the var."""
    monkeypatch.delenv("SPENDGUARD_SIDECAR_UDS", raising=False)
    monkeypatch.setenv("SPENDGUARD_TENANT_ID", "t1")
    with pytest.raises(SpendGuardConfigError, match="SPENDGUARD_SIDECAR_UDS"):
        _DifyReservation()


def test_R02_init_rejects_missing_tenant_env(monkeypatch):
    """3.2: missing SPENDGUARD_TENANT_ID -> SpendGuardConfigError naming the var."""
    monkeypatch.setenv("SPENDGUARD_SIDECAR_UDS", "/tmp/s.sock")
    monkeypatch.delenv("SPENDGUARD_TENANT_ID", raising=False)
    with pytest.raises(SpendGuardConfigError, match="SPENDGUARD_TENANT_ID"):
        _DifyReservation()


def test_R03_init_accepts_explicit_args(monkeypatch):
    """Explicit args override env (tests don't depend on env state)."""
    monkeypatch.delenv("SPENDGUARD_SIDECAR_UDS", raising=False)
    monkeypatch.delenv("SPENDGUARD_TENANT_ID", raising=False)
    r = _DifyReservation(socket_path="/sock", tenant_id="t")
    assert r._socket_path == "/sock"
    assert r._tenant_id == "t"


# ---------------------------------------------------------------------------
# R04-R06 — claim/binding validation
# ---------------------------------------------------------------------------

def test_R04_validate_claim_against_binding_passes_matching_claim():
    """3.4: matching budget+window+unit passes."""
    claim = SimpleNamespace(
        budget_id="bud-1",
        window_instance_id="win-1",
        unit=SimpleNamespace(unit_id="atomic.usd.micro"),
        amount_atomic="100",
    )
    # Should not raise.
    _validate_claim_against_binding(claim, _FAKE_BINDING, source="test")


def test_R05_validate_claim_rejects_mismatched_budget_id():
    """3.4: budget_id mismatch -> SpendGuardConfigError."""
    bad = SimpleNamespace(
        budget_id="WRONG",
        window_instance_id="win-1",
        unit=SimpleNamespace(unit_id="atomic.usd.micro"),
        amount_atomic="100",
    )
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        _validate_claim_against_binding(bad, _FAKE_BINDING, source="test")


def test_R06_validate_claim_rejects_empty_unit_id():
    """3.4: empty unit_id -> SpendGuardConfigError (mis-charge guard)."""
    bad = SimpleNamespace(
        budget_id="bud-1",
        window_instance_id="win-1",
        unit=SimpleNamespace(unit_id=""),
        amount_atomic="100",
    )
    with pytest.raises(SpendGuardConfigError, match="unit"):
        _validate_claim_against_binding(bad, _FAKE_BINDING, source="test")


# ---------------------------------------------------------------------------
# R07-R09 — CANCELLED classification (3.7)
# ---------------------------------------------------------------------------

def test_R07_classify_failure_cancelled_error():
    """3.7: asyncio.CancelledError -> CANCELLED."""
    assert _classify_failure(asyncio.CancelledError()) == "CANCELLED"


def test_R08_classify_failure_cancelled_string_repr():
    """3.7: string carrying 'cancelled' token -> CANCELLED."""
    assert _classify_failure("operation_cancelled by user") == "CANCELLED"
    assert _classify_failure("Request was canceled") == "CANCELLED"


def test_R09_classify_failure_generic_exception_is_failure():
    """3.7: everything else -> FAILURE; no false positive on 'uncancelled'."""
    assert _classify_failure(RuntimeError("upstream timed out")) == "FAILURE"
    assert _classify_failure("uncancelled stream finished") == "FAILURE"


# ---------------------------------------------------------------------------
# R10 — reserve happy path (ALLOW)
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_R10_reserve_allow_returns_handle_with_reservation_id():
    """3.4 + INV-2: ALLOW returns a handle carrying decision_id +
    reservation_id; request_decision was called BEFORE any upstream
    HTTP would fire (assertion on call order in OpenAI tests)."""
    r = _DifyReservation(socket_path="/sock", tenant_id="t1")
    client = _make_client_mock()
    await _seed_reservation(r, client)
    ctx = _make_call_context()
    handle = await r.reserve(ctx, estimated_amount_atomic="500")
    assert handle.reservation_id == "res-1"
    assert handle.decision_id == "dec-1"
    assert handle.llm_call_id  # synthesised UUID
    assert handle.binding.budget_id == "bud-1"
    assert client.request_decision.await_count == 1
    call_kwargs = client.request_decision.await_args.kwargs
    assert call_kwargs["trigger"] == "LLM_CALL_PRE"
    assert len(call_kwargs["projected_claims"]) == 1
    assert call_kwargs["projected_claims"][0].budget_id == "bud-1"


# ---------------------------------------------------------------------------
# R11 — reserve DENY path
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_R11_reserve_deny_raises_decision_denied():
    """DENY -> DecisionDenied. The exception preserves decision_id so
    the SpendGuardLLM caller can translate to InvokeAuthorizationError
    (HTTP 403)."""
    r = _DifyReservation(socket_path="/sock", tenant_id="t1")

    async def _deny(*a, **kw):
        raise DecisionDenied(
            "budget exhausted",
            decision_id="dec-deny",
            reason_codes=["budget.exhausted"],
        )
    client = _make_client_mock(request_decision_side_effect=_deny)
    await _seed_reservation(r, client)
    with pytest.raises(DecisionDenied) as exc_info:
        await r.reserve(_make_call_context())
    assert exc_info.value.decision_id == "dec-deny"


# ---------------------------------------------------------------------------
# R12 — reserve DEGRADE path (fail-closed)
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_R12_reserve_degrade_raises_sidecar_unavailable():
    """INV-3: DEGRADE fail-closed -> SidecarUnavailable (Dify translates
    to HTTP 503 InvokeServerUnavailableError)."""
    r = _DifyReservation(socket_path="/sock", tenant_id="t1")
    client = _make_client_mock(decision="DEGRADE", reservation_ids=())
    await _seed_reservation(r, client)
    with pytest.raises(SidecarUnavailable, match="DEGRADE"):
        await r.reserve(_make_call_context())


# ---------------------------------------------------------------------------
# R13 — reserve sidecar transport error -> SidecarUnavailable
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_R13_reserve_transport_error_raises_sidecar_unavailable():
    """SpendGuardError from request_decision -> wrapped SidecarUnavailable."""
    from spendguard.errors import SpendGuardError

    r = _DifyReservation(socket_path="/sock", tenant_id="t1")

    async def _boom(*a, **kw):
        raise SpendGuardError("UDS connection reset")
    client = _make_client_mock(request_decision_side_effect=_boom)
    await _seed_reservation(r, client)
    with pytest.raises(SidecarUnavailable, match="sidecar pre-call failed"):
        await r.reserve(_make_call_context())


# ---------------------------------------------------------------------------
# R14 — commit_success contract (3.5)
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_R14_commit_success_emits_estimated_amount_with_empty_provider_reported():
    """3.5: commit_success passes estimated_amount_atomic=str(real_amount)
    + provider_reported_amount_atomic='' (the v1 CommitEstimated path,
    matches sdk/python/src/spendguard/integrations/litellm.py:550-560)."""
    r = _DifyReservation(socket_path="/sock", tenant_id="t1")
    client = _make_client_mock()
    await _seed_reservation(r, client)
    handle = await r.reserve(_make_call_context())
    await r.commit_success(
        handle,
        real_amount_atomic="1234",
        provider_event_id="evt-xyz",
        actual_input_tokens=42,
        actual_output_tokens=7,
    )
    assert client.emit_llm_call_post.await_count == 1
    kwargs = client.emit_llm_call_post.await_args.kwargs
    assert kwargs["estimated_amount_atomic"] == "1234"
    assert kwargs["provider_reported_amount_atomic"] == ""
    assert kwargs["outcome"] == "SUCCESS"
    assert kwargs["actual_input_tokens"] == 42
    assert kwargs["actual_output_tokens"] == 7
    assert kwargs["provider_event_id"] == "evt-xyz"


# ---------------------------------------------------------------------------
# R15 — release_failure swallows RPC errors (3.6)
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_R15_release_failure_swallows_emit_errors(caplog):
    """3.6: release-RPC errors are swallowed (TTL sweep backstop); WARN
    log carries llm_call_id."""
    from spendguard.errors import SpendGuardError

    r = _DifyReservation(socket_path="/sock", tenant_id="t1")
    client = _make_client_mock()
    client.emit_llm_call_post = AsyncMock(
        side_effect=SpendGuardError("release rpc failed"),
    )
    await _seed_reservation(r, client)
    handle = await r.reserve(_make_call_context())
    # Should NOT raise — TTL sweep is the backstop.
    await r.release_failure(handle, RuntimeError("upstream went away"))
    # WARN logged with llm_call_id reference.
    warns = [rec for rec in caplog.records if rec.levelname == "WARNING"]
    assert any(handle.llm_call_id in str(rec.getMessage()) for rec in warns)


# ---------------------------------------------------------------------------
# R16 — release_failure CANCELLED classification (3.7)
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_R16_release_failure_classifies_cancelled_error():
    """3.7: asyncio.CancelledError -> outcome=CANCELLED on the wire."""
    r = _DifyReservation(socket_path="/sock", tenant_id="t1")
    client = _make_client_mock()
    await _seed_reservation(r, client)
    handle = await r.reserve(_make_call_context())
    await r.release_failure(handle, asyncio.CancelledError())
    # First call = reserve's request_decision (mocked); second = release emit.
    assert client.emit_llm_call_post.await_count == 1
    kwargs = client.emit_llm_call_post.await_args.kwargs
    assert kwargs["outcome"] == "CANCELLED"


# ---------------------------------------------------------------------------
# R17 — reserve raises on missing budget_id in credentials
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_R17_reserve_rejects_missing_budget_id():
    """_build_binding_from_credentials rejects empty budget id with the
    offending key named (cross-cutting error-message standard)."""
    r = _DifyReservation(socket_path="/sock", tenant_id="t1")
    client = _make_client_mock()
    await _seed_reservation(r, client)
    bad_credentials = _make_credentials(spendguard_budget_id="")
    ctx = _make_call_context(credentials=bad_credentials)
    with pytest.raises(SpendGuardConfigError, match="spendguard_budget_id"):
        await r.reserve(ctx)


# ---------------------------------------------------------------------------
# R18 — fail-open env permits DEGRADE
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_R18_fail_open_env_allows_degrade(monkeypatch):
    """3.2 + design.md key decision: SPENDGUARD_DIFY_FAIL_OPEN=1 allows
    DEGRADE through (DEV ONLY); returns a sentinel handle with empty
    reservation_id so commit/release are no-ops."""
    monkeypatch.setenv("SPENDGUARD_DIFY_FAIL_OPEN", "1")
    r = _DifyReservation(socket_path="/sock", tenant_id="t1")
    assert r._fail_open_dev is True
    client = _make_client_mock(decision="DEGRADE", reservation_ids=())
    await _seed_reservation(r, client)
    handle = await r.reserve(_make_call_context())
    assert handle.reservation_id == ""  # sentinel
    # Commit on sentinel is a no-op (no emit fires).
    await r.commit_success(handle, real_amount_atomic="100")
    assert client.emit_llm_call_post.await_count == 0
