# ruff: noqa: ANN001, ANN201, ANN401, S108
# Rationale: test fixtures use `monkeypatch` (Any), lambda resolvers
# returning None, and /tmp paths which are appropriate for tests.
"""Slice 1 SDK skeleton — Tier 1 unit tests per TEST_PLAN.md §2.1."""

from __future__ import annotations

import importlib
import re
from dataclasses import FrozenInstanceError
from pathlib import Path

import pytest

# Skip the whole module cleanly when litellm is not installed.
litellm = pytest.importorskip(
    "litellm.integrations.custom_logger",
    reason="LiteLLM not installed; install spendguard-sdk[litellm]",
)

from spendguard.errors import SpendGuardConfigError  # noqa: E402
from spendguard.integrations.litellm import (  # noqa: E402
    BudgetBinding,
    BudgetResolver,
    ClaimEstimator,
    ClaimReconciler,
    LiteLLMRunContext,
    ResolverContext,
    SpendGuardLiteLLMCallback,
    _LoopBoundCallback,
    current_run_context,
    install,
    run_context,
)


def _module_source() -> str:
    return Path(
        importlib.import_module(
            "spendguard.integrations.litellm"
        ).__file__
    ).read_text(encoding="utf-8")


def test_module_imports_with_litellm_installed():
    """Happy path; __all__ contains every DESIGN.md §6 public symbol."""
    import spendguard.integrations.litellm as mod

    expected = {
        "BudgetBinding",
        "BudgetResolver",
        "ClaimEstimator",
        "ClaimReconciler",
        "LiteLLMRunContext",
        "ResolverContext",
        "SpendGuardLiteLLMCallback",
        "_LoopBoundCallback",
        "current_run_context",
        "install",
        "run_context",
    }
    assert set(mod.__all__) == expected


def test_litellm_run_context_is_frozen_slots():
    ctx = LiteLLMRunContext(run_id="r1")
    with pytest.raises(FrozenInstanceError):
        ctx.run_id = "r2"  # type: ignore[misc]
    # slots → no __dict__
    assert not hasattr(ctx, "__dict__")


def test_resolver_context_dataclass_shape():
    """ResolverContext is frozen + slotted with the 3 documented fields."""
    rctx = ResolverContext(
        data={"model": "gpt-4o-mini"},
        user_api_key_dict=None,
        call_type="acompletion",
    )
    with pytest.raises(FrozenInstanceError):
        rctx.call_type = "aembedding"  # type: ignore[misc]
    assert not hasattr(rctx, "__dict__")
    assert rctx.data == {"model": "gpt-4o-mini"}
    assert rctx.user_api_key_dict is None
    assert rctx.call_type == "acompletion"


def test_budget_binding_dataclass_shape():
    b = BudgetBinding(
        budget_id="b1",
        window_instance_id="w1",
        unit="sentinel-unit",
        pricing="sentinel-pricing",
    )
    with pytest.raises(FrozenInstanceError):
        b.budget_id = "b2"  # type: ignore[misc]
    assert not hasattr(b, "__dict__")


@pytest.mark.asyncio
async def test_run_context_async_cm_set_get_reset():
    assert current_run_context() is None
    async with run_context(LiteLLMRunContext(run_id="r1", step_id="s1")):
        got = current_run_context()
        assert got is not None
        assert got.run_id == "r1" and got.step_id == "s1"
    assert current_run_context() is None


@pytest.mark.asyncio
async def test_async_hooks_raise_notimplementederror_per_slice():
    cb = SpendGuardLiteLLMCallback(
        client=None,
        budget_resolver=lambda ctx: None,
        claim_estimator=lambda ctx: [],
        claim_reconciler=lambda ctx, resp: [],
    )
    with pytest.raises(NotImplementedError, match="Slice 2"):
        await cb.async_pre_call_hook(None, None, {}, "acompletion")
    with pytest.raises(NotImplementedError, match=r"Slice 3 / Slice 4"):
        await cb.async_log_success_event({}, None, None, None)
    with pytest.raises(NotImplementedError, match="Slice 5"):
        await cb.async_log_failure_event({}, None, None, None)


def test_sync_log_pre_api_call_fails_closed():
    """Round 2 P0.7 / ADR-005: sync pre-wire hook MUST raise loudly
    before the wire so sync litellm.completion() never bypasses
    enforcement."""
    cb = SpendGuardLiteLLMCallback(
        client=None,
        budget_resolver=lambda ctx: None,
        claim_estimator=lambda ctx: [],
        claim_reconciler=lambda ctx, resp: [],
    )
    with pytest.raises(SpendGuardConfigError, match="Sync"):
        cb.log_pre_api_call(model="gpt-4o-mini", messages=[], kwargs={})


def test_install_raises_in_slice_1():
    """install() body lands in Slice 2."""
    with pytest.raises(NotImplementedError, match="Slice 2"):
        install(
            client=None,  # type: ignore[arg-type]
            budget_resolver=lambda ctx: None,
            claim_estimator=lambda ctx: [],
            claim_reconciler=lambda ctx, resp: [],
        )


def test_loop_bound_callback_is_subclass():
    """_LoopBoundCallback is exported and subclasses
    SpendGuardLiteLLMCallback (Round 3 P0.3 fix)."""
    assert issubclass(_LoopBoundCallback, SpendGuardLiteLLMCallback)
    # Class instantiation does not require an event loop.
    cb = _LoopBoundCallback(
        socket_path="/tmp/x",
        tenant_id="t1",
        budget_resolver=lambda ctx: None,
        claim_estimator=lambda ctx: [],
        claim_reconciler=lambda ctx, resp: [],
    )
    assert cb._client is None
    assert cb._socket_path == "/tmp/x"
    assert cb._tenant_id == "t1"


def test_type_aliases_exposed():
    """BudgetResolver / ClaimEstimator / ClaimReconciler are callable
    type aliases (typing.Callable). Asserts they are imported."""
    # Just confirm the names resolved at import time (already imported).
    assert BudgetResolver is not None
    assert ClaimEstimator is not None
    assert ClaimReconciler is not None


def test_no_default_budget_env_var():
    """Round 2 P0.10: SPENDGUARD_LITELLM_DEFAULT_BUDGET_ID was REMOVED.
    Confirm module source does not read or reference it."""
    src = _module_source()
    assert "SPENDGUARD_LITELLM_DEFAULT_BUDGET_ID" not in src


def test_no_provider_api_key_handling():
    """S3: SDK MUST NOT handle/log/transport provider keys. Confirm
    no references in the module source."""
    src = _module_source()
    forbidden = [
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "GEMINI_API_KEY",
        "BEDROCK_API_KEY",
        "AZURE_API_KEY",
    ]
    for name in forbidden:
        assert name not in src, (
            f"provider key reference forbidden in SDK: {name}"
        )


def test_module_level_mutable_state_scan():
    """NF4: only _RUN_CONTEXT ContextVar is permitted module-level
    mutable state (parity with agt.py precedent)."""
    src = _module_source()
    # Find all module-level top-level assignments. Simple regex: a
    # name = value pattern at column 0 (not indented).
    pattern = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)\s*[:=]", re.MULTILINE)
    names = pattern.findall(src)
    # Allowed module-level names: ContextVar, type aliases, __all__,
    # and the dataclass / function definitions themselves (filtered
    # via inspection in a separate check).
    allowed = {
        "_RUN_CONTEXT",
        "BudgetResolver",  # type alias
        "ClaimEstimator",
        "ClaimReconciler",
        "__all__",
    }
    # Anything else that's an assignment must be a class/def line,
    # which the regex won't match (those start with class/def/async).
    # So just assert that any captured top-level "= value" lines are
    # in the allowed set.
    unexpected = set(names) - allowed - {
        # The dataclass field annotations within @dataclass bodies are
        # indented; not matched. But the regex picks up `try:` etc as
        # `try`. Filter Python keywords.
        "from", "import", "try", "if", "else", "with", "return", "raise",
        "for", "while", "def", "class", "async", "await", "yield",
        "pass", "break", "continue", "assert", "global", "nonlocal",
    }
    assert unexpected == set(), (
        f"unexpected module-level state: {unexpected}"
    )


@pytest.mark.asyncio
async def test_loop_bound_callback_ensure_client_stub():
    """_LoopBoundCallback._ensure_client is a Slice 1 stub; Slice 2
    fills in the handshake."""
    cb = _LoopBoundCallback(
        socket_path="/tmp/x",
        tenant_id="t1",
        budget_resolver=lambda ctx: None,
        claim_estimator=lambda ctx: [],
        claim_reconciler=lambda ctx, resp: [],
    )
    with pytest.raises(NotImplementedError, match="Slice 2"):
        await cb._ensure_client()


@pytest.mark.asyncio
async def test_loop_bound_callback_async_hooks_call_ensure_client_first():
    """Round 1 P1 fix: _LoopBoundCallback's async hook overrides MUST
    call _ensure_client() BEFORE delegating to super(), so the
    event-loop-affinity binding is locked in regardless of which
    Slice (2-5) is filling the super body."""
    cb = _LoopBoundCallback(
        socket_path="/tmp/x",
        tenant_id="t1",
        budget_resolver=lambda ctx: None,
        claim_estimator=lambda ctx: [],
        claim_reconciler=lambda ctx, resp: [],
    )
    # _ensure_client raises NotImplementedError("Slice 2 wires ...")
    # in Slice 1; each async hook MUST surface that BEFORE the parent's
    # NotImplementedError("Slice 2"/"Slice 3"/...) would have fired.
    for hook, sample_args in [
        (cb.async_pre_call_hook, (None, None, {}, "acompletion")),
        (cb.async_log_success_event, ({}, None, None, None)),
        (cb.async_log_failure_event, ({}, None, None, None)),
    ]:
        with pytest.raises(
            NotImplementedError, match="_LoopBoundCallback handshake"
        ):
            await hook(*sample_args)
