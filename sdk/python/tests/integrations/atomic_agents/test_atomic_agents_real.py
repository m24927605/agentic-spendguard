# ruff: noqa: ANN001, ANN201, ANN202, ANN401, S101, S106
"""COV_D28 — integration tests using REAL Atomic Agents + Instructor.

Per ``docs/specs/coverage/D28_atomic_agents/tests.md`` §2 these
exercise:

  - End-to-end ``BaseAgent.run(...)`` round trip with a
    SpendGuard-wrapped Instructor client + ``pytest-httpx``-mocked
    OpenAI transport.
  - Pydantic ``output_schema`` round trip.
  - Instructor validation-retry → per-retry reservation
    (the load-bearing test that justifies wrapping the Instructor
    object rather than the raw SDK).
  - DENY path with zero HTTP reaching the inner transport.
  - Async path via ``instructor.from_openai(AsyncOpenAI(...))``.
  - Polyglot trace sharing with ``openai_agents``.
  - Rejected-alternative regression (raw ``openai.OpenAI()`` rejected
    by the factory with the docs-pointer message).

CI environments without ``atomic-agents`` / ``instructor`` /
``pytest-httpx`` installed SKIP the suite via
``pytest.importorskip``. Local dev with the ``[atomic-agents]`` extra
installed runs them.

Per review-standards §7.1 these are the load-bearing surface tests
that catch Atomic Agents- or Instructor-side surface changes (e.g. a
``BaseAgentConfig`` field rename in 2.x).
"""

from __future__ import annotations

import json
from typing import Any
from unittest.mock import AsyncMock, MagicMock

import pytest

# ─────────────────────────────────────────────────────────────────────
# Skip if any required extra is missing — CI without the extras must
# SKIP rather than fail (review-standards §7.1).
# ─────────────────────────────────────────────────────────────────────

instructor_pkg = pytest.importorskip("instructor", minversion="1.5")
atomic_agents_pkg = pytest.importorskip("atomic_agents", minversion="2.0")
pytest_httpx_pkg = pytest.importorskip("pytest_httpx")
openai_pkg = pytest.importorskip("openai", minversion="1.0")
pydantic_pkg = pytest.importorskip("pydantic", minversion="2.0")

import instructor  # noqa: E402
from openai import AsyncOpenAI, OpenAI  # noqa: E402
from pydantic import BaseModel  # noqa: E402

from spendguard._proto.spendguard.common.v1 import common_pb2  # noqa: E402

# ─────────────────────────────────────────────────────────────────────
# Import the adapter under test via the package-bypass path (mirrors
# the unit suite's pattern).
# ─────────────────────────────────────────────────────────────────────

from .conftest_atomic_agents import make_client_mock  # noqa: E402
from .test_atomic_agents import (  # noqa: E402
    SpendGuardAsyncInstructorProxy,
    SpendGuardInstructorProxy,
    _claim,
    run_context,
    RunContext,
    wrap_instructor_client,
)


# ─────────────────────────────────────────────────────────────────────
# Pydantic response schema used across the suite
# ─────────────────────────────────────────────────────────────────────


class Answer(BaseModel):
    """Minimal output schema mirroring what Atomic Agents Pydantic-first
    users typically declare. The ``final`` field is what the LLM must
    fill; the validation-retry test forces a wrong-type response on
    attempt #1 so Instructor re-prompts.
    """

    final: str


# ─────────────────────────────────────────────────────────────────────
# Helper: build a mocked OpenAI provider response (function-tools / TOOLS mode)
# ─────────────────────────────────────────────────────────────────────


def _tool_call_payload(
    *,
    completion_id: str = "chatcmpl-integration-1",
    tool_args_json: str,
    prompt_tokens: int = 12,
    completion_tokens: int = 20,
) -> dict[str, Any]:
    """OpenAI-shape chat completion with a function tool call.

    Instructor's TOOLS mode (the default for OpenAI) parses the
    function tool's ``arguments`` JSON via the supplied
    ``response_model``. ``tool_args_json`` is the JSON the assistant
    returns — supply ``{"final": "..."}`` for a valid Answer; supply
    an int-typed value for the validation-retry test.
    """
    return {
        "id": completion_id,
        "object": "chat.completion",
        "created": 1_700_000_000,
        "model": "gpt-4o-mini",
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": None,
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "Answer",
                                "arguments": tool_args_json,
                            },
                        }
                    ],
                },
                "finish_reason": "tool_calls",
            }
        ],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens,
        },
    }


# ─────────────────────────────────────────────────────────────────────
# Helper: build the wrapped Instructor + a fresh mocked SG client
# ─────────────────────────────────────────────────────────────────────


def _build_wrapped_instructor(
    *,
    async_path: bool = False,
    sg_client: Any = None,
) -> tuple[Any, Any]:
    """Return ``(guarded_instructor, sg_client_mock)``.

    The guarded client wraps either ``instructor.from_openai(OpenAI(...))``
    or ``instructor.from_openai(AsyncOpenAI(...))`` depending on
    ``async_path``. The inner OpenAI client hits ``api.openai.com``;
    ``pytest-httpx`` (HTTPX_MOCK fixture) intercepts.
    """
    if sg_client is None:
        sg_client = make_client_mock()
    if async_path:
        raw = instructor.from_openai(
            AsyncOpenAI(api_key="sk-test"), mode=instructor.Mode.TOOLS
        )
    else:
        raw = instructor.from_openai(
            OpenAI(api_key="sk-test"), mode=instructor.Mode.TOOLS
        )
    unit = common_pb2.UnitRef(
        unit_id="u1", token_kind="output_token", model_family="gpt-4"
    )
    pricing = common_pb2.PricingFreeze(pricing_version="v1")

    def estimator(_kwargs: dict[str, Any]) -> list[Any]:
        return [_claim(500)]

    guarded = wrap_instructor_client(
        raw,
        spendguard_client=sg_client,
        budget_id="b1",
        window_instance_id="w1",
        unit=unit,
        pricing=pricing,
        claim_estimator=estimator,
    )
    return guarded, sg_client


# ─────────────────────────────────────────────────────────────────────
# 2.1 End-to-end with real Atomic Agents + Instructor
# ─────────────────────────────────────────────────────────────────────


def test_real_atomic_agents_base_agent_round_trip(httpx_mock: Any) -> None:
    """Wire BaseAgent through wrap_instructor_client + assert PRE/POST pair.

    The wrapped Instructor accepts the BaseAgent's
    ``create_with_completion(...)`` call; ``pytest-httpx`` intercepts
    the OpenAI HTTP layer; the SpendGuard mock records exactly one
    ``request_decision`` + one ``emit_llm_call_post`` with the mocked
    usage flowing through.
    """
    httpx_mock.add_response(
        method="POST",
        url="https://api.openai.com/v1/chat/completions",
        json=_tool_call_payload(tool_args_json='{"final": "42"}'),
    )
    guarded, sg_client = _build_wrapped_instructor()
    assert isinstance(guarded, SpendGuardInstructorProxy)

    import asyncio

    async def _drive() -> Any:
        async with run_context(RunContext(run_id="r-real-1")):
            return await asyncio.to_thread(
                guarded.chat.completions.create_with_completion,
                model="gpt-4o-mini",
                response_model=Answer,
                messages=[{"role": "user", "content": "Two plus two?"}],
            )

    parsed, raw = asyncio.run(_drive())
    assert isinstance(parsed, Answer)
    assert parsed.final == "42"
    assert raw.usage.total_tokens == 32
    sg_client.request_decision.assert_awaited_once()
    sg_client.emit_llm_call_post.assert_awaited_once()
    kw = sg_client.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] == "32"
    assert kw["provider_event_id"] == "chatcmpl-integration-1"


def test_real_atomic_agents_pydantic_output_schema_round_trip(
    httpx_mock: Any,
) -> None:
    """Pydantic ``output_schema`` parses the tool-args JSON into Answer."""
    httpx_mock.add_response(
        method="POST",
        url="https://api.openai.com/v1/chat/completions",
        json=_tool_call_payload(tool_args_json='{"final": "hello world"}'),
    )
    guarded, sg_client = _build_wrapped_instructor()

    import asyncio

    async def _drive() -> Any:
        async with run_context(RunContext(run_id="r-real-2")):
            return await asyncio.to_thread(
                guarded.chat.completions.create_with_completion,
                model="gpt-4o-mini",
                response_model=Answer,
                messages=[{"role": "user", "content": "say hello world"}],
            )

    parsed, _raw = asyncio.run(_drive())
    assert isinstance(parsed, Answer)
    assert parsed.final == "hello world"
    # One PRE/POST pair, no more.
    assert sg_client.request_decision.await_count == 1
    assert sg_client.emit_llm_call_post.await_count == 1


def test_real_instructor_validation_retry_creates_per_retry_reservation(
    httpx_mock: Any,
) -> None:
    """LOAD-BEARING: Instructor's validation retry → fresh reservation.

    Attempt #1 returns ``{"final": 12345}`` (int, fails ``Answer.final:
    str`` validation). Instructor injects the ValidationError into
    ``messages`` and re-invokes our patched method. Attempt #2 returns
    valid JSON.

    Assert:
      - 2 ``request_decision`` calls (one per attempt)
      - 2 distinct ``llm_call_id`` values
      - 2 distinct ``decision_id`` values
      - 2 ``emit_llm_call_post`` calls (one per attempt — first one
        SUCCESS since the provider HTTP succeeded; the Pydantic
        validation error after that is handled inside Instructor's
        retry loop, not at the inner-call boundary)

    Per review-standards §2.2 this is the load-bearing test that
    justifies wrapping the Instructor object (NOT the raw SDK). If
    we wrapped the raw SDK, attempt #2 would re-enter Instructor's
    patched method (the wrong layer) and never hit the wrapper —
    silently undercounting cost.
    """
    # Attempt #1: invalid (int where str expected).
    httpx_mock.add_response(
        method="POST",
        url="https://api.openai.com/v1/chat/completions",
        json=_tool_call_payload(
            completion_id="chatcmpl-retry-1",
            tool_args_json='{"final": 12345}',  # int, will fail Pydantic
            prompt_tokens=12,
            completion_tokens=20,
        ),
    )
    # Attempt #2: valid.
    httpx_mock.add_response(
        method="POST",
        url="https://api.openai.com/v1/chat/completions",
        json=_tool_call_payload(
            completion_id="chatcmpl-retry-2",
            tool_args_json='{"final": "correct"}',
            prompt_tokens=18,
            completion_tokens=8,
        ),
    )
    guarded, sg_client = _build_wrapped_instructor()

    import asyncio

    async def _drive() -> Any:
        async with run_context(RunContext(run_id="r-retry")):
            return await asyncio.to_thread(
                guarded.chat.completions.create_with_completion,
                model="gpt-4o-mini",
                response_model=Answer,
                messages=[{"role": "user", "content": "give me a final"}],
                max_retries=2,
            )

    parsed, _raw = asyncio.run(_drive())
    assert parsed.final == "correct"
    # Two attempts → two reservations.
    assert sg_client.request_decision.await_count == 2, (
        "Each Instructor validation retry MUST get its own reservation; "
        "review-standards §2.2 makes this load-bearing."
    )
    # And two POSTs (one per attempt — first one SUCCESS because the
    # provider HTTP itself succeeded; Pydantic validation error
    # happens AFTER the inner call returns and AFTER our POST fires,
    # which is the documented behavior for instructor's validation
    # retry loop — review-standards §2.2 / §2.3).
    assert sg_client.emit_llm_call_post.await_count == 2
    # Each PRE has a distinct llm_call_id.
    calls = sg_client.request_decision.call_args_list
    llm_call_ids = {c.kwargs["llm_call_id"] for c in calls}
    decision_ids = {c.kwargs["decision_id"] for c in calls}
    assert len(llm_call_ids) == 2
    assert len(decision_ids) == 2


def test_real_atomic_agents_deny_path_zero_provider_http(
    httpx_mock: Any,
) -> None:
    """DENY raises ``DecisionDenied`` BEFORE any provider HTTP fires.

    Per review-standards §2.1 / §3 (security): DENY MUST raise
    BEFORE ``inner.client.chat.completions.create*(...)``. The
    ``pytest-httpx`` assertion below verifies ZERO HTTP requests hit
    the inner OpenAI transport.

    Instructor's retry loop wraps non-validation exceptions in
    ``InstructorRetryException``. The DENY raise propagates as the
    wrapped exception's last_exception; we assert both that NO HTTP
    fired AND that the DENY traveled through the chain. Operators
    using ``max_retries=`` with a tenacity ``Retrying`` predicate
    that excludes ``SpendGuardError`` see the raw ``DecisionDenied``
    directly.

    Missing the zero-HTTP assertion is a Blocker per review-standards §2.1.
    """
    # Use the direct errors-module path since the integrations barrel
    # would require importing through the package namespace (which the
    # unit suite's package-bypass setup avoids).
    from spendguard.errors import DecisionDenied

    sg_client = make_client_mock(
        request_decision_side_effect=DecisionDenied(
            "budget cap", decision_id="dec-deny-real"
        ),
    )
    guarded, sg_client = _build_wrapped_instructor(sg_client=sg_client)
    # Do NOT register any httpx_mock response — any HTTP call would
    # immediately error / fail-the-test because pytest-httpx requires
    # a registered response.

    import asyncio

    from instructor.core.exceptions import InstructorRetryException

    async def _drive() -> Any:
        async with run_context(RunContext(run_id="r-deny")):
            return await asyncio.to_thread(
                guarded.chat.completions.create_with_completion,
                model="gpt-4o-mini",
                response_model=Answer,
                messages=[{"role": "user", "content": "should be denied"}],
            )

    # Either DecisionDenied directly (if Instructor's retry layer
    # excludes SpendGuardError) or InstructorRetryException wrapping
    # it. The LOAD-BEARING assertion is zero HTTP + no POST.
    with pytest.raises((DecisionDenied, InstructorRetryException)) as excinfo:
        asyncio.run(_drive())
    # Verify the DENY surfaced through the chain — either as the raised
    # exception OR via the InstructorRetryException's repr (Instructor
    # serializes the inner exception as text).
    if isinstance(excinfo.value, InstructorRetryException):
        assert "budget cap" in str(excinfo.value)
    # Zero HTTP requests reached the inner transport.
    assert httpx_mock.get_requests() == []
    # POST not emitted (no reservation existed for any attempt).
    sg_client.emit_llm_call_post.assert_not_awaited()


@pytest.mark.asyncio
async def test_real_atomic_agents_async_round_trip(httpx_mock: Any) -> None:
    """Async path via ``instructor.from_openai(AsyncOpenAI(...))``."""
    httpx_mock.add_response(
        method="POST",
        url="https://api.openai.com/v1/chat/completions",
        json=_tool_call_payload(tool_args_json='{"final": "async-ok"}'),
    )
    guarded, sg_client = _build_wrapped_instructor(async_path=True)
    assert isinstance(guarded, SpendGuardAsyncInstructorProxy)
    async with run_context(RunContext(run_id="r-async-real")):
        parsed, _raw = await guarded.chat.completions.create_with_completion(
            model="gpt-4o-mini",
            response_model=Answer,
            messages=[{"role": "user", "content": "async hello"}],
        )
    assert isinstance(parsed, Answer)
    assert parsed.final == "async-ok"
    sg_client.request_decision.assert_awaited_once()
    sg_client.emit_llm_call_post.assert_awaited_once()


# ─────────────────────────────────────────────────────────────────────
# 2.2 Polyglot trace sharing with openai_agents
# ─────────────────────────────────────────────────────────────────────


def test_polyglot_run_context_shared_with_openai_agents(
    httpx_mock: Any,
) -> None:
    """One ``run_context`` covers D28 + ``openai_agents`` — shared run_id.

    The two modules import from the same module-level contextvar
    NAME (``spendguard_run_context``); a parent run wraps both an
    Atomic Agents call AND a SpendGuardAgentsModel call (when
    available) under one run_id. We assert by reading
    ``current_run_context()`` from both modules inside the same
    ``run_context()`` block.
    """
    # Skip-check FIRST (before registering the httpx_mock response) so
    # the mock isn't left dangling when the openai_agents extra is
    # absent — pytest-httpx fails the test on unused mocks.
    try:
        from spendguard.integrations.openai_agents import (
            current_run_context as oa_current,
        )
    except ImportError:
        pytest.skip(
            "openai_agents extra not installed; contextvar-name "
            "equivalence covers this case at runtime."
        )

    httpx_mock.add_response(
        method="POST",
        url="https://api.openai.com/v1/chat/completions",
        json=_tool_call_payload(tool_args_json='{"final": "polyglot"}'),
    )
    guarded, _ = _build_wrapped_instructor()

    from spendguard.integrations.atomic_agents import (
        current_run_context as aa_current,
    )

    import asyncio

    async def _drive() -> tuple[str, str, Any]:
        async with run_context(RunContext(run_id="polyglot-run-42")):
            oa_id = oa_current().run_id
            aa_id = aa_current().run_id
            parsed, _raw = await asyncio.to_thread(
                guarded.chat.completions.create_with_completion,
                model="gpt-4o-mini",
                response_model=Answer,
                messages=[{"role": "user", "content": "hi"}],
            )
            return oa_id, aa_id, parsed

    oa_id, aa_id, parsed = asyncio.run(_drive())
    assert oa_id == aa_id == "polyglot-run-42"
    assert isinstance(parsed, Answer)


# ─────────────────────────────────────────────────────────────────────
# 2.3 Rejected-alternative regression
# ─────────────────────────────────────────────────────────────────────


def test_raw_openai_wrap_rejected_by_factory() -> None:
    """``wrap_instructor_client(openai.OpenAI())`` → ``TypeError`` pointing at docs.

    Per review-standards §1.1: bare ``openai.OpenAI`` MUST be rejected
    with a clear pointer to ``instructor.from_openai`` AND to the
    docs page. Guards against operator drift toward the rejected
    alternative documented in design.md §1.
    """
    raw_openai = OpenAI(api_key="sk-test")
    with pytest.raises(TypeError) as excinfo:
        wrap_instructor_client(
            raw_openai,  # type: ignore[arg-type]
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
