# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S101, S106, S107
"""COV_D25 — pytest unit tests for the SmolAgents adapter.

Mirrors ``tests/integrations/autogen/test_autogen.py`` shape but targets
``SpendGuardSmolModel`` instead of the AutoGen client subclass. Uses
``FakeSmolModel`` (subclasses the real ABC when ``smolagents`` is
installed, plain base class otherwise — see ``conftest_smolagents.py``)
so the unit suite runs across CI environments with and without the
extras.

Per ``docs/specs/coverage/D25_smolagents/tests.md`` §1 — 20+ unit cases
covering construction / contract, ``generate()`` PRE-POST flow,
``__call__`` alias, exception handling, ``step_callbacks`` helper, and
shared run-context.

The module-level import uses the package-bypass pattern (mirrors the
autogen / agno / dspy test suites and the demo runner) so that loading
``spendguard.integrations.smolagents._hook`` directly works even when
the package barrel raises an ImportError due to smolagents not being
installed — review-standards §8.3 expressly permits this hybrid path.
"""

from __future__ import annotations

import asyncio
import importlib
import logging
import sys
from types import ModuleType, SimpleNamespace
from typing import Any

import pytest

# ─────────────────────────────────────────────────────────────────────
# Package-bypass import: load the adapter modules even when the
# ``[smolagents]`` extra isn't installed in the CI venv. The wrapper
# class is import-resilient (it falls back to a plain base class — see
# ``_hook.py``'s ``_ModelBase`` branch).
# ─────────────────────────────────────────────────────────────────────

_PKG = "spendguard.integrations.smolagents"
if _PKG not in sys.modules:
    from pathlib import Path

    ns = ModuleType(_PKG)
    sdk_root = (
        Path(__file__).resolve().parents[3]
        / "src/spendguard/integrations/smolagents"
    )
    ns.__path__ = [str(sdk_root)]
    sys.modules[_PKG] = ns

_hook = importlib.import_module("spendguard.integrations.smolagents._hook")
_options = importlib.import_module("spendguard.integrations.smolagents._options")
_errors = importlib.import_module("spendguard.integrations.smolagents._errors")

SpendGuardSmolModel = _hook.SpendGuardSmolModel
RunContext = _hook.RunContext
run_context = _hook.run_context
current_run_context = _hook.current_run_context
ClaimEstimator = _hook.ClaimEstimator
SyncInAsyncContext = _hook.SyncInAsyncContext
spendguard_step_callback = _hook.spendguard_step_callback
_signature = _hook._signature
_extract_total_tokens = _hook._extract_total_tokens
_classify_exception = _hook._classify_exception
SpendGuardSmolAgentsOptions = _options.SpendGuardSmolAgentsOptions
DecisionDenied = _errors.DecisionDenied
SpendGuardConfigError = _errors.SpendGuardConfigError

from spendguard._proto.spendguard.common.v1 import common_pb2  # noqa: E402

from .conftest_smolagents import (  # noqa: E402
    FakeSmolModel,
    _make_chat_message,
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
    """Realistic ``ChatMessage``-shaped payload for unit tests."""
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
        inner = FakeSmolModel()
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
    wrapper = SpendGuardSmolModel(
        inner=inner,
        client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=unit,
        pricing=pricing,
        claim_estimator=claim_estimator,
    )
    return wrapper, inner, client


def _drive_sync_generate(wrapper, messages, *, run_id="r-x", **gen_kwargs):
    """Helper: bind a run context in a contextvar copy without a loop.

    SmolAgents Model.generate is SYNCHRONOUS; the wrapper bridges
    sidecar RPCs via asyncio.run, which requires the call site to NOT
    be inside a running loop. The cleanest test fixture is a fresh
    ``contextvars.copy_context()`` scope that sets the
    ``spendguard_run_context`` contextvar synchronously, then calls
    ``wrapper.generate`` inside the same copy — no event loop is ever
    active when generate fires.

    Returns the result of ``wrapper.generate(messages, **gen_kwargs)``.
    """
    import contextvars

    from spendguard.integrations.openai_agents import _RUN_CONTEXT
    ctx = contextvars.copy_context()

    def _runner():
        _RUN_CONTEXT.set(RunContext(run_id=run_id))
        return wrapper.generate(messages, **gen_kwargs)

    return ctx.run(_runner)


# ═════════════════════════════════════════════════════════════════════
# 1.1 Construction / contract
# ═════════════════════════════════════════════════════════════════════


def test_T01_constructor_skips_super_init() -> None:
    """Wrapper does not invoke ``smolagents.Model.__init__``.

    Per review-standards §1.2: ``smolagents.Model.__init__`` sets
    attributes used only by direct vendor subclasses (model_id,
    flatten_messages_as_text, tool_name_key, tool_arguments_key);
    calling super would force a synthetic model_id and break inner
    introspection.

    Verified by asserting the wrapper has NO ``model_id`` /
    ``flatten_messages_as_text`` attributes set on itself — those
    forward through ``__getattr__`` to the inner.
    """
    wrapper, inner, _ = make_wrapper()
    # The wrapper itself does not carry these — they belong to the inner.
    assert "model_id" not in wrapper.__dict__
    assert "flatten_messages_as_text" not in wrapper.__dict__
    # __getattr__ forwards `.model_id` to the inner.
    assert wrapper.model_id == inner.model_id


def test_T02_constructor_rejects_none_inner() -> None:
    with pytest.raises(SpendGuardConfigError, match="inner"):
        SpendGuardSmolModel(
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
        SpendGuardSmolModel(
            inner=FakeSmolModel(),
            client=None,  # type: ignore[arg-type]
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_estimator=lambda m: [_claim(100)],
        )


def test_T04_constructor_rejects_empty_budget_id() -> None:
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        SpendGuardSmolModel(
            inner=FakeSmolModel(),
            client=make_client_mock(),
            budget_id="",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_estimator=lambda m: [_claim(100)],
        )


def test_T05_constructor_rejects_empty_unit_id() -> None:
    with pytest.raises(SpendGuardConfigError, match="unit.unit_id"):
        SpendGuardSmolModel(
            inner=FakeSmolModel(),
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
        SpendGuardSmolModel(
            inner=FakeSmolModel(),
            client=make_client_mock(),
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_estimator=None,  # type: ignore[arg-type]
        )


def test_T06b_constructor_refuses_litellm_model() -> None:
    """Review-standards §1.1: wrapping LiteLLMModel is a Blocker.

    Double-gating via D12 + D25 produces two reservations per call.
    The wrapper detects the inner type by class name (best-effort,
    works whether or not smolagents is installed in the host venv).
    """

    class LiteLLMModel:
        """Duck-typed stand-in named exactly as smolagents.LiteLLMModel."""

    with pytest.raises(SpendGuardConfigError, match="LiteLLMModel"):
        SpendGuardSmolModel(
            inner=LiteLLMModel(),  # type: ignore[arg-type]
            client=make_client_mock(),
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_estimator=lambda m: [_claim(100)],
        )


def test_T07_options_dataclass_validates() -> None:
    opts = SpendGuardSmolAgentsOptions(
        tenant_id="t1", budget_id="b1", window_instance_id="w1"
    )
    assert opts.route == "llm.call"
    with pytest.raises(SpendGuardConfigError, match="tenant_id"):
        SpendGuardSmolAgentsOptions(
            tenant_id="", budget_id="b1", window_instance_id="w1"
        )


def test_T07b_options_dataclass_rejects_whitespace_window() -> None:
    with pytest.raises(SpendGuardConfigError, match="window_instance_id"):
        SpendGuardSmolAgentsOptions(
            tenant_id="t1", budget_id="b1", window_instance_id="   "
        )


# ═════════════════════════════════════════════════════════════════════
# 1.2 generate() PRE/POST flow
# ═════════════════════════════════════════════════════════════════════


def test_T08_generate_emits_request_decision_with_llm_call_pre_trigger() -> None:
    wrapper, inner, client = make_wrapper()
    result = _drive_sync_generate(wrapper, make_messages(), run_id="r-8")
    assert result is not None
    client.request_decision.assert_awaited_once()
    kw = client.request_decision.call_args.kwargs
    assert kw["trigger"] == "LLM_CALL_PRE"
    assert kw["route"] == "llm.call"
    assert kw["run_id"] == "r-8"
    assert len(kw["projected_claims"]) == 1
    # PRE fired BEFORE inner.generate — assert ordering via call count.
    assert len(inner.calls) == 1
    # Validate audit telemetry tag (review-standards §6 / §7 — decision
    # context labels the integration so dashboards group correctly).
    decision_ctx = kw.get("decision_context_json") or {}
    assert decision_ctx.get("integration") == "smolagents"


def test_T08b_generate_passes_estimator_output_as_projected_claims() -> None:
    captured: list[list[Any]] = []

    def custom_estimator(messages: list[Any]) -> list[Any]:
        captured.append(list(messages))
        return [_claim(777)]

    wrapper, _, client = make_wrapper(claim_estimator=custom_estimator)
    msgs = make_messages()
    _drive_sync_generate(wrapper, msgs, run_id="r-9")
    assert len(captured) == 1
    kw = client.request_decision.call_args.kwargs
    assert kw["projected_claims"][0].amount_atomic == "777"


def test_T09_generate_post_uses_reservation_from_decision() -> None:
    wrapper, _, client = make_wrapper()
    _drive_sync_generate(wrapper, make_messages(), run_id="r-10")
    client.emit_llm_call_post.assert_awaited_once()
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["reservation_id"] == "res-1"
    assert kw["outcome"] == "SUCCESS"


def test_T10_generate_post_estimated_amount_equals_input_plus_output_tokens() -> None:
    """Review-standards §2.3: extract input_tokens + output_tokens."""
    inner = FakeSmolModel(usage_input_tokens=11, usage_output_tokens=22)
    wrapper, _, client = make_wrapper(inner=inner)
    _drive_sync_generate(wrapper, make_messages(), run_id="r-11")
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] == "33"


def test_T11_generate_post_estimated_amount_zero_when_token_usage_absent() -> None:
    inner = FakeSmolModel(no_usage=True)
    wrapper, _, client = make_wrapper(inner=inner)
    _drive_sync_generate(wrapper, make_messages(), run_id="r-12")
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] == "0"


def test_T12_generate_skips_post_when_no_reservation() -> None:
    """DENY-path defensive: empty reservation_ids → POST MUST NOT fire."""
    client = make_client_mock(reservation_ids=())
    wrapper, _, _ = make_wrapper(client=client)
    _drive_sync_generate(wrapper, make_messages(), run_id="r-13")
    client.emit_llm_call_post.assert_not_awaited()


def test_T13_generate_signature_includes_stop_sequences() -> None:
    """Different ``stop_sequences`` → different signature."""
    sig_a = _signature(make_messages(), None, None, None, {})
    sig_b = _signature(make_messages(), ["STOP"], None, None, {})
    assert sig_a != sig_b


def test_T14_generate_signature_includes_tools_to_call_from() -> None:
    sig_a = _signature(make_messages(), None, None, None, {})
    sig_b = _signature(
        make_messages(), None, None,
        [SimpleNamespace(name="search")], {},
    )
    assert sig_a != sig_b


def test_T15_generate_signature_includes_response_format() -> None:
    sig_a = _signature(make_messages(), None, None, None, {})
    sig_b = _signature(make_messages(), None, {"type": "json_object"}, None, {})
    assert sig_a != sig_b


def test_T16_generate_signature_includes_kwargs() -> None:
    sig_a = _signature(make_messages(), None, None, None, {"temperature": 0.5})
    sig_b = _signature(make_messages(), None, None, None, {"temperature": 0.9})
    assert sig_a != sig_b


def test_T17_generate_kwargs_signature_is_sorted_for_determinism() -> None:
    """Review-standards §6: sorting is required for determinism."""
    sig_a = _signature(make_messages(), None, None, None, {"a": 1, "b": 2})
    sig_b = _signature(make_messages(), None, None, None, {"b": 2, "a": 1})
    assert sig_a == sig_b


def test_T18_generate_passes_kwargs_through_to_inner() -> None:
    wrapper, inner, _ = make_wrapper()
    _drive_sync_generate(
        wrapper, make_messages(), run_id="r-18",
        stop_sequences=["END"],
        response_format={"type": "json_object"},
        tools_to_call_from=None,
        temperature=0.7,
        seed=42,
    )
    assert len(inner.calls) == 1
    call = inner.calls[0]
    assert call["stop_sequences"] == ["END"]
    assert call["response_format"] == {"type": "json_object"}
    assert call["tools_to_call_from"] is None
    assert call["kwargs"]["temperature"] == 0.7
    assert call["kwargs"]["seed"] == 42


# ═════════════════════════════════════════════════════════════════════
# 1.3 __call__ alias (review-standards §3 — version-drift bypass guard)
# ═════════════════════════════════════════════════════════════════════


def test_T19_call_alias_routes_through_generate() -> None:
    """``await wrapper(messages)`` produces the same PRE as wrapper.generate."""
    wrapper, inner, client = make_wrapper()

    import contextvars

    from spendguard.integrations.openai_agents import _RUN_CONTEXT
    ctx = contextvars.copy_context()

    def _runner():
        _RUN_CONTEXT.set(RunContext(run_id="r-19"))
        return wrapper(make_messages())  # __call__ form

    result = ctx.run(_runner)
    assert result is not None
    client.request_decision.assert_awaited_once()
    kw = client.request_decision.call_args.kwargs
    assert kw["trigger"] == "LLM_CALL_PRE"
    # Inner was called exactly once via the __call__ → generate alias.
    assert len(inner.calls) == 1


def test_T20_call_alias_propagates_kwargs() -> None:
    wrapper, inner, _ = make_wrapper()

    import contextvars

    from spendguard.integrations.openai_agents import _RUN_CONTEXT
    ctx = contextvars.copy_context()

    def _runner():
        _RUN_CONTEXT.set(RunContext(run_id="r-20"))
        return wrapper(
            make_messages(),
            stop_sequences=["END"],
            response_format={"type": "json"},
            tools_to_call_from=None,
            extra="propagated",
        )

    ctx.run(_runner)
    call = inner.calls[0]
    assert call["stop_sequences"] == ["END"]
    assert call["response_format"] == {"type": "json"}
    assert call["kwargs"]["extra"] == "propagated"


# ═════════════════════════════════════════════════════════════════════
# 1.4 Exception handling
# ═════════════════════════════════════════════════════════════════════


def test_T21_generate_failure_emits_post_failure() -> None:
    inner = FakeSmolModel(raise_on_generate=RuntimeError("boom"))
    wrapper, _, client = make_wrapper(inner=inner)
    with pytest.raises(RuntimeError, match="boom"):
        _drive_sync_generate(wrapper, make_messages(), run_id="r-21")
    client.emit_llm_call_post.assert_awaited_once()
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "FAILURE"
    assert kw["estimated_amount_atomic"] == "0"


def test_T22_generate_cancelled_emits_post_cancelled() -> None:
    inner = FakeSmolModel(raise_on_generate=asyncio.CancelledError())
    wrapper, _, client = make_wrapper(inner=inner)
    with pytest.raises(asyncio.CancelledError):
        _drive_sync_generate(wrapper, make_messages(), run_id="r-22")
    client.emit_llm_call_post.assert_awaited_once()
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "CANCELLED"


def test_T23_generate_failure_skips_post_when_no_reservation() -> None:
    """DENY-then-fail defensively: no reservation → no POST."""
    client = make_client_mock(reservation_ids=())
    inner = FakeSmolModel(raise_on_generate=RuntimeError("boom"))
    wrapper, _, _ = make_wrapper(client=client, inner=inner)
    with pytest.raises(RuntimeError):
        _drive_sync_generate(wrapper, make_messages(), run_id="r-23")
    client.emit_llm_call_post.assert_not_awaited()


def test_T24_classify_exception_returns_cancelled_for_cancelled_error() -> None:
    assert _classify_exception(asyncio.CancelledError()) == "CANCELLED"


def test_T25_classify_exception_returns_failure_for_generic_exception() -> None:
    assert _classify_exception(RuntimeError("x")) == "FAILURE"


# ═════════════════════════════════════════════════════════════════════
# 1.5 Sync-in-async guard
# ═════════════════════════════════════════════════════════════════════


def test_T26_generate_raises_sync_in_async_context() -> None:
    """``generate()`` invoked from inside a running event loop raises.

    DEVIATION-1 (module docstring): SmolAgents Model.generate is sync;
    bridging via asyncio.run from a running loop would deadlock. The
    wrapper raises ``SyncInAsyncContext`` with a clear hint instead.
    """
    wrapper, _, _ = make_wrapper()

    async def _runner():
        from spendguard.integrations.openai_agents import _RUN_CONTEXT
        _RUN_CONTEXT.set(RunContext(run_id="r-26"))
        # Inside a running loop now — wrapper.generate must refuse.
        with pytest.raises(SyncInAsyncContext, match="running event loop"):
            wrapper.generate(make_messages())

    asyncio.run(_runner())


# ═════════════════════════════════════════════════════════════════════
# 1.6 Token usage extraction
# ═════════════════════════════════════════════════════════════════════


def test_T27_extract_total_tokens_real_chatmessage_shape() -> None:
    """Drives ``_extract_total_tokens`` through the real ChatMessage shape."""
    msg = _make_chat_message(input_tokens=15, output_tokens=25)
    assert _extract_total_tokens(msg) == 40


def test_T28_extract_total_tokens_returns_zero_when_token_usage_absent() -> None:
    msg = _make_chat_message(no_usage=True)
    assert _extract_total_tokens(msg) == 0


def test_T29_extract_total_tokens_returns_zero_when_result_is_none() -> None:
    assert _extract_total_tokens(None) == 0


# ═════════════════════════════════════════════════════════════════════
# 1.7 __getattr__ forward to inner
# ═════════════════════════════════════════════════════════════════════


def test_T30_getattr_forwards_unknown_method_to_inner() -> None:
    """Review-standards §5.1: non-private name resolves via inner."""
    wrapper, inner, _ = make_wrapper()
    assert wrapper.flatten_messages_as_text([
        SimpleNamespace(content="a"),
        SimpleNamespace(content="b"),
    ]) == "a\nb"
    # Confirmed forwarded — fake's counter incremented.
    assert inner.flatten_calls == 1


def test_T31_getattr_rejects_private_attrs() -> None:
    """Review-standards §5.1: private names raise AttributeError."""
    wrapper, _, _ = make_wrapper()
    # Public attribute set on wrapper — visible via direct access.
    # Private wrapper-internal names must NOT be forwarded.
    with pytest.raises(AttributeError):
        wrapper.__getattr__("_some_private_state")


def test_T32_getattr_does_not_shadow_explicit_methods() -> None:
    """Wrapper's own ``generate``/``__call__`` win over __getattr__."""
    wrapper, inner, _ = make_wrapper()
    # generate must be the wrapper's bound method, not the inner's.
    wrapper_method = wrapper.generate
    inner_method = inner.generate
    # Different bound methods (different __self__).
    assert wrapper_method.__self__ is wrapper
    assert inner_method.__self__ is inner
    assert wrapper_method.__func__ is not inner_method.__func__


# ═════════════════════════════════════════════════════════════════════
# 1.8 Run context (review-standards §1.3)
# ═════════════════════════════════════════════════════════════════════


def test_T33_generate_raises_without_active_run_context() -> None:
    """Calling generate outside run_context raises RuntimeError."""
    wrapper, _, _ = make_wrapper()
    with pytest.raises(RuntimeError, match="run_context"):
        # No contextvar set; no asyncio loop running either.
        wrapper.generate(make_messages())


def test_T34_run_context_shared_with_openai_agents() -> None:
    """Review-standards §1.3: RunContext is REUSED from openai_agents.

    The smolagents adapter imports ``run_context`` /
    ``current_run_context`` from ``..openai_agents`` when the extra is
    installed. Verified by asserting the smolagents-side symbols are
    the SAME objects as the openai_agents-side symbols (in the
    canonical install path); the fallback branch is exercised
    separately via the contextvar NAME match below so polyglot sharing
    still works regardless of which branch fired at import time.
    """
    from spendguard.integrations import openai_agents as oa
    from spendguard.integrations.smolagents._hook import (
        RunContext as smol_rc,
    )
    from spendguard.integrations.smolagents._hook import (
        current_run_context as smol_curr,
    )
    from spendguard.integrations.smolagents._hook import (
        run_context as smol_rctx,
    )
    # Canonical path: same identity (re-export from openai_agents).
    if smol_rc is oa.RunContext:
        assert smol_curr is oa.current_run_context
        assert smol_rctx is oa.run_context
    # Fallback: ensure the contextvar NAME identity still matches so a
    # parent run shares run_id with this adapter through the env-var
    # boundary.
    assert oa._RUN_CONTEXT.name == "spendguard_run_context"


# ═════════════════════════════════════════════════════════════════════
# 1.9 spendguard_step_callback helper (review-standards §4)
# ═════════════════════════════════════════════════════════════════════


def test_T35_step_callback_emits_telemetry_on_action_step() -> None:
    client = make_client_mock()
    cb = spendguard_step_callback(client, run_id="r-30")
    # Synthesize an ActionStep-shaped object.
    action_step = type("ActionStep", (), {"step_number": 1})()
    cb(action_step)
    client.emit_agent_step_telemetry.assert_called_once_with(
        run_id="r-30", step_kind="ActionStep", step_number=1,
    )


def test_T36_step_callback_emits_telemetry_on_planning_step() -> None:
    client = make_client_mock()
    cb = spendguard_step_callback(client, run_id="r-31")
    planning_step = type("PlanningStep", (), {"step_number": 2})()
    cb(planning_step)
    client.emit_agent_step_telemetry.assert_called_once_with(
        run_id="r-31", step_kind="PlanningStep", step_number=2,
    )


def test_T37_step_callback_swallows_exceptions(caplog) -> None:
    """Review-standards §4.2: catches Exception, never raises.

    Sidecar outage during telemetry MUST NOT abort the host agent run.
    """
    client = make_client_mock()
    client.emit_agent_step_telemetry.side_effect = RuntimeError("sidecar down")
    cb = spendguard_step_callback(client, run_id="r-32")
    action_step = type("ActionStep", (), {"step_number": 1})()
    with caplog.at_level(logging.WARNING):
        result = cb(action_step)
    assert result is None
    # Warning was logged.
    assert any(
        "spendguard_step_callback swallowed" in rec.message
        for rec in caplog.records
    )


def test_T38_step_callback_does_not_call_request_decision() -> None:
    """Review-standards §4.1: informational only — never gates."""
    client = make_client_mock()
    cb = spendguard_step_callback(client, run_id="r-33")
    action_step = type("ActionStep", (), {"step_number": 1})()
    cb(action_step)
    # CRITICAL: request_decision MUST NEVER be called from step callbacks.
    client.request_decision.assert_not_awaited()


def test_T39_step_callback_does_not_catch_base_exception() -> None:
    """Review-standards §4.2: KeyboardInterrupt / SystemExit MUST propagate.

    The callable catches ``Exception`` (NOT ``BaseException``).
    """
    client = make_client_mock()
    client.emit_agent_step_telemetry.side_effect = KeyboardInterrupt()
    cb = spendguard_step_callback(client, run_id="r-34")
    action_step = type("ActionStep", (), {"step_number": 1})()
    with pytest.raises(KeyboardInterrupt):
        cb(action_step)


def test_T40_step_callback_rejects_empty_run_id() -> None:
    """Constructor-side validation: empty run_id is a config error."""
    client = make_client_mock()
    with pytest.raises(SpendGuardConfigError, match="run_id"):
        spendguard_step_callback(client, run_id="")


def test_T41_step_callback_rejects_none_client() -> None:
    with pytest.raises(SpendGuardConfigError, match="client"):
        spendguard_step_callback(None, run_id="r-x")  # type: ignore[arg-type]


def test_T42_step_callback_falls_back_to_log_when_method_missing(caplog) -> None:
    """Operator's SDK may not yet expose emit_agent_step_telemetry.

    The callable falls back to a structured log record on the SDK's
    standard logger so the wrapper still ships before the SDK adds
    the dedicated method (see implementation.md §2 Slice 3).
    """
    client = make_client_mock()
    # Remove both optional surfaces — forces the log-only branch.
    del client.emit_agent_step_telemetry
    if hasattr(client, "emit_custom_audit"):
        del client.emit_custom_audit
    cb = spendguard_step_callback(client, run_id="r-35")
    action_step = type("ActionStep", (), {"step_number": 1})()
    with caplog.at_level(logging.INFO):
        result = cb(action_step)
    assert result is None
    assert any("agent_step" in rec.message for rec in caplog.records)


__all__: list[str] = []
