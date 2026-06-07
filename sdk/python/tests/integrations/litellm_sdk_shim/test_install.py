# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S106
"""D12 SLICE 1+2+3+4 — install / patch / recursion / Router tests.

Covers the install + uninstall state machine (Slice 1), the
``acompletion`` / ``atext_completion`` patches (Slice 2), the sync
``completion`` patch (Slice 3), and the ``Router.acompletion`` +
subclass walk (Slice 4).

All tests use ``monkeypatch`` + ``AsyncMock`` to avoid any real LiteLLM
provider HTTP. The fake ``SpendGuardClient`` is a ``MagicMock`` that
records ``request_decision`` + ``emit_llm_call_post`` calls.

Per the review-standards Slice 5 §5.1 Blocker, every test that calls
``install_shim`` is wrapped in ``try/finally`` calling
``uninstall_shim`` (the ``shim_clean`` fixture enforces this).
"""

from __future__ import annotations

import asyncio
from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock

import pytest

# Skip the whole module cleanly when LiteLLM isn't installed — the shim
# requires it at install time (litellm is patched in place). We probe
# the litellm module rather than spendguard.integrations.litellm because
# that test-utility module pulls in the proto stubs which is heavier.
pytest.importorskip("litellm", reason="LiteLLM not installed")

import litellm  # noqa: E402

from spendguard.integrations.litellm_sdk_shim import (  # noqa: E402
    SpendGuardShimAlreadyInstalled,
    SpendGuardShimOptions,
    SpendGuardShimSyncInAsyncContext,
    install_shim,
    is_installed,
    uninstall_shim,
)
from spendguard.integrations.litellm_sdk_shim._state import (  # noqa: E402
    _IN_FLIGHT,
)

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


def _fake_client(*, tenant: str = "tenant-1") -> MagicMock:
    """Build a SpendGuardClient mock with the methods _DirectCore calls.

    Defaults to a CONTINUE decision with a single reservation; tests
    override ``request_decision`` / ``emit_llm_call_post`` for DENY /
    DEGRADE / commit-failure scenarios.
    """
    cli = MagicMock()
    cli.tenant_id = tenant
    cli.session_id = "session-1"
    cli.request_decision = AsyncMock(return_value=SimpleNamespace(
        decision="CONTINUE",
        decision_id="dec-1",
        reservation_ids=("res-1",),
        audit_decision_event_id="audit-1",
    ))
    cli.emit_llm_call_post = AsyncMock(return_value=None)
    return cli


@pytest.fixture
def shim_clean():
    """Guarantee ``uninstall_shim()`` runs even if a test fails partway.

    Mirrors the D11 ``shim_clean`` pattern from review-standards §5.1
    (Blocker). Yields a no-op object so tests can also pass it through
    to other fixtures if needed.
    """
    yield
    if is_installed():
        uninstall_shim()


@pytest.fixture
def options(shim_clean):
    """Default options pointing at a fresh fake client."""
    return SpendGuardShimOptions(
        client=_fake_client(),
        tenant_id="tenant-1",
        budget_id="b1",
        fail_open=False,
    )


def _patch_litellm_acompletion(monkeypatch, *, response=None, side_effect=None):
    """Replace the *pre-shim* ``litellm.acompletion`` with a mock.

    MUST be called BEFORE ``install_shim`` so the mock is captured as
    the "original" the shim saves + restores.
    """
    if response is None:
        response = SimpleNamespace(
            id="chatcmpl-x",
            usage=SimpleNamespace(prompt_tokens=10, completion_tokens=92),
        )
    mock = AsyncMock(return_value=response, side_effect=side_effect)
    monkeypatch.setattr(litellm, "acompletion", mock)
    return mock


# ---------------------------------------------------------------------------
# Slice 1 — install / uninstall / idempotency
# ---------------------------------------------------------------------------


def test_install_then_is_installed_true(monkeypatch, options):
    """``install_shim`` flips ``is_installed`` from False → True."""
    _patch_litellm_acompletion(monkeypatch)
    assert is_installed() is False
    install_shim(options)
    assert is_installed() is True


def test_install_idempotent_same_options(monkeypatch, options):
    """Re-calling with the same options is a silent no-op.

    The originals stack length stays constant — proves the second
    install did NOT re-stack the originals (which would corrupt
    uninstall's reverse-walk).
    """
    _patch_litellm_acompletion(monkeypatch)
    install_shim(options)
    from spendguard.integrations.litellm_sdk_shim._state import _current_state

    first_originals = list(_current_state().originals)
    install_shim(options)  # idempotent
    second_originals = list(_current_state().originals)
    assert first_originals == second_originals


def test_install_different_config_raises(monkeypatch, options):
    """A second ``install_shim`` with different options raises."""
    _patch_litellm_acompletion(monkeypatch)
    install_shim(options)
    other = SpendGuardShimOptions(
        client=_fake_client(),  # different client → different signature
        tenant_id="tenant-1",
        budget_id="b1",
        fail_open=False,
    )
    with pytest.raises(SpendGuardShimAlreadyInstalled):
        install_shim(other)


def test_uninstall_restores_originals(monkeypatch, options):
    """After ``uninstall_shim``, every patched attribute is its pre-shim
    value. The most load-bearing one is ``litellm.acompletion``."""
    original_acompletion = _patch_litellm_acompletion(monkeypatch)
    original_completion = litellm.completion
    original_router_a = litellm.Router.acompletion
    install_shim(options)
    assert litellm.acompletion is not original_acompletion  # patched
    assert litellm.Router.acompletion is not original_router_a
    uninstall_shim()
    assert litellm.acompletion is original_acompletion
    assert litellm.completion is original_completion
    assert litellm.Router.acompletion is original_router_a
    assert is_installed() is False


def test_uninstall_when_not_installed_is_noop():
    """``uninstall_shim`` is safe to call when nothing is installed."""
    assert is_installed() is False
    uninstall_shim()  # must not raise
    assert is_installed() is False


# ---------------------------------------------------------------------------
# Slice 2 — patched acompletion routes through the core
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_acompletion_patched_routes_through_core(monkeypatch, options):
    """``await litellm.acompletion(...)`` after install drives the
    sidecar reserve + commit."""
    acomp_mock = _patch_litellm_acompletion(monkeypatch)
    install_shim(options)
    resp = await litellm.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "hi"}],
    )
    assert resp.id == "chatcmpl-x"
    # Sidecar reserve + commit fired exactly once each.
    options.client.request_decision.assert_called_once()
    options.client.emit_llm_call_post.assert_called_once()
    # Original acompletion was driven exactly once.
    acomp_mock.assert_called_once()
    # Commit outcome is SUCCESS with the reconciled completion_tokens.
    commit_kw = options.client.emit_llm_call_post.call_args.kwargs
    assert commit_kw["outcome"] == "SUCCESS"
    assert commit_kw["estimated_amount_atomic"] == "92"
    assert commit_kw["actual_output_tokens"] == 92


@pytest.mark.asyncio
async def test_acompletion_reserve_fires_before_provider(monkeypatch, options):
    """INV-2 (LOAD-BEARING): the sidecar reserve completes BEFORE the
    provider HTTP call. Recorded as a strict-order list — anything other
    than ``["reserve", "provider"]`` proves D12's thesis is broken."""
    order: list[str] = []

    async def _record_reserve(**_kw):
        order.append("reserve")
        return SimpleNamespace(
            decision="CONTINUE",
            decision_id="dec-1",
            reservation_ids=("res-1",),
            audit_decision_event_id="audit-1",
        )

    options.client.request_decision = AsyncMock(side_effect=_record_reserve)

    async def _record_provider(**_kw):
        order.append("provider")
        return SimpleNamespace(
            id="chatcmpl-x",
            usage=SimpleNamespace(prompt_tokens=10, completion_tokens=92),
        )

    monkeypatch.setattr(litellm, "acompletion", AsyncMock(side_effect=_record_provider))
    install_shim(options)
    await litellm.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "hi"}],
    )
    assert order == ["reserve", "provider"], (
        f"INV-2 broken: ordering was {order!r}"
    )


@pytest.mark.asyncio
async def test_atext_completion_patched(monkeypatch, options):
    """``atext_completion`` is patched alongside ``acompletion``."""
    original = AsyncMock(return_value=SimpleNamespace(
        id="cmpl-y",
        usage=SimpleNamespace(prompt_tokens=5, completion_tokens=12),
    ))
    monkeypatch.setattr(litellm, "atext_completion", original)
    install_shim(options)
    resp = await litellm.atext_completion(
        model="gpt-3.5-turbo-instruct",
        prompt="hi",
    )
    assert resp.id == "cmpl-y"
    original.assert_called_once()
    options.client.request_decision.assert_called_once()


@pytest.mark.asyncio
async def test_recursion_guard_short_circuits(monkeypatch, options):
    """If the original ``litellm.acompletion`` re-enters
    ``litellm.acompletion`` (e.g. fallback chain), the inner call sees
    ``_IN_FLIGHT=True`` and hits the original directly — no double
    reserve."""
    call_count = {"original": 0}

    async def _self_calling_original(**kwargs):
        call_count["original"] += 1
        if call_count["original"] == 1:
            # Simulate a fallback chain: recurse into the patched
            # entry point. The recursion guard should send us
            # straight to this same function without another reserve.
            await litellm.acompletion(model="gpt-4o-mini", messages=[])
        return SimpleNamespace(
            id="chatcmpl-x",
            usage=SimpleNamespace(prompt_tokens=10, completion_tokens=92),
        )

    monkeypatch.setattr(litellm, "acompletion", AsyncMock(
        side_effect=_self_calling_original,
    ))
    install_shim(options)
    await litellm.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "hi"}],
    )
    # Original called twice (once outer, once via fallback) but reserve
    # only fired ONCE.
    assert call_count["original"] == 2
    assert options.client.request_decision.call_count == 1


@pytest.mark.asyncio
async def test_in_flight_contextvar_isolated_per_task(monkeypatch, options):
    """Two concurrent ``asyncio.gather`` tasks each get their own
    ``_IN_FLIGHT`` token; one's recursion guard never bleeds into the
    other (which would silently disable gating)."""
    _patch_litellm_acompletion(monkeypatch)
    install_shim(options)

    async def _one_call():
        await litellm.acompletion(
            model="gpt-4o-mini",
            messages=[{"role": "user", "content": "x"}],
        )

    await asyncio.gather(_one_call(), _one_call())
    # Both tasks got reservations — no cross-task IN_FLIGHT bleed.
    assert options.client.request_decision.call_count == 2


# ---------------------------------------------------------------------------
# Slice 3 — sync completion + async-context guard
# ---------------------------------------------------------------------------


def test_completion_sync_outside_loop(monkeypatch, options):
    """``litellm.completion(...)`` outside any running loop bridges via
    ``asyncio.run`` and reserves + commits."""
    original = MagicMock(return_value=SimpleNamespace(
        id="cmpl-sync",
        usage=SimpleNamespace(prompt_tokens=4, completion_tokens=8),
    ))
    monkeypatch.setattr(litellm, "completion", original)
    install_shim(options)
    resp = litellm.completion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "sync hi"}],
    )
    assert resp.id == "cmpl-sync"
    original.assert_called_once()
    # Reserve + commit happened on the bridged loop.
    options.client.request_decision.assert_called_once()
    options.client.emit_llm_call_post.assert_called_once()


@pytest.mark.asyncio
async def test_completion_inside_loop_raises(monkeypatch, options):
    """Sync ``litellm.completion`` from inside ``pytest.mark.asyncio``
    raises ``SpendGuardShimSyncInAsyncContext`` — never silently
    deadlocks."""
    monkeypatch.setattr(litellm, "completion", MagicMock())
    install_shim(options)
    with pytest.raises(SpendGuardShimSyncInAsyncContext, match="acompletion"):
        litellm.completion(
            model="gpt-4o-mini",
            messages=[{"role": "user", "content": "x"}],
        )


# ---------------------------------------------------------------------------
# Slice 4 — Router patching
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_router_acompletion_patched(monkeypatch, options):
    """``Router(...).acompletion(...)`` after install routes through
    the core. The Router method is patched at the CLASS level so any
    instance picks up the wrapper."""
    # Patch the parent Router.acompletion BEFORE install so the shim
    # captures the mock as the "original".
    router_mock = AsyncMock(return_value=SimpleNamespace(
        id="chatcmpl-router",
        usage=SimpleNamespace(prompt_tokens=10, completion_tokens=92),
    ))
    monkeypatch.setattr(litellm.Router, "acompletion", router_mock)
    install_shim(options)

    # Build a Router instance with a minimal model_list. We never hit
    # the network because the patched original (our mock) handles
    # dispatch.
    router = litellm.Router(model_list=[
        {
            "model_name": "gpt-4o-mini",
            "litellm_params": {
                "model": "gpt-4o-mini",
                "api_key": "sk-test",
            },
        },
    ])
    resp = await router.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "router hi"}],
    )
    assert resp.id == "chatcmpl-router"
    router_mock.assert_called_once()
    options.client.request_decision.assert_called_once()


@pytest.mark.asyncio
async def test_router_subclass_overriding_acompletion_patched(monkeypatch, options):
    """A Router subclass that overrides ``acompletion`` BEFORE install
    is also patched. Subclass uninstall restores the override."""

    class MyRouter(litellm.Router):
        async def acompletion(self, **kwargs):
            return SimpleNamespace(
                id="chatcmpl-subclass-mine",
                usage=SimpleNamespace(prompt_tokens=1, completion_tokens=1),
            )

    original_subclass_method = MyRouter.acompletion
    install_shim(options)
    # The subclass method is now the wrapper, not the original.
    assert MyRouter.acompletion is not original_subclass_method

    router = MyRouter(model_list=[
        {
            "model_name": "gpt-4o-mini",
            "litellm_params": {
                "model": "gpt-4o-mini",
                "api_key": "sk-test",
            },
        },
    ])
    resp = await router.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "subclass hi"}],
    )
    assert resp.id == "chatcmpl-subclass-mine"  # subclass override ran
    options.client.request_decision.assert_called_once()
    # Cleanup: uninstall restores subclass too.
    uninstall_shim()
    assert MyRouter.acompletion is original_subclass_method


@pytest.mark.asyncio
async def test_router_subclass_inheriting_picks_up_patched_parent(
    monkeypatch, options,
):
    """A Router subclass that does NOT override ``acompletion`` inherits
    the patched parent via MRO — no per-subclass walk needed."""
    router_mock = AsyncMock(return_value=SimpleNamespace(
        id="chatcmpl-inherited",
        usage=SimpleNamespace(prompt_tokens=2, completion_tokens=3),
    ))
    monkeypatch.setattr(litellm.Router, "acompletion", router_mock)

    class InheritingRouter(litellm.Router):
        # No acompletion override.
        pass

    install_shim(options)
    router = InheritingRouter(model_list=[
        {
            "model_name": "gpt-4o-mini",
            "litellm_params": {
                "model": "gpt-4o-mini",
                "api_key": "sk-test",
            },
        },
    ])
    resp = await router.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "inh hi"}],
    )
    assert resp.id == "chatcmpl-inherited"
    options.client.request_decision.assert_called_once()


# ---------------------------------------------------------------------------
# Module hygiene
# ---------------------------------------------------------------------------


def test_in_flight_default_false():
    """The recursion guard ContextVar starts ``False`` for any new
    asyncio task (production callers MUST never observe it true)."""
    assert _IN_FLIGHT.get() is False


def test_options_rejects_missing_tenant():
    """``SpendGuardShimOptions`` refuses empty ``tenant_id`` — surfaces
    a misconfiguration loudly at install time, not at first request."""
    with pytest.raises(ValueError, match="tenant_id"):
        SpendGuardShimOptions(client=_fake_client(), tenant_id="")
