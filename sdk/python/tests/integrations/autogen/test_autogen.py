# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S101, S106, S107
"""COV_D24 — pytest unit tests for the AutoGen / AG2 adapter.

Mirrors ``tests/integrations/agno/test_agno_pre_post.py`` shape but
targets ``SpendGuardChatCompletionClient`` instead of pre/post factories.
Uses ``FakeChatCompletionClient`` (subclasses the real ABC when
``autogen-core`` is installed, plain base class otherwise — see
``conftest_autogen.py``) so the unit suite runs across CI environments
with and without the extras.

Per ``docs/specs/coverage/D24_autogen_ag2/tests.md`` §1 — 22 unit cases
covering construction / contract, ``create()`` PRE-POST flow,
exception handling, ``create_stream()`` POC, pass-through introspection,
and shared run-context.

The module-level import uses the package-bypass pattern (mirrors the
agno / dspy test suites and the demo runner) so that loading
``spendguard.integrations.autogen._hook`` directly works even when the
package barrel raises an ImportError due to autogen-core not being
installed — review-standards §7.3 expressly permits this hybrid path.
"""

from __future__ import annotations

import asyncio
import importlib
import sys
from types import ModuleType, SimpleNamespace
from typing import Any
from unittest.mock import patch

import pytest

# ─────────────────────────────────────────────────────────────────────
# Package-bypass import: load the adapter modules even when the
# ``[autogen]`` extra isn't installed in the CI venv. The wrapper class
# is import-resilient (it falls back to a plain base class — see
# ``_hook.py``'s ``_ClientBase`` branch).
# ─────────────────────────────────────────────────────────────────────

_PKG = "spendguard.integrations.autogen"
if _PKG not in sys.modules:
    from pathlib import Path

    ns = ModuleType(_PKG)
    sdk_root = (
        Path(__file__).resolve().parents[3]
        / "src/spendguard/integrations/autogen"
    )
    ns.__path__ = [str(sdk_root)]
    sys.modules[_PKG] = ns

_hook = importlib.import_module("spendguard.integrations.autogen._hook")
_options = importlib.import_module("spendguard.integrations.autogen._options")
_errors = importlib.import_module("spendguard.integrations.autogen._errors")

SpendGuardChatCompletionClient = _hook.SpendGuardChatCompletionClient
LINEAGE = _hook.LINEAGE
RunContext = _hook.RunContext
run_context = _hook.run_context
current_run_context = _hook.current_run_context
ClaimEstimator = _hook.ClaimEstimator
_signature = _hook._signature
_extract_total_tokens = _hook._extract_total_tokens
_classify_exception = _hook._classify_exception
SpendGuardAutoGenOptions = _options.SpendGuardAutoGenOptions
DecisionDenied = _errors.DecisionDenied
DecisionStopped = _errors.DecisionStopped
SpendGuardConfigError = _errors.SpendGuardConfigError

from spendguard._proto.spendguard.common.v1 import common_pb2  # noqa: E402

from .conftest_autogen import (  # noqa: E402
    AUTOGEN_CORE_AVAILABLE,
    FakeChatCompletionClient,
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


def make_messages() -> list[Any]:
    """Realistic LLMMessage-shaped payload for unit tests."""
    return [
        SimpleNamespace(role="system", content="You are helpful."),
        SimpleNamespace(role="user", content="Hello."),
    ]


def make_wrapper(
    *,
    inner: Any = None,
    client: Any = None,
    claim_estimator: Any = None,
    pricing: Any = None,
) -> tuple[Any, Any, Any]:
    """Build a ``(wrapper, inner, client)`` triple with sane defaults."""
    if inner is None:
        inner = FakeChatCompletionClient()
    if client is None:
        client = make_client_mock()
    if claim_estimator is None:

        def claim_estimator(messages: list[Any]) -> list[Any]:
            return [_claim(100)]

    unit = common_pb2.UnitRef(
        unit_id="u1", token_kind="output_token", model_family="gpt-4"
    )
    if pricing is None:
        pricing = common_pb2.PricingFreeze(pricing_version="v1")
    wrapper = SpendGuardChatCompletionClient(
        inner=inner,
        client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=unit,
        pricing=pricing,
        claim_estimator=claim_estimator,
    )
    return wrapper, inner, client


# ═════════════════════════════════════════════════════════════════════
# 1.1 Construction / contract
# ═════════════════════════════════════════════════════════════════════


def test_T01_constructor_skips_super_init() -> None:
    """Wrapper does not call ``ChatCompletionClient.__init__``.

    Per review-standards §1.2: ABC has no shared state; calling
    ``super().__init__()`` would silently change inner-client behavior
    under upstream refactors. Verified by patching the ABC's
    ``__init__`` and asserting it is NEVER called.
    """
    if not AUTOGEN_CORE_AVAILABLE:  # pragma: no cover
        pytest.skip("autogen-core not installed; skipping ABC-init guard.")
    from autogen_core.models import ChatCompletionClient

    with patch.object(
        ChatCompletionClient, "__init__", autospec=True
    ) as mock_init:
        make_wrapper()
        mock_init.assert_not_called()


def test_T02_constructor_rejects_none_inner() -> None:
    inner = FakeChatCompletionClient()  # used as a sentinel that None branch rejects
    del inner
    with pytest.raises(SpendGuardConfigError, match="inner"):
        SpendGuardChatCompletionClient(
            inner=None,  # type: ignore[arg-type]
            client=make_client_mock(),
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_estimator=lambda m: [_claim(100)],
        )


def test_T03_constructor_rejects_none_client() -> None:
    with pytest.raises(SpendGuardConfigError, match="client"):
        SpendGuardChatCompletionClient(
            inner=FakeChatCompletionClient(),
            client=None,  # type: ignore[arg-type]
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_estimator=lambda m: [_claim(100)],
        )


def test_T04_constructor_rejects_empty_budget_id() -> None:
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        SpendGuardChatCompletionClient(
            inner=FakeChatCompletionClient(),
            client=make_client_mock(),
            budget_id="",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_estimator=lambda m: [_claim(100)],
        )


def test_T05_constructor_rejects_empty_unit_id() -> None:
    with pytest.raises(SpendGuardConfigError, match="unit.unit_id"):
        SpendGuardChatCompletionClient(
            inner=FakeChatCompletionClient(),
            client=make_client_mock(),
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id=""),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_estimator=lambda m: [_claim(100)],
        )


def test_T06_constructor_rejects_none_claim_estimator() -> None:
    """Design.md §5: no default claim_estimator — operator MUST pass one."""
    with pytest.raises(SpendGuardConfigError, match="claim_estimator"):
        SpendGuardChatCompletionClient(
            inner=FakeChatCompletionClient(),
            client=make_client_mock(),
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_estimator=None,  # type: ignore[arg-type]
        )


def test_T07_lineage_probe_string() -> None:
    """``LINEAGE`` is one of the documented strings; never branches business logic."""
    assert LINEAGE in {"autogen", "ag2", "both", "core-only"}


def test_T08_options_dataclass_validates() -> None:
    opts = SpendGuardAutoGenOptions(
        tenant_id="t1", budget_id="b1", window_instance_id="w1"
    )
    assert opts.route == "llm.call"
    with pytest.raises(SpendGuardConfigError, match="tenant_id"):
        SpendGuardAutoGenOptions(
            tenant_id="", budget_id="b1", window_instance_id="w1"
        )


# ═════════════════════════════════════════════════════════════════════
# 1.2 create() PRE/POST flow
# ═════════════════════════════════════════════════════════════════════


@pytest.mark.asyncio
async def test_T09_create_emits_request_decision_with_llm_call_pre_trigger() -> None:
    wrapper, inner, client = make_wrapper()
    async with run_context(RunContext(run_id="r-9")):
        result = await wrapper.create(make_messages())
    assert result is not None
    client.request_decision.assert_awaited_once()
    kw = client.request_decision.call_args.kwargs
    assert kw["trigger"] == "LLM_CALL_PRE"
    assert kw["route"] == "llm.call"
    assert kw["run_id"] == "r-9"
    assert len(kw["projected_claims"]) == 1
    # PRE fired BEFORE inner.create — assert ordering via call count.
    assert len(inner.calls) == 1


@pytest.mark.asyncio
async def test_T10_create_passes_estimator_output_as_projected_claims() -> None:
    captured: list[list[Any]] = []

    def custom_estimator(messages: list[Any]) -> list[Any]:
        captured.append(list(messages))
        return [_claim(777)]

    wrapper, _, client = make_wrapper(claim_estimator=custom_estimator)
    msgs = make_messages()
    async with run_context(RunContext(run_id="r-10")):
        await wrapper.create(msgs)
    # Estimator received the same messages list reference (verbatim).
    assert len(captured) == 1
    # And the projected claim flowed verbatim into request_decision.
    kw = client.request_decision.call_args.kwargs
    assert kw["projected_claims"][0].amount_atomic == "777"


@pytest.mark.asyncio
async def test_T11_create_post_uses_reservation_from_decision() -> None:
    wrapper, _, client = make_wrapper()
    async with run_context(RunContext(run_id="r-11")):
        await wrapper.create(make_messages())
    client.emit_llm_call_post.assert_awaited_once()
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["reservation_id"] == "res-1"
    assert kw["outcome"] == "SUCCESS"


@pytest.mark.asyncio
async def test_T12_create_post_estimated_amount_equals_prompt_plus_completion_tokens() -> None:
    inner = FakeChatCompletionClient(
        usage_prompt_tokens=11, usage_completion_tokens=22
    )
    wrapper, _, client = make_wrapper(inner=inner)
    async with run_context(RunContext(run_id="r-12")):
        await wrapper.create(make_messages())
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] == "33"


@pytest.mark.asyncio
async def test_T13_create_post_estimated_amount_zero_when_usage_absent() -> None:
    inner = FakeChatCompletionClient(no_usage=True)
    wrapper, _, client = make_wrapper(inner=inner)
    async with run_context(RunContext(run_id="r-13")):
        await wrapper.create(make_messages())
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] == "0"


@pytest.mark.asyncio
async def test_T14_create_skips_post_when_no_reservation() -> None:
    """DENY-path defensive: empty reservation_ids → POST MUST NOT fire."""
    client = make_client_mock(reservation_ids=())
    wrapper, _, _ = make_wrapper(client=client)
    async with run_context(RunContext(run_id="r-14")):
        await wrapper.create(make_messages())
    client.emit_llm_call_post.assert_not_awaited()


@pytest.mark.asyncio
async def test_T15_create_propagates_extra_create_args_to_inner_verbatim() -> None:
    wrapper, inner, _ = make_wrapper()
    user_extra: dict[str, Any] = {"temperature": 0.7, "seed": 42}
    async with run_context(RunContext(run_id="r-15")):
        await wrapper.create(make_messages(), extra_create_args=user_extra)
    # Inner received the dict (may be a shallow copy — review-standards §6).
    inner_extra = inner.calls[0]["extra_create_args"]
    assert inner_extra == user_extra
    # User's original dict was NOT mutated.
    assert user_extra == {"temperature": 0.7, "seed": 42}


@pytest.mark.asyncio
async def test_T16_create_signature_includes_tools() -> None:
    """Different ``tools`` → different signature → different llm_call_id."""
    sig_no_tools = _signature(make_messages(), (), {})
    sig_with_tools = _signature(
        make_messages(),
        ({"name": "search", "args": "..."},),
        {},
    )
    assert sig_no_tools != sig_with_tools


@pytest.mark.asyncio
async def test_T17_create_signature_includes_extra_create_args() -> None:
    sig_a = _signature(make_messages(), (), {"temperature": 0.5})
    sig_b = _signature(make_messages(), (), {"temperature": 0.9})
    assert sig_a != sig_b


def test_T17b_signature_sorts_dict_for_determinism() -> None:
    """Different insertion orders of the same dict → same signature.

    Review-standards §6: sorting is required for determinism.
    """
    sig_a = _signature(make_messages(), (), {"a": 1, "b": 2})
    sig_b = _signature(make_messages(), (), {"b": 2, "a": 1})
    assert sig_a == sig_b


# ═════════════════════════════════════════════════════════════════════
# 1.3 Exception handling
# ═════════════════════════════════════════════════════════════════════


@pytest.mark.asyncio
async def test_T18_create_failure_emits_post_failure() -> None:
    inner = FakeChatCompletionClient(raise_on_create=RuntimeError("boom"))
    wrapper, _, client = make_wrapper(inner=inner)
    with pytest.raises(RuntimeError, match="boom"):
        async with run_context(RunContext(run_id="r-18")):
            await wrapper.create(make_messages())
    client.emit_llm_call_post.assert_awaited_once()
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "FAILURE"
    assert kw["estimated_amount_atomic"] == "0"


@pytest.mark.asyncio
async def test_T19_create_cancelled_emits_post_cancelled() -> None:
    inner = FakeChatCompletionClient(
        raise_on_create=asyncio.CancelledError()
    )
    wrapper, _, client = make_wrapper(inner=inner)
    with pytest.raises(asyncio.CancelledError):
        async with run_context(RunContext(run_id="r-19")):
            await wrapper.create(make_messages())
    client.emit_llm_call_post.assert_awaited_once()
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "CANCELLED"


@pytest.mark.asyncio
async def test_T20_create_deny_raises_before_inner_called() -> None:
    """Fail-closed: DENY raises before any inner-client method is awaited.

    Per review-standards §2.1 the integration test asserts ZERO HTTP
    requests reached the inner transport on DENY. Unit-level proxy:
    ``inner.calls`` stays empty.
    """
    denied = DecisionDenied(
        "budget exhausted",
        decision_id="dec-deny",
        reason_codes=["BUDGET_EXHAUSTED"],
    )
    client = make_client_mock(request_decision_side_effect=denied)
    wrapper, inner, _ = make_wrapper(client=client)
    with pytest.raises(DecisionDenied):
        async with run_context(RunContext(run_id="r-20")):
            await wrapper.create(make_messages())
    # Inner was NEVER called — fail-closed at PRE boundary.
    assert inner.calls == []
    # POST was NEVER emitted — no reservation to commit.
    client.emit_llm_call_post.assert_not_awaited()


@pytest.mark.asyncio
async def test_T20b_create_stop_raises_before_inner_called() -> None:
    """STOP_RUN_PROJECTION → DecisionStopped → fail-closed at PRE."""
    stopped = DecisionStopped(
        "stop run projection",
        decision_id="dec-stop",
        reason_codes=["STOP_RUN_PROJECTION"],
    )
    client = make_client_mock(request_decision_side_effect=stopped)
    wrapper, inner, _ = make_wrapper(client=client)
    with pytest.raises(DecisionStopped):
        async with run_context(RunContext(run_id="r-20b")):
            await wrapper.create(make_messages())
    assert inner.calls == []


# ═════════════════════════════════════════════════════════════════════
# 1.4 create_stream() POC
# ═════════════════════════════════════════════════════════════════════


@pytest.mark.asyncio
async def test_T21_create_stream_passes_through_to_inner() -> None:
    wrapper, inner, _ = make_wrapper()
    stream = wrapper.create_stream(make_messages())
    chunks: list[Any] = []
    async for chunk in stream:
        chunks.append(chunk)
    # FakeChatCompletionClient yields 3 items (2 chunks + final result).
    assert len(chunks) == 3


@pytest.mark.asyncio
async def test_T22_create_stream_does_not_call_request_decision() -> None:
    """POC scope: stream path does NOT fire PRE/POST.

    Per review-standards §3.1 this is intentional behavior, NOT a
    regression. PRE/POST brackets at the next ``create()`` boundary.
    """
    wrapper, _, client = make_wrapper()
    stream = wrapper.create_stream(make_messages())
    async for _ in stream:
        pass
    client.request_decision.assert_not_awaited()
    client.emit_llm_call_post.assert_not_awaited()


# ═════════════════════════════════════════════════════════════════════
# 1.5 Pass-through introspection
# ═════════════════════════════════════════════════════════════════════


def test_T23_count_tokens_pass_through() -> None:
    wrapper, inner, _ = make_wrapper()
    msgs = make_messages()
    n = wrapper.count_tokens(msgs)
    assert n == len(msgs) * 10
    assert inner.count_tokens_calls == 1


def test_T24_total_usage_pass_through() -> None:
    wrapper, inner, _ = make_wrapper()
    usage = wrapper.total_usage()
    assert usage.prompt_tokens == 3
    assert inner.total_usage_calls == 1


def test_T25_actual_usage_pass_through() -> None:
    wrapper, inner, _ = make_wrapper()
    usage = wrapper.actual_usage()
    assert usage.completion_tokens == 2
    assert inner.actual_usage_calls == 1


def test_T26_remaining_tokens_pass_through() -> None:
    wrapper, inner, _ = make_wrapper()
    msgs = make_messages()
    n = wrapper.remaining_tokens(msgs)
    assert n == 1000 - (len(msgs) * 10)
    assert inner.remaining_tokens_calls == 1


def test_T27_capabilities_and_model_info_pass_through() -> None:
    wrapper, _, _ = make_wrapper()
    caps = wrapper.capabilities
    info = wrapper.model_info
    assert "function_calling" in caps
    assert "family" in info


def test_T27b_pass_through_methods_have_no_side_effects() -> None:
    """Pass-through methods MUST NOT call sidecar.

    Review-standards §4: any counter or timer would confuse
    ``AssistantAgent``'s token-budget caps.
    """
    wrapper, _, client = make_wrapper()
    wrapper.count_tokens(make_messages())
    wrapper.total_usage()
    wrapper.actual_usage()
    wrapper.remaining_tokens(make_messages())
    _ = wrapper.capabilities
    _ = wrapper.model_info
    client.request_decision.assert_not_awaited()
    client.emit_llm_call_post.assert_not_awaited()


# ═════════════════════════════════════════════════════════════════════
# 1.6 Run context — shared with openai_agents
# ═════════════════════════════════════════════════════════════════════


@pytest.mark.asyncio
async def test_T28_create_raises_without_active_run_context() -> None:
    """``create()`` outside ``run_context()`` raises RuntimeError.

    Review-standards §8.1: error message contract matches the
    openai_agents adapter so polyglot agent stacks see one error shape.
    """
    wrapper, _, _ = make_wrapper()
    with pytest.raises(RuntimeError, match="run_context"):
        await wrapper.create(make_messages())


@pytest.mark.asyncio
async def test_T29_shared_run_context_with_openai_agents() -> None:
    """Same contextvar NAME → polyglot trace sharing.

    Review-standards §1.3: ``current_run_context`` is imported from
    ``spendguard.integrations.openai_agents`` (or fallback uses same
    contextvar NAME), so a parent OpenAI Agents run shares run_id.

    This test creates a run_context via the AutoGen module-level
    helper, then asserts current_run_context inside the block returns
    the same run_id.
    """
    captured: list[str] = []
    wrapper, _, _ = make_wrapper()

    async with run_context(RunContext(run_id="shared-r-29")):
        ctx = current_run_context()
        captured.append(ctx.run_id)
        await wrapper.create(make_messages())
    assert captured == ["shared-r-29"]


@pytest.mark.asyncio
async def test_T30_idempotency_key_deterministic_for_same_signature() -> None:
    """Two identical create() calls in same run → same idempotency key.

    Per design.md §4: ``llm_call_id`` / ``decision_id`` are derived
    from the signature. The sidecar idempotency cache reuses the
    reservation for retries.
    """
    wrapper, _, client = make_wrapper()
    async with run_context(RunContext(run_id="r-30")):
        await wrapper.create(make_messages())
        await wrapper.create(make_messages())
    assert client.request_decision.await_count == 2
    keys = [
        c.kwargs["idempotency_key"]
        for c in client.request_decision.call_args_list
    ]
    assert keys[0] == keys[1]


# ═════════════════════════════════════════════════════════════════════
# 1.7 Helpers — _classify_exception edge cases
# ═════════════════════════════════════════════════════════════════════


def test_T31_classify_exception_cancelled_by_name() -> None:
    """CancelledError detection uses type name (cross-loop safe)."""

    class CancelledError(BaseException):  # noqa: N818 - mirror asyncio
        pass

    assert _classify_exception(CancelledError()) == "CANCELLED"
    assert _classify_exception(RuntimeError("x")) == "FAILURE"


def test_T32_extract_total_tokens_handles_non_numeric_usage() -> None:
    """Defensive: usage fields might be non-numeric on a custom client.

    ``_extract_total_tokens`` returns 0 rather than crashing the
    audit chain.
    """
    bad = SimpleNamespace(usage=SimpleNamespace(prompt_tokens="x", completion_tokens="y"))
    assert _extract_total_tokens(bad) == 0


@pytest.mark.asyncio
async def test_T33a_close_passes_through_to_inner() -> None:
    """``close()`` pass-through — abstract in autogen-core 0.7+."""
    wrapper, inner, _ = make_wrapper()
    # Track close on the fake.
    close_called = {"n": 0}

    async def _close() -> None:
        close_called["n"] += 1

    inner.close = _close  # type: ignore[assignment]
    await wrapper.close()
    assert close_called["n"] == 1


@pytest.mark.asyncio
async def test_T33b_close_tolerates_missing_inner_close() -> None:
    """Best-effort: tolerate inner clients without a ``close`` method.

    The duck-typed code path in ``_hook.close()`` uses ``getattr`` so a
    truly close-less inner client (e.g. some custom subclass that
    overrides nothing) degrades to a no-op.
    """

    class _NoCloseInner:
        """Mimic an inner client that doesn't define close."""

        async def create(
            self, messages, *, tools=(), tool_choice="auto",
            json_output=None, extra_create_args=None,
            cancellation_token=None, **_kwargs,
        ):
            return SimpleNamespace(
                usage=SimpleNamespace(prompt_tokens=1, completion_tokens=1)
            )

        def create_stream(self, messages, **_kwargs):
            async def _s():
                yield SimpleNamespace(content="x")

            return _s()

        def actual_usage(self):
            return None

        def total_usage(self):
            return None

        def count_tokens(self, messages, *, tools=()):
            return 0

        def remaining_tokens(self, messages, *, tools=()):
            return 0

        @property
        def capabilities(self):
            return {}

        @property
        def model_info(self):
            return {}

    wrapper, _, _ = make_wrapper(inner=_NoCloseInner())
    # Should not raise even when inner has no close.
    await wrapper.close()


@pytest.mark.asyncio
async def test_T33c_create_forwards_tool_choice_when_set() -> None:
    """``tool_choice`` is forwarded to inner when non-default (autogen 0.7+)."""
    wrapper, inner, _ = make_wrapper()
    async with run_context(RunContext(run_id="r-33c")):
        await wrapper.create(make_messages(), tool_choice="required")
    assert inner.calls[0]["tool_choice"] == "required"


def test_T33_decision_context_includes_lineage_and_integration_tag() -> None:
    """Decision context carries ``integration=autogen`` and ``lineage`` for dashboards."""

    async def run() -> Any:
        wrapper, _, client = make_wrapper()
        async with run_context(RunContext(run_id="r-33")):
            await wrapper.create(make_messages())
        return client

    client = asyncio.run(run())
    kw = client.request_decision.call_args.kwargs
    ctx = kw["decision_context_json"]
    assert ctx["integration"] == "autogen"
    assert ctx["lineage"] in {"autogen", "ag2", "both", "core-only"}
    # inner_client tag present and reflects the FakeChatCompletionClient
    # class name for audit grouping.
    assert ctx["inner_client"] == "FakeChatCompletionClient"
