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

from litellm.integrations.custom_logger import CustomLogger  # noqa: E402

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
async def test_failure_hook_no_stash_silent_noop():
    """Slice 5 shipped: async_log_failure_event with no stash is a
    silent no-op (pre-call hook never fired → no reservation to
    release). Full Slice 5 coverage in test_litellm_failure_unit.py."""
    cb = SpendGuardLiteLLMCallback(
        client=None,
        budget_resolver=lambda ctx: None,
        claim_estimator=lambda ctx: [],
        claim_reconciler=lambda ctx, resp: [],
    )
    # Must NOT raise even with client=None — silent no-op contract.
    await cb.async_log_failure_event(
        {"litellm_call_id": "no-stash"}, None, None, None,
    )


def test_no_log_pre_api_call_override():
    """Slice 1 R2 (2026-05-20): the prior `log_pre_api_call` override
    was verified ineffective against LiteLLM's logging dispatcher
    (exceptions swallowed at litellm_logging.py:45887). Confirm the
    SpendGuard callback does NOT override `log_pre_api_call` — relying
    on it would silently fail-open. Sync callers route to Shape A
    egress proxy per DESIGN §3.4 v1 Path A."""
    # CustomLogger ships a default no-op log_pre_api_call. The
    # SpendGuardLiteLLMCallback subclass MUST NOT override it.
    base_method = CustomLogger.log_pre_api_call
    sg_method = SpendGuardLiteLLMCallback.log_pre_api_call
    assert sg_method is base_method, (
        "SpendGuardLiteLLMCallback.log_pre_api_call MUST NOT be "
        "overridden — Slice 1 R2 verified raising from it is "
        "ineffective. See DESIGN.md ADR-005 revised."
    )


def test_install_factory_removed_in_pivot():
    """Pivot R1 P0.2: `install()` was removed because direct
    `litellm.callbacks=[...]` registration was verified ineffective
    (Slice 1 R2). Confirm the module does NOT export `install`."""
    import spendguard.integrations.litellm as mod
    assert "install" not in mod.__all__
    assert not hasattr(mod, "install"), (
        "install() factory must NOT exist in v1; pivot disabled it"
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
        "log",  # module logger (Slice 2)
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
async def test_loop_bound_callback_ensure_client_retries_then_fails(monkeypatch):
    """Slice 2 implementation: _ensure_client retries 5× with
    exponential backoff. With an unreachable socket and asyncio.sleep
    monkey-patched to no-op, the 5 attempts surface
    SidecarUnavailable. Pivot R1 P1.6 startup-race contract."""
    from spendguard.errors import SidecarUnavailable

    async def _instant_sleep(_) -> None:
        return None
    monkeypatch.setattr("asyncio.sleep", _instant_sleep)

    cb = _LoopBoundCallback(
        socket_path="/tmp/nonexistent-sock-spendguard-test",
        tenant_id="t1",
        budget_resolver=lambda ctx: None,
        claim_estimator=lambda ctx: [],
        claim_reconciler=lambda ctx, resp: [],
    )
    with pytest.raises(SidecarUnavailable, match=r"deadline.*5 attempts"):
        await cb._ensure_client()


@pytest.mark.asyncio
async def test_loop_bound_callback_async_hooks_call_ensure_client_first(monkeypatch):
    """Round 1 P1 fix (Slice 1): _LoopBoundCallback's async hook
    overrides MUST call _ensure_client() BEFORE delegating to
    super(). With unreachable socket + instant retry, all three
    async hooks surface SidecarUnavailable rather than reaching the
    parent body (which would surface NotImplementedError or attempt
    a real call)."""
    from spendguard.errors import SidecarUnavailable

    async def _instant_sleep(_) -> None:
        return None
    monkeypatch.setattr("asyncio.sleep", _instant_sleep)

    cb = _LoopBoundCallback(
        socket_path="/tmp/nonexistent-sock-spendguard-test",
        tenant_id="t1",
        budget_resolver=lambda ctx: None,
        claim_estimator=lambda ctx: [],
        claim_reconciler=lambda ctx, resp: [],
    )
    for hook, sample_args in [
        (cb.async_pre_call_hook, (None, None, {}, "acompletion")),
        (cb.async_log_success_event, ({}, None, None, None)),
        (cb.async_log_failure_event, ({}, None, None, None)),
    ]:
        with pytest.raises(SidecarUnavailable):
            await hook(*sample_args)
