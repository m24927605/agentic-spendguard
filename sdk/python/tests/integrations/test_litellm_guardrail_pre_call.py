# ruff: noqa: ANN001, ANN201, ANN202, ANN401, S106
# Rationale: test fixtures use ``monkeypatch`` (Any) + non-secret literal
# tokens; the test never speaks to an actual sidecar / LLM provider.
"""COV_D11_S2 — ``SpendGuardGuardrail.async_pre_call_hook`` wiring tests.

Tier 1 unit tests per ``docs/slices/COV_D11_S2_pre_call.md`` test plan
+ ``docs/specs/coverage/D11_litellm_proxy_plugin/review-standards.md``
§Slice 2 reviewer checklist (2.1 - 2.4).

Strategy:
    The guardrail hook is PURE DELEGATION to
    ``_LoopBoundCallback.async_pre_call_hook`` (review-standards 2.1
    Blocker: < 5 LOC body, no new error handling, no return mutation).
    Tests therefore swap ``g._delegate`` with an ``AsyncMock`` and
    assert:

        ALLOW  - delegate returns dict → hook returns identical dict.
        DENY   - delegate raises ``DecisionDenied`` → hook re-raises
                 unchanged (preserves ``status_code = 403`` class attr
                 so LiteLLM proxy maps to HTTP 403; see
                 ``errors.py`` L53).
        DEGRADE- delegate raises ``SidecarUnavailable`` (the wire-level
                 DEGRADE translation done inside the delegate at
                 ``litellm.py`` L418-429) → hook re-raises unchanged
                 (``status_code = 503`` → HTTP 503).
        UNKNOWN- delegate raises arbitrary ``RuntimeError`` → hook
                 re-raises (fail-closed: no ``except`` swallowing).

    User-prompt-noted edge cases ("missing model", "missing messages",
    "empty messages") are reduced to "the hook never inspects ``data``
    on its own; only the delegate does". The tests assert the hook
    forwards each ``data`` shape verbatim and lets the delegate's
    existing validation own the semantic check. This keeps
    review-standards 2.3 (Blocker: no ``data`` mutation) bright-line.

Anti-scope:
    * No commit/release wiring tests — SLICE 3.
    * No env-driven factory tests — SLICE 4.
    * No real sidecar / no real LiteLLM proxy boot.
"""

from __future__ import annotations

import inspect
from typing import Any
from unittest.mock import AsyncMock

import pytest

# Skip cleanly when LiteLLM (and therefore CustomGuardrail) is missing,
# mirroring the SLICE 1 skeleton test gate. ``BadRequestError`` is
# pre-imported per the slice doc instruction even though the wired hook
# does NOT translate exceptions (per design.md §7 + review-standards
# 2.1/2.2/2.3 pure-delegation contract) — keeping the importorskip
# means the test module fails fast on a LiteLLM that's too old to
# ship the exception class the slice doc references.
pytest.importorskip(
    "litellm.integrations.custom_guardrail",
    reason="LiteLLM with guardrail support not installed; "
    "install spendguard-sdk[litellm-guardrail]",
)
pytest.importorskip(
    "litellm.exceptions",
    reason="LiteLLM exceptions module not available",
)

from litellm.exceptions import BadRequestError  # noqa: E402, F401  # noqa  pre-imported per slice doc

from spendguard.errors import (  # noqa: E402
    DecisionDenied,
    SidecarUnavailable,
    SpendGuardConfigError,
)
from spendguard.integrations.litellm_guardrail import SpendGuardGuardrail  # noqa: E402


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_guardrail_with_mock_delegate(
    *,
    delegate_return: Any | None = None,
    delegate_side_effect: Any | None = None,
) -> tuple[SpendGuardGuardrail, AsyncMock]:
    """Build a ``SpendGuardGuardrail`` and swap its ``_delegate`` with
    an ``AsyncMock`` configured per the call site.

    Returns the guardrail + the mock so individual tests can also
    inspect ``await_args`` / ``await_count``.
    """
    g = SpendGuardGuardrail(guardrail_name="test")
    mock_delegate = AsyncMock()
    if delegate_side_effect is not None:
        mock_delegate.async_pre_call_hook.side_effect = delegate_side_effect
    else:
        mock_delegate.async_pre_call_hook.return_value = delegate_return
    g._delegate = mock_delegate  # type: ignore[assignment]
    return g, mock_delegate


def _baseline_data() -> dict[str, Any]:
    """Canonical LiteLLM proxy ``data`` dict shape — minimal but valid.
    The hook's contract is to forward this verbatim; tests assert the
    object identity is preserved across the delegate boundary."""
    return {
        "litellm_call_id": "call-pre-call-test",
        "model": "gpt-4o-mini",
        "messages": [{"role": "user", "content": "hi"}],
    }


# ---------------------------------------------------------------------------
# ALLOW path (reviewer check 2.3: return value verbatim from delegate)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_pre_call_allow_returns_delegate_data_unchanged():
    """ALLOW: delegate returns the same ``data`` dict on success;
    guardrail forwards it verbatim. Asserts object identity (the same
    dict, not just equal content) to pin review-standards 2.3."""
    data = _baseline_data()
    g, mock_delegate = _make_guardrail_with_mock_delegate(
        delegate_return=data,
    )
    result = await g.async_pre_call_hook(None, None, data, "acompletion")
    assert result is data, (
        "Hook MUST forward the delegate's return value verbatim "
        "(review-standards 2.3 Blocker — no data mutation)."
    )
    mock_delegate.async_pre_call_hook.assert_awaited_once_with(
        None, None, data, "acompletion",
    )


@pytest.mark.asyncio
async def test_pre_call_allow_with_none_return_propagates_none():
    """LiteLLM treats a ``None`` return as 'no data mutation', so the
    hook must forward ``None`` from the delegate verbatim — never
    coerce to a dict, never substitute the input ``data``."""
    data = _baseline_data()
    g, _ = _make_guardrail_with_mock_delegate(delegate_return=None)
    result = await g.async_pre_call_hook(None, None, data, "acompletion")
    assert result is None


# ---------------------------------------------------------------------------
# DENY path (reviewer check 2.2: DecisionDenied propagates)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_pre_call_deny_propagates_decision_denied():
    """DENY: delegate raises ``DecisionDenied``; hook re-raises the
    exact same exception object (no wrapping, no translation).

    Why no ``BadRequestError`` translation: ``DecisionDenied`` already
    carries ``status_code = 403`` (``errors.py`` L53) and LiteLLM's
    proxy maps it to HTTP 403 via ``getattr(exc, 'status_code', 500)``.
    Translating to ``BadRequestError(code=429)`` would silently swap
    403 → 429 — wrong HTTP semantics for an authoritative policy
    denial. Spec design.md §7 + review-standards 2.1/2.2 lock the
    pure-delegation choice.
    """
    deny = DecisionDenied(
        "budget exceeded",
        decision_id="dec-deny-1",
        reason_codes=["budget_exhausted"],
    )
    g, _ = _make_guardrail_with_mock_delegate(delegate_side_effect=deny)
    with pytest.raises(DecisionDenied) as exc_info:
        await g.async_pre_call_hook(None, None, _baseline_data(), "acompletion")
    assert exc_info.value is deny, "Identity preserved — no wrapping"
    assert exc_info.value.decision_id == "dec-deny-1"
    assert exc_info.value.reason_codes == ["budget_exhausted"]
    # LiteLLM proxy reads this class attribute to map to HTTP 403.
    assert exc_info.value.status_code == 403


@pytest.mark.asyncio
async def test_pre_call_deny_status_code_403_for_litellm_proxy_mapping():
    """Pin the contract: ``DecisionDenied.status_code`` survives the
    delegation boundary unchanged. This is what LiteLLM's proxy
    queries via ``getattr(exc, 'status_code', 500)`` to convert the
    callback exception into an HTTP response.
    """
    g, _ = _make_guardrail_with_mock_delegate(
        delegate_side_effect=DecisionDenied("x", decision_id="d"),
    )
    with pytest.raises(DecisionDenied) as exc_info:
        await g.async_pre_call_hook(None, None, _baseline_data(), "acompletion")
    assert exc_info.value.status_code == 403


# ---------------------------------------------------------------------------
# DEGRADE path (reviewer check 2.2: SidecarUnavailable propagates)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_pre_call_degrade_propagates_sidecar_unavailable():
    """DEGRADE: the delegate's ``_LoopBoundCallback`` already translates
    a sidecar DEGRADE outcome into ``SidecarUnavailable`` at
    ``litellm.py`` L418-429 (DESIGN §5 ledger-down row, fail-closed).
    The guardrail wrapper re-raises that exception verbatim — never
    coerces to a dict-with-mutated-model and NEVER returns ``data``.

    Note: the slice prompt mentions ``SpendGuardDegradeError(replacement_model=...)``
    but no such exception exists in the codebase (verified via grep).
    The actual DEGRADE-to-exception translation already happens in the
    delegate; the wrapper's job is pure propagation.
    """
    deg = SidecarUnavailable(
        "sidecar returned DEGRADE (ledger or dependent service "
        "unavailable); LiteLLM proxy fails closed on DEGRADE."
    )
    g, _ = _make_guardrail_with_mock_delegate(delegate_side_effect=deg)
    with pytest.raises(SidecarUnavailable) as exc_info:
        await g.async_pre_call_hook(None, None, _baseline_data(), "acompletion")
    assert exc_info.value is deg
    # status_code class attr = 503 → LiteLLM proxy returns HTTP 503.
    assert exc_info.value.status_code == 503


# ---------------------------------------------------------------------------
# UNKNOWN exception path (reviewer check 2.2: no swallowing)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_pre_call_unknown_exception_propagates_unchanged():
    """UNKNOWN: a raw ``RuntimeError`` from inside the delegate (e.g.
    a programmer bug) propagates unchanged — fail-closed posture, no
    ``except`` swallowing. Review-standards 2.2 Blocker."""

    class _DemoBug(RuntimeError):
        pass

    bug = _DemoBug("simulated programmer bug")
    g, _ = _make_guardrail_with_mock_delegate(delegate_side_effect=bug)
    with pytest.raises(_DemoBug) as exc_info:
        await g.async_pre_call_hook(None, None, _baseline_data(), "acompletion")
    assert exc_info.value is bug, "Identity preserved — no wrapping"


@pytest.mark.asyncio
async def test_pre_call_config_error_propagates_unchanged():
    """``SpendGuardConfigError`` (e.g. resolver returned None, missing
    env var) is an operator misconfiguration, NOT a transient infra
    failure. It must propagate unchanged so the LiteLLM proxy returns
    HTTP 500 (the default) loudly — silent fail-open here would
    mask boot-time config bugs in production."""
    err = SpendGuardConfigError(
        "budget_resolver returned None; resolver MUST yield a "
        "BudgetBinding (DESIGN.md ADR-001)"
    )
    g, _ = _make_guardrail_with_mock_delegate(delegate_side_effect=err)
    with pytest.raises(SpendGuardConfigError) as exc_info:
        await g.async_pre_call_hook(None, None, _baseline_data(), "acompletion")
    assert exc_info.value is err


# ---------------------------------------------------------------------------
# Argument forwarding (reviewer check 2.1: pure delegation)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_pre_call_forwards_all_four_args_verbatim_to_delegate():
    """Pin review-standards 2.1: the hook MUST forward the four
    positional args (``user_api_key_dict``, ``cache``, ``data``,
    ``call_type``) to the delegate in the same order, without any
    transformation, filtering, or extra kwargs."""

    class _UAK:
        team_id = "team-fwd"

    class _Cache:
        marker = "cache-fwd"

    uak = _UAK()
    cache = _Cache()
    data = _baseline_data()
    g, mock_delegate = _make_guardrail_with_mock_delegate(delegate_return=data)
    await g.async_pre_call_hook(uak, cache, data, "embeddings")
    mock_delegate.async_pre_call_hook.assert_awaited_once_with(
        uak, cache, data, "embeddings",
    )
    # Verify identity of each forwarded arg (not just equality —
    # mutable args like ``data`` MUST not be defensive-copied).
    call_args = mock_delegate.async_pre_call_hook.await_args
    assert call_args.args[0] is uak
    assert call_args.args[1] is cache
    assert call_args.args[2] is data
    assert call_args.args[3] == "embeddings"


# ---------------------------------------------------------------------------
# Data-shape edge cases (per slice doc test plan)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_pre_call_with_missing_model_key_forwards_verbatim():
    """Edge case: ``data`` arrives without a ``model`` key (caller bug
    or non-completion call_type). The hook does NOT inspect ``data``
    itself — that's the delegate's job — so it forwards verbatim and
    lets the delegate's resolver / estimator raise the right
    ``SpendGuardConfigError`` if needed."""
    data_no_model = {
        "litellm_call_id": "call-no-model",
        "messages": [{"role": "user", "content": "hi"}],
        # NB: no "model" key.
    }
    g, mock_delegate = _make_guardrail_with_mock_delegate(
        delegate_return=data_no_model,
    )
    result = await g.async_pre_call_hook(
        None, None, data_no_model, "acompletion",
    )
    assert result is data_no_model
    # Delegate received the same dict — wrapper did not inject a model.
    mock_delegate.async_pre_call_hook.assert_awaited_once_with(
        None, None, data_no_model, "acompletion",
    )
    assert "model" not in data_no_model, "Hook MUST NOT mutate data"


@pytest.mark.asyncio
async def test_pre_call_with_missing_messages_key_forwards_verbatim():
    """Edge case: ``data`` arrives without a ``messages`` key (e.g.
    ``embeddings`` call type). Same contract: forward verbatim,
    delegate owns prompt-hash + decision_context construction."""
    data_no_messages = {
        "litellm_call_id": "call-no-messages",
        "model": "text-embedding-3-small",
        "input": ["hello world"],  # embeddings shape
    }
    g, mock_delegate = _make_guardrail_with_mock_delegate(
        delegate_return=data_no_messages,
    )
    result = await g.async_pre_call_hook(
        None, None, data_no_messages, "embeddings",
    )
    assert result is data_no_messages
    mock_delegate.async_pre_call_hook.assert_awaited_once_with(
        None, None, data_no_messages, "embeddings",
    )
    assert "messages" not in data_no_messages


@pytest.mark.asyncio
async def test_pre_call_with_empty_messages_forwards_verbatim():
    """Edge case: ``messages=[]`` is technically valid (provider may
    reject, but the gate does not). The delegate's ``_serialize_messages_for_hash``
    handles ``[]`` cleanly (litellm.py L196-198). Wrapper must not
    short-circuit on the empty list."""
    data_empty = {
        "litellm_call_id": "call-empty-messages",
        "model": "gpt-4o-mini",
        "messages": [],  # empty but present
    }
    g, mock_delegate = _make_guardrail_with_mock_delegate(
        delegate_return=data_empty,
    )
    result = await g.async_pre_call_hook(
        None, None, data_empty, "acompletion",
    )
    assert result is data_empty
    mock_delegate.async_pre_call_hook.assert_awaited_once_with(
        None, None, data_empty, "acompletion",
    )


# ---------------------------------------------------------------------------
# Source-level invariants (review-standards 2.1 — body < 5 LOC)
# ---------------------------------------------------------------------------


def test_pre_call_hook_body_is_pure_delegation():
    """Source-level guard: ``async_pre_call_hook`` body is fewer than
    5 LOC excluding signature + docstring. Locks review-standards 2.1
    Blocker against drive-by refactors that add error handling /
    translation logic to the wrapper.
    """
    src = inspect.getsource(SpendGuardGuardrail.async_pre_call_hook)
    # Strip the signature lines + docstring.
    lines = [ln for ln in src.splitlines() if ln.strip()]
    # Find the closing triple-quote of the docstring.
    in_doc = False
    body_lines: list[str] = []
    for ln in lines:
        s = ln.strip()
        if s.startswith('"""') or s.startswith("'''"):
            # Toggle docstring on/off; a same-line ``"""..."""`` opens
            # AND closes on the same line.
            quote = s[:3]
            count = s.count(quote)
            if not in_doc:
                in_doc = True
                if count >= 2:
                    in_doc = False
            else:
                in_doc = False
            continue
        if in_doc:
            continue
        # Skip signature lines (``async def`` header through the
        # closing ``) -> ... :`` line).
        if s.startswith("async def "):
            continue
        if s.startswith(("self,", "user_api_key_dict", "cache", "data", "call_type")):
            continue
        if s.startswith(") -> "):
            continue
        body_lines.append(s)
    # ``return await self._delegate.async_pre_call_hook(...)`` is
    # typically formatted as 2-3 lines after black; allow ≤ 4 to leave
    # headroom for trailing commas / line-wrap, well under the 5-LOC
    # Blocker bar from review-standards 2.1.
    assert len(body_lines) <= 4, (
        f"Body must be < 5 LOC (review-standards 2.1); got "
        f"{len(body_lines)}:\n" + "\n".join(body_lines)
    )
    # The single statement is exactly the delegate call.
    body_text = " ".join(body_lines)
    assert "self._delegate.async_pre_call_hook" in body_text, (
        "Body MUST delegate to self._delegate.async_pre_call_hook"
    )
    assert "return await" in body_text, (
        "Body MUST be a `return await ...` statement (no swallowing)"
    )


def test_pre_call_hook_has_no_try_except():
    """Source-level guard: review-standards 2.2 Blocker forbids any
    ``except`` clause in the hook body. The delegate owns all
    exception translation; the wrapper is pure forwarding."""
    src = inspect.getsource(SpendGuardGuardrail.async_pre_call_hook)
    # Remove docstring before scanning so prose mentioning the word
    # 'except' in a comment does not false-positive.
    lines = src.splitlines()
    in_doc = False
    scan_lines: list[str] = []
    for ln in lines:
        s = ln.strip()
        if s.startswith('"""') or s.startswith("'''"):
            quote = s[:3]
            count = s.count(quote)
            if not in_doc:
                in_doc = True
                if count >= 2:
                    in_doc = False
            else:
                in_doc = False
            continue
        if in_doc:
            continue
        scan_lines.append(ln)
    body = "\n".join(scan_lines)
    assert "try:" not in body, (
        "Hook body MUST NOT contain a try block (review-standards 2.2 "
        "Blocker — no error swallowing)"
    )
    assert "except " not in body and "except:" not in body, (
        "Hook body MUST NOT contain an except clause (review-standards "
        "2.2 Blocker — no error swallowing)"
    )


# ---------------------------------------------------------------------------
# Other hooks remain stubbed (SLICE 3 scope-fence)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_post_call_success_hook_still_raises_not_implemented():
    """Scope-fence: SLICE 2 wires ONLY async_pre_call_hook. The two
    post-call hooks MUST remain ``NotImplementedError`` stubs pointing
    at SLICE 3. This pins the slice boundary per the slice doc's
    'Anti-scope' section."""
    g = SpendGuardGuardrail(guardrail_name="test")
    with pytest.raises(NotImplementedError, match="COV_D11_S3"):
        await g.async_post_call_success_hook({}, None, None)


@pytest.mark.asyncio
async def test_post_call_failure_hook_still_raises_not_implemented():
    """Scope-fence sibling: failure hook MUST also remain a SLICE 3 stub."""
    g = SpendGuardGuardrail(guardrail_name="test")
    with pytest.raises(NotImplementedError, match="COV_D11_S3"):
        await g.async_post_call_failure_hook(
            {}, RuntimeError("x"), None,
        )
