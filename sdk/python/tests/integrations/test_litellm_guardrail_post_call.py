# ruff: noqa: ANN001, ANN201, ANN202, ANN401, S106
# Rationale: test fixtures use ``monkeypatch`` (Any) + non-secret literal
# tokens; the test never speaks to an actual sidecar / LLM provider.
"""COV_D11_S3 — ``SpendGuardGuardrail`` post-call hook wiring tests.

Tier 1 unit tests per ``docs/slices/COV_D11_S3_commit_release.md`` test
plan + ``docs/specs/coverage/D11_litellm_proxy_plugin/review-standards.md``
§Slice 3 reviewer checklist (3.1 - 3.5).

Strategy:
    Both post-call hooks are PURE DELEGATION + signature translation
    to ``_LoopBoundCallback.async_log_success_event`` /
    ``async_log_failure_event``. Tests swap ``g._delegate`` with an
    ``AsyncMock`` and assert:

      Success commit (review-standards 3.1, 3.3, 3.4):
        * kwargs dict carries ``litellm_call_id`` from ``data``.
        * kwargs dict carries ``user_api_key_dict``.
        * ``response`` is forwarded verbatim as ``response_obj`` so
          the delegate's reconciler / streaming-fallback path runs.
        * Missing ``litellm_call_id`` → delegate's ``_get_stash`` is
          contracted to no-op silently (not a wrapper concern).
        * Hook returns ``None``.
        * ``start_time`` / ``end_time`` propagate from ``data`` when
          present; default to ``None`` when absent.

      Failure release (review-standards 3.1, 3.2, 3.3):
        * kwargs dict carries ``litellm_call_id`` from
          ``request_data``.
        * ``kwargs["exception"] = original_exception`` populated so
          the delegate's ``_classify_failure`` sees the exception.
        * The wrapper re-raises ``original_exception`` after the
          delegate runs, so the LiteLLM proxy propagates the upstream
          HTTP error rather than swallowing it.

Anti-scope:
    * No real reserve flow tests — SLICE 2 owns the pre-call path.
    * No env-driven factory tests — SLICE 4.
    * No real sidecar / no real LiteLLM proxy boot.
"""

from __future__ import annotations

import asyncio
import inspect
from types import SimpleNamespace
from typing import Any
from unittest.mock import AsyncMock

import pytest

# Skip cleanly when LiteLLM (and therefore CustomGuardrail) is missing.
pytest.importorskip(
    "litellm.integrations.custom_guardrail",
    reason="LiteLLM with guardrail support not installed; "
    "install spendguard-sdk[litellm-guardrail]",
)

from spendguard.integrations.litellm_guardrail import SpendGuardGuardrail  # noqa: E402

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_guardrail_with_mock_delegate() -> tuple[SpendGuardGuardrail, AsyncMock]:
    """Build a ``SpendGuardGuardrail`` and swap its ``_delegate`` with
    an ``AsyncMock``. Default mock return for ``async_log_success_event``
    / ``async_log_failure_event`` is ``None`` (the delegate's actual
    contract — both methods return ``None`` per ``litellm.py`` L501,
    L674).
    """
    g = SpendGuardGuardrail(guardrail_name="test")
    mock_delegate = AsyncMock()
    mock_delegate.async_log_success_event.return_value = None
    mock_delegate.async_log_failure_event.return_value = None
    g._delegate = mock_delegate  # type: ignore[assignment]
    return g, mock_delegate


def _baseline_data() -> dict[str, Any]:
    """Canonical LiteLLM proxy ``data`` dict — minimal but carries
    the load-bearing ``litellm_call_id`` so the delegate's ``_get_stash``
    can find the SLICE 2 reservation entry."""
    return {
        "litellm_call_id": "call-post-call-test",
        "model": "gpt-4o-mini",
        "messages": [{"role": "user", "content": "hi"}],
    }


def _baseline_response() -> SimpleNamespace:
    """Canonical successful LiteLLM response with ``id`` + ``usage``.
    Mirrors the shape ``_LoopBoundCallback.async_log_success_event``
    reads (``response.usage.completion_tokens``, ``response.id``)."""
    return SimpleNamespace(
        id="chatcmpl-test",
        usage=SimpleNamespace(prompt_tokens=10, completion_tokens=42),
    )


# ===========================================================================
# SUCCESS HOOK — async_post_call_success_hook
# ===========================================================================


# ---------------------------------------------------------------------------
# Test 1: kwargs translation — litellm_call_id propagated to delegate
# (review-standards 3.1 Blocker)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_success_hook_propagates_litellm_call_id_into_delegate_kwargs():
    """Review-standards 3.1 (Blocker): ``kwargs["litellm_call_id"]``
    MUST be populated from ``data["litellm_call_id"]`` so the delegate's
    ``_get_stash`` (``litellm.py`` L482-483) finds the SLICE 2 reserve
    stash. Without this populate, every commit would silently no-op."""
    g, mock = _make_guardrail_with_mock_delegate()
    data = _baseline_data()
    response = _baseline_response()
    await g.async_post_call_success_hook(data, None, response)
    mock.async_log_success_event.assert_awaited_once()
    forwarded_kwargs = mock.async_log_success_event.await_args.args[0]
    assert forwarded_kwargs["litellm_call_id"] == "call-post-call-test", (
        "Delegate kwargs MUST carry litellm_call_id so _get_stash hits "
        "the SLICE 2 reservation entry."
    )


# ---------------------------------------------------------------------------
# Test 2: success hook forwards response verbatim + returns None
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_success_hook_forwards_response_verbatim_and_returns_none():
    """The ``response`` arg is forwarded to the delegate verbatim as
    ``response_obj`` (positional arg 1 of ``async_log_success_event``)
    so the reconciler reads ``response.usage`` and ``response.id``
    correctly. The hook itself returns ``None`` per LiteLLM's success
    hook contract."""
    g, mock = _make_guardrail_with_mock_delegate()
    response = _baseline_response()
    result = await g.async_post_call_success_hook(_baseline_data(), None, response)
    assert result is None, "Success hook must return None (LiteLLM contract)"
    # Positional args: (kwargs, response_obj, start_time, end_time).
    forwarded_response = mock.async_log_success_event.await_args.args[1]
    assert forwarded_response is response, (
        "Response forwarded by identity (not defensive-copied) so the "
        "reconciler sees provider-set ``usage`` / ``id`` fields."
    )


# ---------------------------------------------------------------------------
# Test 3: success hook populates user_api_key_dict in kwargs
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_success_hook_populates_user_api_key_dict_in_kwargs():
    """The delegate's commit path builds its resolver context from
    ``kwargs.get("user_api_key_dict")`` (``litellm.py`` L519). The
    wrapper MUST populate that key so the resolver can read team /
    tenant scope (matches the legacy CustomLogger surface)."""
    g, mock = _make_guardrail_with_mock_delegate()

    class _UAK:
        team_id = "team-post-call"

    uak = _UAK()
    await g.async_post_call_success_hook(_baseline_data(), uak, _baseline_response())
    forwarded_kwargs = mock.async_log_success_event.await_args.args[0]
    assert forwarded_kwargs["user_api_key_dict"] is uak


# ---------------------------------------------------------------------------
# Test 4: start_time / end_time propagation when present in data
# (review-standards 3.3 Major)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_success_hook_propagates_start_end_time_from_data():
    """Review-standards 3.3 (Major): when LiteLLM stamps timing fields
    on ``data``, the wrapper propagates them as positional args 2/3 to
    the delegate. The delegate's commit path doesn't currently consume
    these (commit reads ``response.usage``), but forward-compat
    requires the wrapper to plumb them rather than silently drop."""
    g, mock = _make_guardrail_with_mock_delegate()
    data = _baseline_data()
    data["start_time"] = 1000.0
    data["end_time"] = 1002.5
    await g.async_post_call_success_hook(data, None, _baseline_response())
    args = mock.async_log_success_event.await_args.args
    assert args[2] == 1000.0, "start_time MUST flow to positional arg 2"
    assert args[3] == 1002.5, "end_time MUST flow to positional arg 3"


@pytest.mark.asyncio
async def test_success_hook_defaults_timing_to_none_when_absent():
    """When ``data`` lacks timing fields (some LiteLLM versions do not
    stamp them), the wrapper falls back to ``None``. The delegate's
    commit path tolerates ``None`` because it reads ``response.usage``,
    not timestamps (review-standards 3.3 pin)."""
    g, mock = _make_guardrail_with_mock_delegate()
    data = _baseline_data()
    # Explicitly assert no timing keys — guard against test drift.
    assert "start_time" not in data
    assert "end_time" not in data
    await g.async_post_call_success_hook(data, None, _baseline_response())
    args = mock.async_log_success_event.await_args.args
    assert args[2] is None, "Missing start_time MUST default to None"
    assert args[3] is None, "Missing end_time MUST default to None"


# ---------------------------------------------------------------------------
# Test 5: data dict is NOT mutated by the wrapper (defensive copy)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_success_hook_does_not_mutate_caller_data():
    """Pin: ``dict(data)`` shallow copy → mutating ``kwargs`` inside the
    delegate path MUST NOT leak back to the caller's ``data`` dict.
    The LiteLLM proxy may re-use ``data`` for subsequent middleware,
    so silent mutation would corrupt downstream state."""
    g, mock = _make_guardrail_with_mock_delegate()
    data = _baseline_data()
    snapshot = dict(data)
    await g.async_post_call_success_hook(data, None, _baseline_response())
    assert data == snapshot, (
        "Hook MUST NOT mutate the caller's data dict (shallow-copy "
        "via dict(data) before forwarding)"
    )


# ---------------------------------------------------------------------------
# Test 6: missing litellm_call_id in data — silent no-op via delegate
# (review-standards 3.1 spec: delegate's _get_stash returns None → no
# exception)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_success_hook_missing_litellm_call_id_no_exception():
    """Review-standards 3.1 Blocker spec: when ``data`` lacks
    ``litellm_call_id`` the wrapper still forwards to the delegate
    (with ``None`` populated as the call id). The delegate's
    ``_get_stash`` returns ``None`` for missing call ids
    (``litellm.py`` L482-483) → silent no-op. The wrapper itself
    raises NO exception — the spec specifically locks 'no-op (no
    exception)' as the contract."""
    g, mock = _make_guardrail_with_mock_delegate()
    data = {"model": "gpt-4o-mini"}  # NB: no litellm_call_id
    # Must NOT raise — review-standards 3.1 contract.
    await g.async_post_call_success_hook(data, None, _baseline_response())
    # Delegate is still called (the wrapper doesn't short-circuit on
    # missing call id; the delegate handles the no-op).
    mock.async_log_success_event.assert_awaited_once()
    forwarded_kwargs = mock.async_log_success_event.await_args.args[0]
    # The call_id was None on the input → propagated as None to the
    # delegate. The delegate's _get_stash will return None → no-op.
    assert forwarded_kwargs["litellm_call_id"] is None


# ===========================================================================
# FAILURE HOOK — async_post_call_failure_hook
# ===========================================================================


# ---------------------------------------------------------------------------
# Test 7: kwargs translation — litellm_call_id from request_data
# (review-standards 3.1 Blocker)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_failure_hook_propagates_litellm_call_id_into_delegate_kwargs():
    """Review-standards 3.1 (Blocker, failure path): the failure hook
    MUST mirror the success hook's call-id propagation so the delegate's
    release path (``_get_stash`` at ``litellm.py`` L685) finds the
    reservation. Without this populate, every failure would silently
    no-op and the reservation would TTL-sweep instead of being
    explicitly released."""
    g, mock = _make_guardrail_with_mock_delegate()
    request_data = _baseline_data()
    err = RuntimeError("upstream 500")
    # Failure hook re-raises; use pytest.raises to consume the
    # propagation exception.
    with pytest.raises(RuntimeError, match="upstream 500"):
        await g.async_post_call_failure_hook(request_data, err, None)
    mock.async_log_failure_event.assert_awaited_once()
    forwarded_kwargs = mock.async_log_failure_event.await_args.args[0]
    assert forwarded_kwargs["litellm_call_id"] == "call-post-call-test"


# ---------------------------------------------------------------------------
# Test 8: kwargs translation — exception object populated
# (review-standards 3.2 Blocker)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_failure_hook_populates_exception_in_delegate_kwargs():
    """Review-standards 3.2 (Blocker): ``kwargs["exception"] =
    original_exception`` MUST be populated so the delegate's
    ``_classify_failure`` (``litellm.py`` L739-760) can correctly map
    ``asyncio.CancelledError`` → outcome CANCELLED. Missing this
    populate would silently misclassify every cancellation as
    outcome FAILURE."""
    g, mock = _make_guardrail_with_mock_delegate()
    cancel_err = asyncio.CancelledError()
    with pytest.raises(asyncio.CancelledError):
        await g.async_post_call_failure_hook(_baseline_data(), cancel_err, None)
    forwarded_kwargs = mock.async_log_failure_event.await_args.args[0]
    assert forwarded_kwargs["exception"] is cancel_err, (
        "Failure kwargs MUST carry the exception object so the "
        "delegate's _classify_failure (CancelledError → CANCELLED) "
        "branch fires."
    )


# ---------------------------------------------------------------------------
# Test 9: failure hook re-raises the original exception
# (slice-doc requirement: 'LiteLLM expects propagation')
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_failure_hook_reraises_original_exception_identity_preserved():
    """The failure hook re-raises ``original_exception`` after the
    delegate's release call so the LiteLLM proxy returns the upstream
    HTTP error rather than swallowing it. Identity preserved (not a
    wrapped exception)."""
    g, _ = _make_guardrail_with_mock_delegate()

    class _Provider429(Exception):
        status_code = 429

    err = _Provider429("provider rate limit")
    with pytest.raises(_Provider429) as exc_info:
        await g.async_post_call_failure_hook(_baseline_data(), err, None)
    assert exc_info.value is err, (
        "Failure hook MUST re-raise the exact same exception object — "
        "the LiteLLM proxy maps status_code on the original exception "
        "into the HTTP response."
    )


# ---------------------------------------------------------------------------
# Test 10: unknown exception type still flows through delegate + re-raise
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_failure_hook_unknown_exception_type_forwarded_and_reraised():
    """A bespoke exception type (not CancelledError, not LiteLLM's own
    set) still flows through the delegate (which classifies it as
    FAILURE per ``_classify_failure`` L760 default branch) and gets
    re-raised verbatim. No special-casing in the wrapper."""
    g, mock = _make_guardrail_with_mock_delegate()

    class _DemoCustomError(ValueError):
        pass

    err = _DemoCustomError("operator-defined failure")
    with pytest.raises(_DemoCustomError) as exc_info:
        await g.async_post_call_failure_hook(_baseline_data(), err, None)
    assert exc_info.value is err
    # Delegate received the exception object verbatim.
    forwarded_kwargs = mock.async_log_failure_event.await_args.args[0]
    assert forwarded_kwargs["exception"] is err


# ---------------------------------------------------------------------------
# Test 11: failure hook populates user_api_key_dict in kwargs
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_failure_hook_populates_user_api_key_dict_in_kwargs():
    """The delegate's release path also reads ``kwargs["user_api_key_dict"]``
    when building resolver context for logging (defensive parity with
    the success path). Wrapper populates the key the same way."""
    g, mock = _make_guardrail_with_mock_delegate()

    class _UAK:
        team_id = "team-failure-call"

    uak = _UAK()
    with pytest.raises(RuntimeError):
        await g.async_post_call_failure_hook(
            _baseline_data(), RuntimeError("x"), uak,
        )
    forwarded_kwargs = mock.async_log_failure_event.await_args.args[0]
    assert forwarded_kwargs["user_api_key_dict"] is uak


# ---------------------------------------------------------------------------
# Test 12: start_time / end_time propagation on failure path
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_failure_hook_propagates_start_end_time_from_request_data():
    """Symmetric to the success hook: timing fields stamped on
    ``request_data`` propagate as positional args 2/3 to
    ``async_log_failure_event`` (the delegate ignores them today
    but forward-compat plumbing keeps the wrapper honest)."""
    g, mock = _make_guardrail_with_mock_delegate()
    request_data = _baseline_data()
    request_data["start_time"] = 2000.0
    request_data["end_time"] = 2003.0
    with pytest.raises(RuntimeError):
        await g.async_post_call_failure_hook(
            request_data, RuntimeError("x"), None,
        )
    args = mock.async_log_failure_event.await_args.args
    assert args[2] == 2000.0
    assert args[3] == 2003.0


# ---------------------------------------------------------------------------
# Test 13: failure hook passes None as response_obj to delegate
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_failure_hook_passes_none_response_obj_to_delegate():
    """On the failure path there is no successful response object.
    The delegate's release branch (``litellm.py`` L719) reads
    ``getattr(response_obj, "id", "")`` which tolerates ``None`` —
    the wrapper plumbs ``None`` as the response_obj positional arg."""
    g, mock = _make_guardrail_with_mock_delegate()
    with pytest.raises(RuntimeError):
        await g.async_post_call_failure_hook(
            _baseline_data(), RuntimeError("x"), None,
        )
    response_obj_forwarded = mock.async_log_failure_event.await_args.args[1]
    assert response_obj_forwarded is None


# ---------------------------------------------------------------------------
# Test 14: failure hook does NOT mutate caller's request_data
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_failure_hook_does_not_mutate_caller_request_data():
    """Defensive shallow copy via ``dict(request_data)`` keeps the
    LiteLLM proxy's downstream middleware from seeing a mutated
    ``request_data`` (symmetric to the success hook test)."""
    g, _ = _make_guardrail_with_mock_delegate()
    request_data = _baseline_data()
    snapshot = dict(request_data)
    with pytest.raises(RuntimeError):
        await g.async_post_call_failure_hook(
            request_data, RuntimeError("x"), None,
        )
    assert request_data == snapshot


# ---------------------------------------------------------------------------
# Test 15: failure hook accepts optional traceback_str kwarg (1.55+ surface)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_failure_hook_accepts_traceback_str_optional_kwarg():
    """LiteLLM 1.55+ passes an optional ``traceback_str`` argument.
    The wrapper accepts it for forward-compat but does NOT plumb it
    into the delegate's kwargs (the delegate already has the exception
    object; traceback strings risk PII leakage into audit logs)."""
    g, mock = _make_guardrail_with_mock_delegate()
    with pytest.raises(RuntimeError):
        await g.async_post_call_failure_hook(
            _baseline_data(),
            RuntimeError("x"),
            None,
            traceback_str="File 'foo.py', line 42, in bar\n  raise RuntimeError",
        )
    forwarded_kwargs = mock.async_log_failure_event.await_args.args[0]
    # By design, traceback_str is NOT inserted into delegate kwargs
    # (no PII into the audit row).
    assert "traceback_str" not in forwarded_kwargs


# ===========================================================================
# SOURCE-LEVEL INVARIANTS (review-standards 3.5 + slice-doc constraints)
# ===========================================================================


# ---------------------------------------------------------------------------
# Test 16: source-level — neither post-call hook contains a try/except
# (no error swallowing; same rigor as SLICE 2 pre-call hook)
# ---------------------------------------------------------------------------


def test_post_call_hooks_have_no_try_except():
    """Source-level guard: neither post-call hook contains a ``try`` /
    ``except`` clause. Both paths are pure delegation + signature
    translation; the delegate owns all exception handling and its
    own release-RPC error swallowing. Adding a try/except in the
    wrapper would either double-swallow or mask the delegate's
    explicit semantics."""
    for method_name in (
        "async_post_call_success_hook",
        "async_post_call_failure_hook",
    ):
        method = getattr(SpendGuardGuardrail, method_name)
        src = inspect.getsource(method)
        # Strip docstring before scanning so comments / prose mentioning
        # 'except' / 'try' don't false-positive.
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
            f"{method_name} body MUST NOT contain a try block "
            "(no error swallowing; delegate owns exception handling)"
        )
        # Match standalone except clauses; allow word "exception" in
        # docstrings (already stripped) or variable names (already
        # filtered by colon checks).
        assert "except " not in body and "except:" not in body, (
            f"{method_name} body MUST NOT contain an except clause"
        )
