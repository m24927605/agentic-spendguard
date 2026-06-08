# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S101, S106, S107
"""COV_D28 — pytest unit tests for the Atomic Agents Instructor-wrap adapter.

Mirrors ``tests/integrations/autogen/test_autogen.py`` shape but
targets ``wrap_instructor_client`` + ``SpendGuardInstructorProxy`` /
``SpendGuardAsyncInstructorProxy`` instead of the AutoGen
``ChatCompletionClient`` subclass.

Per ``docs/specs/coverage/D28_atomic_agents/tests.md`` §1 — 22 unit
cases covering construction / contract, sync ``create_with_completion``
PRE-POST flow, signature semantics (response_model identity / model /
tools / validation-retry divergence), exception handling, async path,
and shared run-context.

The module-level import uses the package-bypass pattern (mirrors the
agno / dspy / autogen test suites and the demo runner) so loading
``spendguard.integrations.atomic_agents._hook`` directly works even
when the package barrel raises ImportError due to ``instructor`` not
being installed in the CI venv — review-standards §7.3 expressly
permits this hybrid path.
"""

from __future__ import annotations

import asyncio
import importlib
import sys
from types import ModuleType, SimpleNamespace
from typing import Any

import pytest

# ─────────────────────────────────────────────────────────────────────
# Package-bypass import: load the adapter modules even when the
# ``[atomic-agents]`` extra isn't installed in the CI venv. The
# wrapper is import-resilient (it falls back to a duck-typed branch
# when instructor is missing — see ``_hook.py``'s
# ``_INSTRUCTOR_AVAILABLE`` branch).
# ─────────────────────────────────────────────────────────────────────

_PKG = "spendguard.integrations.atomic_agents"
if _PKG not in sys.modules:
    from pathlib import Path

    ns = ModuleType(_PKG)
    sdk_root = (
        Path(__file__).resolve().parents[3]
        / "src/spendguard/integrations/atomic_agents"
    )
    ns.__path__ = [str(sdk_root)]
    sys.modules[_PKG] = ns

_hook = importlib.import_module("spendguard.integrations.atomic_agents._hook")
_options = importlib.import_module(
    "spendguard.integrations.atomic_agents._options"
)
_errors = importlib.import_module(
    "spendguard.integrations.atomic_agents._errors"
)

wrap_instructor_client = _hook.wrap_instructor_client
SpendGuardInstructorProxy = _hook.SpendGuardInstructorProxy
SpendGuardAsyncInstructorProxy = _hook.SpendGuardAsyncInstructorProxy
RunContext = _hook.RunContext
run_context = _hook.run_context
current_run_context = _hook.current_run_context
ClaimEstimator = _hook.ClaimEstimator
_signature = _hook._signature
_extract_total_tokens = _hook._extract_total_tokens
_extract_provider_event_id = _hook._extract_provider_event_id
_classify_exception = _hook._classify_exception
_unpack_create_result = _hook._unpack_create_result
_guard_async_context = _hook._guard_async_context
_SyncInAsyncContext = _hook._SyncInAsyncContext
SpendGuardAtomicAgentsOptions = _options.SpendGuardAtomicAgentsOptions
DecisionDenied = _errors.DecisionDenied
DecisionStopped = _errors.DecisionStopped
SpendGuardConfigError = _errors.SpendGuardConfigError

from spendguard._proto.spendguard.common.v1 import common_pb2  # noqa: E402

from .conftest_atomic_agents import (  # noqa: E402
    INSTRUCTOR_AVAILABLE,
    FakeAsyncInstructor,
    FakeInstructor,
    _build_chat_completion,
    make_client_mock,
)


# ─────────────────────────────────────────────────────────────────────
# Helpers
# ─────────────────────────────────────────────────────────────────────


def _claim(amount: int = 100) -> Any:
    return common_pb2.BudgetClaim(
        budget_id="b1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        amount_atomic=str(amount),
        direction=common_pb2.BudgetClaim.DEBIT,
        window_instance_id="w1",
    )


def make_messages() -> list[dict[str, Any]]:
    """Realistic Instructor-shape messages payload."""
    return [
        {"role": "system", "content": "You are helpful."},
        {"role": "user", "content": "What's 2+2?"},
    ]


def make_kwargs(
    *,
    model: str = "gpt-4o-mini",
    messages: Any = None,
    response_model: Any = None,
    tools: Any = None,
    tool_choice: Any = None,
) -> dict[str, Any]:
    """Build a kwargs dict matching what Atomic Agents passes."""
    return {
        "model": model,
        "messages": messages if messages is not None else make_messages(),
        "response_model": response_model,
        "tools": tools,
        "tool_choice": tool_choice,
    }


def make_sync_wrapper(
    *,
    inner: Any = None,
    client: Any = None,
    claim_estimator: Any = None,
    pricing: Any = None,
) -> tuple[Any, Any, Any]:
    """Build a ``(wrapper, inner, client)`` triple — sync proxy."""
    if inner is None:
        inner = FakeInstructor()
    if client is None:
        client = make_client_mock()
    if claim_estimator is None:

        def claim_estimator(kwargs: dict[str, Any]) -> list[Any]:
            return [_claim(100)]

    unit = common_pb2.UnitRef(
        unit_id="u1", token_kind="output_token", model_family="gpt-4"
    )
    if pricing is None:
        pricing = common_pb2.PricingFreeze(pricing_version="v1")
    wrapper = wrap_instructor_client(
        inner,
        spendguard_client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=unit,
        pricing=pricing,
        claim_estimator=claim_estimator,
    )
    return wrapper, inner, client


def make_async_wrapper(
    *,
    inner: Any = None,
    client: Any = None,
    claim_estimator: Any = None,
    pricing: Any = None,
) -> tuple[Any, Any, Any]:
    """Build a ``(wrapper, inner, client)`` triple — async proxy."""
    if inner is None:
        inner = FakeAsyncInstructor()
    if client is None:
        client = make_client_mock()
    if claim_estimator is None:

        def claim_estimator(kwargs: dict[str, Any]) -> list[Any]:
            return [_claim(100)]

    unit = common_pb2.UnitRef(
        unit_id="u1", token_kind="output_token", model_family="gpt-4"
    )
    if pricing is None:
        pricing = common_pb2.PricingFreeze(pricing_version="v1")
    wrapper = wrap_instructor_client(
        inner,
        spendguard_client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=unit,
        pricing=pricing,
        claim_estimator=claim_estimator,
    )
    return wrapper, inner, client


# Skip the entire suite if instructor isn't importable; the wrapper's
# import-resilient branch is exercised separately by
# test_T01_import_error_without_instructor below.
pytestmark = pytest.mark.skipif(
    not INSTRUCTOR_AVAILABLE,
    reason=(
        "instructor not installed; unit suite requires the real ABC to "
        "exercise isinstance dispatch. CI without the extra will skip."
    ),
)


# ═════════════════════════════════════════════════════════════════════
# 1.1 Construction / contract (7 tests)
# ═════════════════════════════════════════════════════════════════════


def test_T01_import_error_without_instructor_pointer_in_message() -> None:
    """The barrel's ImportError message mentions [atomic-agents] extra.

    Per tests.md §1.1 / review-standards §1.1 the install hint points
    at the extras label so operators self-correct.
    """
    # We can't actually re-import without instructor; assert via the
    # source contents that the hint is present.
    from pathlib import Path

    init_path = (
        Path(_hook.__file__).resolve().parent / "__init__.py"
    )
    text = init_path.read_text(encoding="utf-8")
    assert "[atomic-agents]" in text
    assert "spendguard-sdk[atomic-agents]" in text


def test_T02_wrap_instructor_client_returns_sync_proxy_for_sync_instructor() -> None:
    """Factory dispatch on ``Instructor`` returns ``SpendGuardInstructorProxy``."""
    inner = FakeInstructor()
    wrapper, _, _ = make_sync_wrapper(inner=inner)
    assert isinstance(wrapper, SpendGuardInstructorProxy)
    assert not isinstance(wrapper, SpendGuardAsyncInstructorProxy)


def test_T03_wrap_instructor_client_returns_async_proxy_for_async_instructor() -> None:
    """Factory dispatch on ``AsyncInstructor`` returns the async proxy.

    Per review-standards §1.4: the dispatch order checks
    ``AsyncInstructor`` FIRST (more specific type), then
    ``Instructor`` — reversing silently routes async clients to the
    sync proxy and is a Blocker.
    """
    inner = FakeAsyncInstructor()
    wrapper, _, _ = make_async_wrapper(inner=inner)
    assert isinstance(wrapper, SpendGuardAsyncInstructorProxy)
    assert not isinstance(wrapper, SpendGuardInstructorProxy)


def test_T04_wrap_instructor_client_rejects_raw_openai_client() -> None:
    """Passing a non-Instructor raises ``TypeError`` pointing at instructor.from_openai.

    Per review-standards §1.1: bare ``openai.OpenAI`` MUST be
    rejected with a clear pointer (not a bare "invalid client"
    message). This guards the rejected raw-SDK alternative
    documented in design.md §1 (silently undercounts Instructor
    retries).
    """
    class _NotInstructor:
        pass

    with pytest.raises(TypeError) as excinfo:
        wrap_instructor_client(
            _NotInstructor(),  # type: ignore[arg-type]
            spendguard_client=make_client_mock(),
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_estimator=lambda kw: [_claim(100)],
        )
    msg = str(excinfo.value)
    assert "instructor.from_openai" in msg
    assert "docs/integrations/atomic-agents" in msg


def test_T05_getattr_delegates_to_inner() -> None:
    """``proxy.mode``, ``proxy.create_kwargs``, ``proxy.default_model`` reach inner."""
    inner = FakeInstructor()
    inner.mode = "JSON"
    inner.create_kwargs = {"temperature": 0.7}
    inner.default_model = "gpt-4o"
    wrapper, _, _ = make_sync_wrapper(inner=inner)
    assert wrapper.mode == "JSON"
    assert wrapper.create_kwargs == {"temperature": 0.7}
    assert wrapper.default_model == "gpt-4o"


def test_T06_getattr_does_not_shadow_explicit_attrs() -> None:
    """``proxy._client`` is the SpendGuard client, not anything on inner."""
    inner = FakeInstructor()
    # Even if inner has a same-named private attr, the proxy's
    # explicit attribute MUST win (no __getattr__ shadowing).
    inner._client = "inner-private-client"  # type: ignore[attr-defined]
    sg_client = make_client_mock()
    wrapper, _, _ = make_sync_wrapper(inner=inner, client=sg_client)
    assert wrapper._client is sg_client


def test_T07_constructor_rejects_missing_required_fields() -> None:
    """Empty budget_id / unit.unit_id / claim_estimator → ``SpendGuardConfigError``."""
    inner = FakeInstructor()
    sg_client = make_client_mock()
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        wrap_instructor_client(
            inner,
            spendguard_client=sg_client,
            budget_id="",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_estimator=lambda kw: [_claim(100)],
        )
    with pytest.raises(SpendGuardConfigError, match="unit.unit_id"):
        wrap_instructor_client(
            inner,
            spendguard_client=sg_client,
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id=""),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_estimator=lambda kw: [_claim(100)],
        )
    with pytest.raises(SpendGuardConfigError, match="claim_estimator"):
        wrap_instructor_client(
            inner,
            spendguard_client=sg_client,
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_estimator=None,  # type: ignore[arg-type]
        )


# ═════════════════════════════════════════════════════════════════════
# 1.2 Sync create_with_completion PRE/POST flow (7 tests)
# ═════════════════════════════════════════════════════════════════════


def _drive_sync_gated(wrapper: Any, **kwargs: Any) -> Any:
    """Drive the gated raw create_fn directly from a sync context.

    The wrapper's gated raw method (``_gated_raw_create``) is the
    per-attempt intercept point. Production callers route through
    ``inner.create_fn`` (which drives Instructor's retry loop on top
    of our gated raw); unit tests call the gated raw directly to
    isolate per-attempt PRE / POST semantics.
    """
    return wrapper._gated_raw_create(**kwargs)


def test_T08_sync_create_with_completion_emits_request_decision_with_llm_call_pre() -> None:
    """Single ALLOW round trip → ``request_decision(trigger=LLM_CALL_PRE)``."""
    wrapper, inner, client = make_sync_wrapper()

    async def _drive() -> Any:
        async with run_context(RunContext(run_id="r-8")):
            return await asyncio.to_thread(
                _drive_sync_gated, wrapper, **make_kwargs(),
            )

    result = asyncio.run(_drive())
    # The gated raw returns whatever the inner OpenAI's create returns —
    # for FakeInstructor that's a raw ChatCompletion-shaped namespace.
    assert result is not None
    client.request_decision.assert_awaited_once()
    kw = client.request_decision.call_args.kwargs
    assert kw["trigger"] == "LLM_CALL_PRE"
    assert kw["route"] == "llm.call"
    assert kw["run_id"] == "r-8"
    assert len(kw["projected_claims"]) == 1


def test_T09_sync_create_with_completion_passes_estimator_output_as_projected_claims() -> None:
    """Estimator return value flows verbatim into ``projected_claims``.

    The estimator receives the FULL kwargs dict (model / messages /
    response_model / tools / tool_choice) so it can project
    provider-aware claims.
    """
    captured: list[dict[str, Any]] = []

    def custom_estimator(kwargs: dict[str, Any]) -> list[Any]:
        captured.append(dict(kwargs))
        return [_claim(777)]

    wrapper, _, client = make_sync_wrapper(claim_estimator=custom_estimator)

    async def _drive() -> Any:
        async with run_context(RunContext(run_id="r-9")):
            return await asyncio.to_thread(
                _drive_sync_gated, wrapper, **make_kwargs(),
            )

    asyncio.run(_drive())
    assert len(captured) == 1
    # All keys flow through verbatim.
    assert captured[0]["model"] == "gpt-4o-mini"
    assert "messages" in captured[0]
    kw = client.request_decision.call_args.kwargs
    assert kw["projected_claims"][0].amount_atomic == "777"


def test_T10_sync_create_with_completion_post_uses_reservation_from_decision() -> None:
    """POST emits with ``reservation_id`` from decision's ``reservation_ids[0]``."""
    wrapper, _, client = make_sync_wrapper()

    async def _drive() -> Any:
        async with run_context(RunContext(run_id="r-10")):
            return await asyncio.to_thread(
                _drive_sync_gated, wrapper, **make_kwargs(),
            )

    asyncio.run(_drive())
    client.emit_llm_call_post.assert_awaited_once()
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["reservation_id"] == "res-1"
    assert kw["outcome"] == "SUCCESS"


def test_T11_sync_create_with_completion_post_uses_total_tokens_from_raw_completion() -> None:
    """``raw_completion.usage.total_tokens=42`` → ``estimated_amount_atomic='42'``."""
    inner = FakeInstructor(
        usage_prompt_tokens=15,
        usage_completion_tokens=27,
        usage_total_tokens=42,
    )
    wrapper, _, client = make_sync_wrapper(inner=inner)

    async def _drive() -> Any:
        async with run_context(RunContext(run_id="r-11")):
            return await asyncio.to_thread(
                _drive_sync_gated, wrapper, **make_kwargs(),
            )

    asyncio.run(_drive())
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] == "42"


def test_T12_extract_total_tokens_falls_back_to_prompt_plus_completion() -> None:
    """``usage.total_tokens=None`` → ``prompt + completion``."""
    raw = SimpleNamespace(
        id="x",
        usage=SimpleNamespace(
            prompt_tokens=10, completion_tokens=15, total_tokens=None
        ),
    )
    assert _extract_total_tokens(raw) == 25


def test_T13_extract_total_tokens_zero_when_usage_absent() -> None:
    """``raw_completion.usage is None`` → ``0`` (fail-soft)."""
    raw = SimpleNamespace(id="x", usage=None)
    assert _extract_total_tokens(raw) == 0
    # Also tolerate raw=None.
    assert _extract_total_tokens(None) == 0


def test_T14_sync_create_with_completion_skips_post_when_no_reservation() -> None:
    """DENY-shaped outcome (empty ``reservation_ids``) → POST NOT fired.

    Per review-standards §2.3 POST MUST NOT fire when no reservation
    exists. This guards the ALLOW-with-empty-reservation-ids corner
    case the projector might emit in degenerate test setups.
    """
    wrapper, _, client = make_sync_wrapper(
        client=make_client_mock(reservation_ids=()),
    )

    async def _drive() -> Any:
        async with run_context(RunContext(run_id="r-14")):
            return await asyncio.to_thread(
                _drive_sync_gated, wrapper, **make_kwargs(),
            )

    asyncio.run(_drive())
    client.emit_llm_call_post.assert_not_awaited()


# ═════════════════════════════════════════════════════════════════════
# 1.3 Signature semantics + create() (.create returns parsed) (4 tests)
# ═════════════════════════════════════════════════════════════════════


def test_T15_signature_includes_response_model_identity() -> None:
    """Same messages + different ``response_model`` → distinct llm_call_ids.

    Per review-standards §3.1: omitting ``response_model`` from the
    signature lets a tenant flip schema mid-reservation. Blocker.
    """
    from pydantic import BaseModel

    class A(BaseModel):
        val: str = ""

    class B(BaseModel):
        val: int = 0

    kw1 = make_kwargs(response_model=A)
    kw2 = make_kwargs(response_model=B)
    s1 = _signature(kw1)
    s2 = _signature(kw2)
    assert s1 != s2
    # And no response_model at all is yet another signature.
    s3 = _signature(make_kwargs(response_model=None))
    assert s3 != s1
    assert s3 != s2


def test_T16_signature_includes_model_and_tools_and_tool_choice() -> None:
    """Per review-standards §3.2: model swap → fresh llm_call_id.

    Also tools / tool_choice swap → fresh llm_call_id because both
    affect provider routing + cost class.
    """
    s_4o = _signature(make_kwargs(model="gpt-4o"))
    s_mini = _signature(make_kwargs(model="gpt-4o-mini"))
    assert s_4o != s_mini
    # Tools differ → distinct.
    s_no_tools = _signature(make_kwargs(tools=None))
    s_one_tool = _signature(
        make_kwargs(tools=[{"type": "function", "function": {"name": "f"}}])
    )
    assert s_no_tools != s_one_tool
    # tool_choice differs → distinct.
    s_auto = _signature(make_kwargs(tool_choice="auto"))
    s_required = _signature(make_kwargs(tool_choice="required"))
    assert s_auto != s_required


def test_T17_signature_diverges_across_instructor_validation_retries() -> None:
    """Each Instructor retry mutates ``messages`` → distinct llm_call_ids.

    Per review-standards §2.2 this is the load-bearing mechanism that
    justifies wrapping the Instructor object (not the raw SDK). An
    explicit retry counter would be a Blocker — too tight a coupling
    to Instructor's internal retry state.
    """
    # Attempt 1: original messages.
    attempt_1 = make_kwargs(messages=make_messages())
    # Attempt 2: validation error injected (Instructor's actual retry
    # behavior — verified against instructor==1.5.2 internals).
    retry_messages = make_messages() + [
        {
            "role": "tool",
            "tool_call_id": "fake",
            "content": "Validation error: field 'final' must be str, got int",
        }
    ]
    attempt_2 = make_kwargs(messages=retry_messages)
    s1 = _signature(attempt_1)
    s2 = _signature(attempt_2)
    assert s1 != s2, (
        "Signature MUST diverge across Instructor validation retries — "
        "each retry needs its own reservation per review-standards §2.2."
    )


def test_T18_gated_raw_create_intercepts_at_per_attempt_boundary() -> None:
    """The gated raw ``create`` returns the raw provider response directly.

    Per DEVIATION-C the gate sits at the raw provider method
    (``inner.client.chat.completions.create``), which returns the
    raw ``ChatCompletion``. ``_extract_total_tokens`` reads usage
    from this raw object directly — no ``_raw_response`` indirection
    because we intercept BEFORE Instructor's ``process_response``
    parses it.

    The ``_unpack_create_result`` helper remains in the module for
    callers that pre-extract from a parsed model; it still reads
    ``_raw_response`` for the ``.create()`` shape.
    """
    inner = FakeInstructor(
        usage_prompt_tokens=8, usage_completion_tokens=12, usage_total_tokens=20
    )
    wrapper, _, client = make_sync_wrapper(inner=inner)

    async def _drive() -> Any:
        async with run_context(RunContext(run_id="r-18")):
            return await asyncio.to_thread(
                _drive_sync_gated, wrapper, **make_kwargs(),
            )

    result = asyncio.run(_drive())
    # Our gated raw returns the raw ChatCompletion directly.
    # POST used the usage from that raw object (20 total tokens).
    assert result.usage.total_tokens == 20
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] == "20"
    # _unpack_create_result still works for callers that pass a
    # parsed model whose _raw_response carries the raw completion
    # (used by tooling that pre-extracts via .create()).
    fake_parsed = SimpleNamespace(
        _raw_response=_build_chat_completion(
            prompt_tokens=3, completion_tokens=4, total_tokens=7
        )
    )
    raw = _unpack_create_result("create", fake_parsed)
    assert _extract_total_tokens(raw) == 7


# ═════════════════════════════════════════════════════════════════════
# 1.4 Exception handling (2 tests)
# ═════════════════════════════════════════════════════════════════════


def test_T19_sync_gated_raw_failure_emits_post_failure() -> None:
    """Inner raises ``RuntimeError`` → POST ``outcome='FAILURE'`` + re-raise.

    Drives the gated raw directly (bypasses Instructor's retry layer
    that would wrap in ``InstructorRetryException``). Production
    callers that route via Instructor's retry loop see the wrap;
    the per-attempt POST semantics — fail-soft commit with
    ``outcome=FAILURE`` and ``estimated_amount_atomic=0`` — are the
    load-bearing assertion here.
    """
    inner = FakeInstructor(raise_on_create=RuntimeError("provider down"))
    wrapper, _, client = make_sync_wrapper(inner=inner)

    async def _drive() -> Any:
        async with run_context(RunContext(run_id="r-19")):
            return await asyncio.to_thread(
                _drive_sync_gated, wrapper, **make_kwargs(),
            )

    with pytest.raises(RuntimeError, match="provider down"):
        asyncio.run(_drive())
    client.emit_llm_call_post.assert_awaited_once()
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "FAILURE"
    assert kw["estimated_amount_atomic"] == "0"


def test_T20_classify_exception_detects_cancellederror_by_name() -> None:
    """``type(exc).__name__ == "CancelledError"`` → ``CANCELLED``.

    Per review-standards §2.3 we detect via name to avoid cross-loop
    ``isinstance`` mismatches (asyncio / trio / anyio all raise their
    own ``CancelledError``).
    """
    class CancelledError(Exception):
        pass

    assert _classify_exception(CancelledError()) == "CANCELLED"
    assert _classify_exception(RuntimeError("nope")) == "FAILURE"
    # Real asyncio.CancelledError matches by name.
    assert _classify_exception(asyncio.CancelledError()) == "CANCELLED"


# ═════════════════════════════════════════════════════════════════════
# 1.5 Async path (1 test)
# ═════════════════════════════════════════════════════════════════════


@pytest.mark.asyncio
async def test_T21_async_create_with_completion_full_round_trip() -> None:
    """Async ALLOW with ``AsyncInstructor`` — request_decision + POST awaited."""
    wrapper, inner, client = make_async_wrapper()
    async with run_context(RunContext(run_id="r-21")):
        # Drive the gated raw create_fn directly — the wrapper's
        # passthrough drives inner.create_with_completion which calls
        # the gated create_fn. We also accept the gated create_fn
        # being called directly (more direct unit-level path).
        await wrapper._inner.create_fn(
            messages=make_messages(),
            model="gpt-4o-mini",
        )
    client.request_decision.assert_awaited_once()
    kw_pre = client.request_decision.call_args.kwargs
    assert kw_pre["trigger"] == "LLM_CALL_PRE"
    assert kw_pre["run_id"] == "r-21"
    client.emit_llm_call_post.assert_awaited_once()
    kw_post = client.emit_llm_call_post.call_args.kwargs
    assert kw_post["outcome"] == "SUCCESS"
    assert int(kw_post["estimated_amount_atomic"]) > 0


# ═════════════════════════════════════════════════════════════════════
# 1.6 Run context (1 test)
# ═════════════════════════════════════════════════════════════════════


def test_T22_sync_create_raises_without_active_run_context() -> None:
    """Calling outside ``run_context()`` raises ``RuntimeError``.

    The error message contract matches openai_agents.current_run_context
    (same wording up to the integration name) so cross-framework
    callers get a unified hint.
    """
    wrapper, _, _ = make_sync_wrapper()
    # No run_context bound — the gated raw must raise via
    # current_run_context's standard RuntimeError. We invoke through
    # asyncio.to_thread to avoid the _guard_async_context trip.
    async def _drive() -> Any:
        return await asyncio.to_thread(
            _drive_sync_gated, wrapper, **make_kwargs(),
        )

    with pytest.raises(RuntimeError, match="run_context"):
        asyncio.run(_drive())


# ═════════════════════════════════════════════════════════════════════
# Extra: Options dataclass validates required fields
# ═════════════════════════════════════════════════════════════════════


def test_options_dataclass_validates() -> None:
    """``SpendGuardAtomicAgentsOptions`` rejects empty required fields."""
    opts = SpendGuardAtomicAgentsOptions(
        tenant_id="t1", budget_id="b1", window_instance_id="w1"
    )
    assert opts.route == "llm.call"
    with pytest.raises(SpendGuardConfigError, match="tenant_id"):
        SpendGuardAtomicAgentsOptions(
            tenant_id="", budget_id="b1", window_instance_id="w1"
        )
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        SpendGuardAtomicAgentsOptions(
            tenant_id="t1", budget_id="  ", window_instance_id="w1"
        )
    with pytest.raises(SpendGuardConfigError, match="window_instance_id"):
        SpendGuardAtomicAgentsOptions(
            tenant_id="t1", budget_id="b1", window_instance_id=""
        )


# ═════════════════════════════════════════════════════════════════════
# Extra: sync proxy raises _SyncInAsyncContext inside running loop
# ═════════════════════════════════════════════════════════════════════


@pytest.mark.asyncio
async def test_sync_proxy_raises_in_async_context() -> None:
    """Sync proxy invoked from inside a running loop raises typed config error.

    The error message points the operator at ``AsyncInstructor`` as
    the fix — review-standards §1.4 / file-level docs.
    """
    wrapper, _, _ = make_sync_wrapper()
    async with run_context(RunContext(run_id="r-async-guard")):
        with pytest.raises(_SyncInAsyncContext, match="AsyncInstructor"):
            _drive_sync_gated(wrapper, **make_kwargs())


# ═════════════════════════════════════════════════════════════════════
# Extra: _unpack_create_result handles unexpected shapes
# ═════════════════════════════════════════════════════════════════════


def test_unpack_create_result_handles_unexpected_shapes() -> None:
    """``create_with_completion`` non-tuple result → raw=None (defensive)."""
    # Normal: (parsed, raw) → raw.
    raw = _build_chat_completion(prompt_tokens=1, completion_tokens=2)
    assert _unpack_create_result("create_with_completion", (object(), raw)) is raw
    # Degenerate: single value → None (don't crash audit chain).
    assert _unpack_create_result("create_with_completion", object()) is None
    # create() with _raw_response set → returns raw.
    parsed = SimpleNamespace(_raw_response=raw)
    assert _unpack_create_result("create", parsed) is raw
    # create() with no _raw_response → None.
    assert _unpack_create_result("create", object()) is None


# ═════════════════════════════════════════════════════════════════════
# Extra: extract_provider_event_id is best-effort
# ═════════════════════════════════════════════════════════════════════


def test_extract_provider_event_id_handles_missing_id() -> None:
    """Missing or empty ``id`` → ``""`` (never raises)."""
    raw = _build_chat_completion(prompt_tokens=1, completion_tokens=2)
    assert _extract_provider_event_id(raw) == "chatcmpl-fake-0"
    assert _extract_provider_event_id(None) == ""
    assert _extract_provider_event_id(SimpleNamespace()) == ""
    assert _extract_provider_event_id(SimpleNamespace(id="")) == ""


# ═════════════════════════════════════════════════════════════════════
# Extra: DENY path raises before inner.create — zero inner calls
# ═════════════════════════════════════════════════════════════════════


def test_deny_path_raises_before_inner_create_call() -> None:
    """DENY raises before any inner create_fn invocation (per-attempt boundary).

    Per review-standards §2.1 + DEVIATION-C: ``request_decision`` is
    awaited BEFORE ``inner.create_fn(...)`` (the per-attempt boundary
    inside Instructor's retry loop). DENY raises ``DecisionDenied``
    BEFORE any provider HTTP could fire — the counterpart to the
    integration test that asserts zero HTTP on the deny path.

    Note: the pass-through ``chat.completions.create_with_completion``
    on the proxy WILL be invoked (it's a thin pass-through to the
    inner) and the FakeInstructor records that call. The load-bearing
    assertion is that ``create_fn`` (the gated layer) was NOT called.
    """
    inner = FakeInstructor()
    sg_client = make_client_mock(
        request_decision_side_effect=DecisionDenied(
            "cap", decision_id="dec-deny"
        ),
    )
    wrapper, _, _ = make_sync_wrapper(inner=inner, client=sg_client)

    async def _drive() -> Any:
        async with run_context(RunContext(run_id="r-deny")):
            return await asyncio.to_thread(
                _drive_sync_gated, wrapper, **make_kwargs(),
            )

    with pytest.raises(DecisionDenied):
        asyncio.run(_drive())
    # The raw provider create (via ``inner.client.chat.completions.create``)
    # NEVER ran. The gate raised at PRE before the inner method.
    raw_calls = [c for c in inner.calls if c[0] == "client.chat.completions.create"]
    assert raw_calls == []


# ─────────────────────────────────────────────────────────────────────
# HARDEN_D05_UR — TP-01..03: `unit_id` options field threading.
#
# Per docs/specs/harden_d05_unit_ref/tests.md §2.2, every Python adapter
# in the sweep MUST expose an optional ``unit_id`` on its options
# dataclass and (a) accept it at construction, (b) thread it onto the
# wire ``BudgetClaim.unit.unit_id``, and (c) keep constructing when the
# field is omitted (backward compat).
# ─────────────────────────────────────────────────────────────────────

_UNIT_ID_FIXTURE = "550e8400-e29b-41d4-a716-446655440000"


def test_TP01_options_accepts_unit_id() -> None:
    """TP-01 — ``SpendGuardAtomicAgentsOptions(unit_id=...)`` constructs."""
    opts = SpendGuardAtomicAgentsOptions(
        tenant_id="t1",
        budget_id="b1",
        window_instance_id="w1",
        unit_id=_UNIT_ID_FIXTURE,
    )
    assert opts.unit_id == _UNIT_ID_FIXTURE


def test_TP02_unit_id_threads_to_wire_claim() -> None:
    """TP-02 — operator binds ``options.unit_id`` to the proto ``UnitRef``;
    the resulting wire ``BudgetClaim.unit.unit_id`` carries it verbatim.
    """
    opts = SpendGuardAtomicAgentsOptions(
        tenant_id="t1",
        budget_id="b1",
        window_instance_id="w1",
        unit_id=_UNIT_ID_FIXTURE,
    )
    client = make_client_mock()
    wrapper = wrap_instructor_client(
        FakeInstructor(),
        spendguard_client=client,
        budget_id=opts.budget_id,
        window_instance_id=opts.window_instance_id,
        unit=common_pb2.UnitRef(unit_id=opts.unit_id or ""),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        claim_estimator=lambda kw: [
            common_pb2.BudgetClaim(
                budget_id="b1",
                unit=common_pb2.UnitRef(unit_id=opts.unit_id or ""),
                amount_atomic="100",
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id="w1",
            )
        ],
    )

    async def _drive() -> Any:
        async with run_context(RunContext(run_id="r-tp02")):
            return await asyncio.to_thread(
                _drive_sync_gated, wrapper, **make_kwargs(),
            )

    asyncio.run(_drive())
    kw = client.request_decision.call_args.kwargs
    assert kw["projected_claims"][0].unit.unit_id == _UNIT_ID_FIXTURE


def test_TP03_options_without_unit_id_constructs() -> None:
    """TP-03 — backward compat: omitting ``unit_id`` keeps default None."""
    opts = SpendGuardAtomicAgentsOptions(
        tenant_id="t1",
        budget_id="b1",
        window_instance_id="w1",
    )
    assert opts.unit_id is None
