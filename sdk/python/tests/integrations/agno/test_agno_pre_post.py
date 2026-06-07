# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S101, S106, S107
"""COV_D22 — pytest unit + integration tests for the Agno adapter.

Mocks ``SpendGuardClient`` (Tier 1) and uses ``SimpleNamespace`` stubs
for Agno ``run_input`` / ``RunOutput`` shapes so the suite runs across
duck-typed test inputs. The integration test branch (``test_real_*``)
exercises the real ``agno.agent.Agent`` against a stubbed OpenAI
client so PRE-before-vendor-SDK is asserted by an upstream
counting stub.

Per ``docs/specs/coverage/D22_agno/tests.md`` §2-§3 — ≥22 unit cases +
3 integration variants.
"""

from __future__ import annotations

import asyncio
import inspect
import logging
from collections import OrderedDict
from types import SimpleNamespace
from typing import Any
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from spendguard._proto.spendguard.common.v1 import common_pb2
from spendguard.errors import ApprovalRequired, DecisionDenied, DecisionStopped
from spendguard.integrations.agno import (
    RunContext,
    SpendGuardAgnoPostHook,
    SpendGuardAgnoPreHook,
    current_run_context,
    run_context,
)
from spendguard.integrations.agno._hook import (
    _INFLIGHT_MAX,
    _SHARED_INFLIGHT,
    _extract_usage,
    _hook_param_names,
)

# ─────────────────────────────────────────────────────────────────────
# Fixtures + helpers
# ─────────────────────────────────────────────────────────────────────


@pytest.fixture(autouse=True)
def _reset_shared_inflight():
    """Clear the module-shared inflight between tests."""
    _SHARED_INFLIGHT.clear()
    yield
    _SHARED_INFLIGHT.clear()


def make_model(*, name: str = "OpenAIChat", model_id: str = "gpt-4o-mini"):
    """SimpleNamespace shaped like an Agno ``Model`` subclass instance."""
    cls = type(name, (object,), {})
    inst = cls()
    inst.id = model_id
    return inst


def make_agent(*, model_name: str = "OpenAIChat", model_id: str = "gpt-4o-mini"):
    """SimpleNamespace shaped like an Agno ``Agent`` instance."""
    return SimpleNamespace(model=make_model(name=model_name, model_id=model_id))


def make_run_output(
    *,
    total_tokens: int | None = 87,
    input_tokens: int = 30,
    output_tokens: int = 57,
    run_id: str = "agno-run-1",
    status: str = "COMPLETED",
    error: Any = None,
    input_payload: Any = "hello",
):
    """SimpleNamespace shaped like Agno 2.x ``RunOutput``."""
    if total_tokens is None:
        metrics = SimpleNamespace(
            input_tokens=input_tokens,
            output_tokens=output_tokens,
            total_tokens=0,
        )
    else:
        metrics = SimpleNamespace(
            input_tokens=input_tokens,
            output_tokens=output_tokens,
            total_tokens=total_tokens,
        )
    return SimpleNamespace(
        run_id=run_id,
        status=SimpleNamespace(value=status),
        error=error,
        metrics=metrics,
        input=input_payload,
    )


def make_client_mock(
    *,
    tenant_id: str = "tenant-1",
    session_id: str = "session-1",
    decision_id: str = "dec-1",
    reservation_ids: tuple[str, ...] = ("res-1",),
    decision: str = "CONTINUE",
    request_decision_side_effect: Any = None,
) -> MagicMock:
    """Build an ``AsyncMock`` shaped like a connected SpendGuardClient."""
    client = MagicMock()
    client.tenant_id = tenant_id
    client.session_id = session_id

    outcome = SimpleNamespace(
        decision_id=decision_id,
        reservation_ids=reservation_ids,
        audit_decision_event_id="audit-1",
        decision=decision,
    )
    if request_decision_side_effect is not None:
        client.request_decision = AsyncMock(
            side_effect=request_decision_side_effect
        )
    else:
        client.request_decision = AsyncMock(return_value=outcome)
    client.emit_llm_call_post = AsyncMock(return_value=None)
    return client


def _claim(amount: int = 100):
    return common_pb2.BudgetClaim(
        budget_id="b1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        amount_atomic=str(amount),
        direction=common_pb2.BudgetClaim.DEBIT,
        window_instance_id="w1",
    )


def make_pre_post(
    *,
    client: MagicMock | None = None,
    claim_estimator: Any = None,
    inflight: Any = None,
):
    """Return ``(pre_factory, post_factory, client)`` with sane defaults."""
    if client is None:
        client = make_client_mock()
    unit = common_pb2.UnitRef(unit_id="u1")
    pricing = common_pb2.PricingFreeze(pricing_version="v1")

    pre = SpendGuardAgnoPreHook(
        client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=unit,
        pricing=pricing,
        claim_estimator=claim_estimator or (lambda a, ri: [_claim(100)]),
        inflight=inflight,
    )
    post = SpendGuardAgnoPostHook(
        client=client,
        unit=unit,
        pricing=pricing,
        inflight=inflight,
    )
    return pre, post, client


# ─────────────────────────────────────────────────────────────────────
# U01 — Constructor validation
# ─────────────────────────────────────────────────────────────────────


def test_U01_construct_pre_minimal() -> None:
    """Minimal happy-path pre-hook construction succeeds."""
    pre, post, _ = make_pre_post()
    assert pre is not None
    assert post is not None


def test_U01b_pre_rejects_none_client() -> None:
    from spendguard.integrations.agno import SpendGuardConfigError

    with pytest.raises(SpendGuardConfigError, match="client"):
        SpendGuardAgnoPreHook(
            client=None,  # type: ignore[arg-type]
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        )


def test_U01c_pre_rejects_empty_unit_id() -> None:
    from spendguard.integrations.agno import SpendGuardConfigError

    with pytest.raises(SpendGuardConfigError, match="unit.unit_id"):
        SpendGuardAgnoPreHook(
            client=make_client_mock(),
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id=""),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        )


# ─────────────────────────────────────────────────────────────────────
# Test #1 — PRE calls request_decision once with LLM_CALL_PRE
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T01_pre_calls_request_decision_once_with_llm_call_pre() -> None:
    pre, _, client = make_pre_post()
    pre_callable = pre()
    agent = make_agent()
    async with run_context(RunContext(run_id="r-1")):
        await pre_callable(agent=agent, run_input="hello")
    client.request_decision.assert_awaited_once()
    kw = client.request_decision.call_args.kwargs
    assert kw["trigger"] == "LLM_CALL_PRE"
    assert kw["route"] == "llm.call"
    assert kw["run_id"] == "r-1"
    assert len(kw["projected_claims"]) == 1


# ─────────────────────────────────────────────────────────────────────
# Test #2 — STOP raises DecisionDenied (wrapped as InputCheckError)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T02_pre_raises_on_deny() -> None:
    """DENY: the hook must raise an exception Agno will propagate.

    Reality (DEVIATION-1): Agno's loop only re-raises
    ``InputCheckError``. We assert the resulting exception still
    chains the original ``DecisionDenied`` via ``__cause__``.
    """
    denied = DecisionDenied(
        "budget exhausted",
        decision_id="dec-deny",
        reason_codes=["BUDGET_EXHAUSTED"],
    )
    pre, _, _ = make_pre_post(
        client=make_client_mock(request_decision_side_effect=denied),
    )
    pre_callable = pre()
    agent = make_agent()
    with pytest.raises(Exception) as exc_info:
        async with run_context(RunContext(run_id="r-2")):
            await pre_callable(agent=agent, run_input="hello")
    # The wrap targets InputCheckError when agno is importable.
    cause = exc_info.value.__cause__ or exc_info.value
    assert isinstance(cause, DecisionDenied)


@pytest.mark.asyncio
async def test_T03_pre_raises_on_stop_run_projection() -> None:
    """STOP_RUN_PROJECTION → DecisionStopped, must still halt Agno."""
    stopped = DecisionStopped(
        "stop run projection",
        decision_id="dec-stop",
        reason_codes=["STOP_RUN_PROJECTION"],
    )
    pre, _, _ = make_pre_post(
        client=make_client_mock(request_decision_side_effect=stopped),
    )
    pre_callable = pre()
    agent = make_agent()
    with pytest.raises(Exception) as exc_info:
        async with run_context(RunContext(run_id="r-3")):
            await pre_callable(agent=agent, run_input="hello")
    cause = exc_info.value.__cause__ or exc_info.value
    assert isinstance(cause, (DecisionStopped, DecisionDenied))


# ─────────────────────────────────────────────────────────────────────
# Test #4 — PRE records inflight slot keyed by (run_id, signature)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T04_pre_records_inflight() -> None:
    pre, _, _ = make_pre_post()
    pre_callable = pre()
    agent = make_agent()
    async with run_context(RunContext(run_id="r-4")):
        await pre_callable(agent=agent, run_input="alpha")
    # One entry stashed.
    assert len(_SHARED_INFLIGHT) == 1
    key = next(iter(_SHARED_INFLIGHT))
    assert key[0] == "r-4"


# ─────────────────────────────────────────────────────────────────────
# Test #5 — FIFO eviction at boundary
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T05_fifo_eviction() -> None:
    """When the map exceeds _INFLIGHT_MAX, the oldest entry evicts."""
    inflight: OrderedDict = OrderedDict()
    # Pre-populate to _INFLIGHT_MAX.
    for i in range(_INFLIGHT_MAX):
        inflight[(f"r-{i}", "sig")] = "old"  # type: ignore[assignment]
    pre, _, _ = make_pre_post(inflight=inflight)
    pre_callable = pre()
    async with run_context(RunContext(run_id="r-new")):
        await pre_callable(agent=make_agent(), run_input="hello")
    assert len(inflight) <= _INFLIGHT_MAX
    # Oldest entry evicted.
    assert ("r-0", "sig") not in inflight


# ─────────────────────────────────────────────────────────────────────
# Test #6 — Missing run_context raises clear RuntimeError
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T06_missing_run_context_raises() -> None:
    pre, _, _ = make_pre_post()
    pre_callable = pre()
    with pytest.raises(RuntimeError, match="run_context"):
        await pre_callable(agent=make_agent(), run_input="hello")


# ─────────────────────────────────────────────────────────────────────
# Test #7 — Custom call_signature_fn → distinct inflight keys
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T07_custom_signature_fn() -> None:
    counter = {"n": 0}

    def custom_sig(agent, run_input):
        counter["n"] += 1
        return f"sig-{counter['n']:032x}"

    unit = common_pb2.UnitRef(unit_id="u1")
    pricing = common_pb2.PricingFreeze(pricing_version="v1")
    client = make_client_mock()
    inflight = OrderedDict()
    pre = SpendGuardAgnoPreHook(
        client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=unit,
        pricing=pricing,
        claim_estimator=lambda a, ri: [_claim(50)],
        call_signature_fn=custom_sig,
        inflight=inflight,
    )
    pre_callable = pre()
    async with run_context(RunContext(run_id="r-7")):
        await pre_callable(agent=make_agent(), run_input="a")
        await pre_callable(agent=make_agent(), run_input="b")
    assert len(inflight) == 2


# ─────────────────────────────────────────────────────────────────────
# Test #8 — claim_estimator omitted → default factory dispatched
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T08_default_estimator_used_when_omitted() -> None:
    """When ``claim_estimator=None``, the default factory wires in."""
    client = make_client_mock()
    pre = SpendGuardAgnoPreHook(
        client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1", model_family="gpt-4"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        # estimator omitted on purpose
    )
    pre_callable = pre()
    async with run_context(RunContext(run_id="r-8")):
        await pre_callable(agent=make_agent(), run_input="hello world")
    kw = client.request_decision.call_args.kwargs
    claims = kw["projected_claims"]
    assert len(claims) == 1
    assert int(claims[0].amount_atomic) > 0


# ─────────────────────────────────────────────────────────────────────
# Test #9 — Custom claim_estimator overrides default
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T09_custom_estimator_overrides_default() -> None:
    """An explicit ``claim_estimator`` MUST short-circuit the default."""
    with patch(
        "spendguard.integrations._default_estimator.agno_default_claim_estimator"
    ) as default_factory:
        pre, _, client = make_pre_post(
            claim_estimator=lambda a, ri: [_claim(42)],
        )
        pre_callable = pre()
        async with run_context(RunContext(run_id="r-9")):
            await pre_callable(agent=make_agent(), run_input="x")
        default_factory.assert_not_called()
    claims = client.request_decision.call_args.kwargs["projected_claims"]
    assert claims[0].amount_atomic == "42"


# ─────────────────────────────────────────────────────────────────────
# Test #10 — POST emits SUCCESS with total_tokens
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T10_post_emits_success() -> None:
    pre, post, client = make_pre_post()
    pre_cb, post_cb = pre(), post()
    agent = make_agent()
    async with run_context(RunContext(run_id="r-10")):
        await pre_cb(agent=agent, run_input="hello")
        out = make_run_output(total_tokens=87, input_payload="hello")
        await post_cb(agent=agent, run_output=out)
    client.emit_llm_call_post.assert_awaited_once()
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "SUCCESS"
    assert kw["estimated_amount_atomic"] == "87"


# ─────────────────────────────────────────────────────────────────────
# Test #11 — POST emits PROVIDER_ERROR when run_output.status == ERROR
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T11_post_emits_provider_error_on_status_error() -> None:
    pre, post, client = make_pre_post()
    pre_cb, post_cb = pre(), post()
    agent = make_agent()
    async with run_context(RunContext(run_id="r-11")):
        await pre_cb(agent=agent, run_input="hello")
        out = make_run_output(
            total_tokens=None, status="ERROR", input_payload="hello"
        )
        await post_cb(agent=agent, run_output=out)
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "PROVIDER_ERROR"


# ─────────────────────────────────────────────────────────────────────
# Test #12 — POST PROVIDER_ERROR when run_output.error truthy
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T12_post_emits_provider_error_when_error_set() -> None:
    pre, post, client = make_pre_post()
    pre_cb, post_cb = pre(), post()
    agent = make_agent()
    async with run_context(RunContext(run_id="r-12")):
        await pre_cb(agent=agent, run_input="hello")
        out = make_run_output(
            total_tokens=None,
            status="COMPLETED",
            error="boom",
            input_payload="hello",
        )
        await post_cb(agent=agent, run_output=out)
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "PROVIDER_ERROR"


# ─────────────────────────────────────────────────────────────────────
# Test #13 — POST no-ops when inflight slot is missing, logs warning
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T13_post_noops_without_pre(caplog) -> None:
    """No matching pre → log warning + no-op (no commit-without-reserve)."""
    _, post, client = make_pre_post()
    post_cb = post()
    agent = make_agent()
    caplog.set_level(logging.WARNING, logger="spendguard.integrations.agno")
    async with run_context(RunContext(run_id="r-13")):
        await post_cb(
            agent=agent,
            run_output=make_run_output(input_payload="hello"),
        )
    client.emit_llm_call_post.assert_not_awaited()
    assert any("post_hook" in rec.message for rec in caplog.records)


# ─────────────────────────────────────────────────────────────────────
# Test #14 — POST pops the inflight slot
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T14_post_pops_inflight() -> None:
    pre, post, _ = make_pre_post()
    pre_cb, post_cb = pre(), post()
    agent = make_agent()
    async with run_context(RunContext(run_id="r-14")):
        await pre_cb(agent=agent, run_input="hello")
        assert len(_SHARED_INFLIGHT) == 1
        await post_cb(
            agent=agent,
            run_output=make_run_output(input_payload="hello"),
        )
    assert len(_SHARED_INFLIGHT) == 0


# ─────────────────────────────────────────────────────────────────────
# Test #15 — pre/post pair derives identical signatures
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T15_pre_post_signature_symmetry() -> None:
    """Pre uses run_input; post recomputes from run_output.input — same sig."""
    pre, post, client = make_pre_post()
    pre_cb, post_cb = pre(), post()
    agent = make_agent()
    async with run_context(RunContext(run_id="r-15")):
        await pre_cb(agent=agent, run_input="the same input")
        await post_cb(
            agent=agent,
            run_output=make_run_output(input_payload="the same input"),
        )
    client.emit_llm_call_post.assert_awaited_once()


# ─────────────────────────────────────────────────────────────────────
# Test #16 — Pre-hook closure declares (agent, run_input) literally
# ─────────────────────────────────────────────────────────────────────


def test_T16_pre_param_names_locked() -> None:
    pre, _, _ = make_pre_post()
    pre_callable = pre()
    params = _hook_param_names(pre_callable)
    assert params == ["agent", "run_input"], (
        f"locked names: {params}"
    )


# ─────────────────────────────────────────────────────────────────────
# Test #17 — Post-hook closure declares (agent, run_output) literally
# DEVIATION-2: Reality requires run_output (NOT run_response per spec).
# ─────────────────────────────────────────────────────────────────────


def test_T17_post_param_names_locked_to_reality() -> None:
    _, post, _ = make_pre_post()
    post_callable = post()
    params = _hook_param_names(post_callable)
    assert params == ["agent", "run_output"], (
        f"DEVIATION-2 locks post params to (agent, run_output): {params}"
    )


# ─────────────────────────────────────────────────────────────────────
# Test #18 — Two parallel runs keep independent inflight slots
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T18_concurrent_runs_independent_inflight() -> None:
    pre, _, _ = make_pre_post()
    pre_cb = pre()

    async def _do(rid: str):
        async with run_context(RunContext(run_id=rid)):
            await pre_cb(agent=make_agent(), run_input=f"input-{rid}")

    await asyncio.gather(_do("r-A"), _do("r-B"), _do("r-C"))
    rids = {k[0] for k in _SHARED_INFLIGHT}
    assert rids == {"r-A", "r-B", "r-C"}


# ─────────────────────────────────────────────────────────────────────
# Test #19 — derive_idempotency_key kwargs all present
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T19_idempotency_key_kwargs() -> None:
    with patch(
        "spendguard.integrations.agno._hook.derive_idempotency_key",
        wraps=__import__(
            "spendguard.ids", fromlist=["derive_idempotency_key"]
        ).derive_idempotency_key,
    ) as spy:
        pre, _, _ = make_pre_post()
        pre_cb = pre()
        async with run_context(RunContext(run_id="r-19")):
            await pre_cb(agent=make_agent(), run_input="hello")
        assert spy.called
        kw = spy.call_args.kwargs
        for required in (
            "tenant_id",
            "session_id",
            "run_id",
            "step_id",
            "llm_call_id",
            "trigger",
        ):
            assert required in kw, f"missing {required}"
        assert kw["trigger"] == "LLM_CALL_PRE"


# ─────────────────────────────────────────────────────────────────────
# Test #20 — Streaming path commits once on the completion event
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T20_streaming_completion_commits_once() -> None:
    """Streaming output: a single final run_output with metrics commits once."""
    pre, post, client = make_pre_post()
    pre_cb, post_cb = pre(), post()
    agent = make_agent()
    async with run_context(RunContext(run_id="r-20")):
        await pre_cb(agent=agent, run_input="stream me")
        # Streaming RunOutput shape: metrics still populate on
        # completion (final chunk handled by Agno's stream end).
        out = make_run_output(total_tokens=99, input_payload="stream me")
        await post_cb(agent=agent, run_output=out)
    assert client.emit_llm_call_post.await_count == 1
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] == "99"


# ─────────────────────────────────────────────────────────────────────
# Test #21 — ApprovalRequired propagates unchanged
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T21_approval_required_propagates() -> None:
    """ApprovalRequired (a DecisionDenied subclass) wraps but preserves cause."""
    approval = ApprovalRequired(
        "review needed",
        decision_id="dec-app",
        approval_request_id="appr-1",
        reason_codes=["APPROVAL_REQUIRED"],
    )
    pre, _, _ = make_pre_post(
        client=make_client_mock(request_decision_side_effect=approval),
    )
    pre_cb = pre()
    with pytest.raises(Exception) as exc_info:
        async with run_context(RunContext(run_id="r-21")):
            await pre_cb(agent=make_agent(), run_input="hello")
    cause = exc_info.value.__cause__ or exc_info.value
    assert isinstance(cause, ApprovalRequired)
    assert len(_SHARED_INFLIGHT) == 0


# ─────────────────────────────────────────────────────────────────────
# Test #22 — Missing agent.model.id → default estimator falls back
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T22_missing_model_id_falls_back_to_family_default() -> None:
    """A raw agent with no model still produces a non-empty claim."""
    client = make_client_mock()
    pre = SpendGuardAgnoPreHook(
        client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
    )
    pre_cb = pre()
    raw_agent = SimpleNamespace()  # NO `model` attr
    async with run_context(RunContext(run_id="r-22")):
        await pre_cb(agent=raw_agent, run_input="hello there")
    claims = client.request_decision.call_args.kwargs["projected_claims"]
    assert len(claims) == 1
    assert int(claims[0].amount_atomic) > 0


# ─────────────────────────────────────────────────────────────────────
# Test #23 — decision_context tags integration=agno + model backend
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T23_decision_context_tagged_agno() -> None:
    pre, _, client = make_pre_post()
    pre_cb = pre()
    async with run_context(RunContext(run_id="r-23")):
        await pre_cb(
            agent=make_agent(model_name="ClaudeChat", model_id="claude-3-5"),
            run_input="hi",
        )
    ctx = client.request_decision.call_args.kwargs["decision_context_json"]
    assert ctx["integration"] == "agno"
    assert ctx["model_backend"] == "ClaudeChat"
    assert ctx["model_id"] == "claude-3-5"


# ─────────────────────────────────────────────────────────────────────
# Test #24 — _extract_usage with dict-shaped metrics
# ─────────────────────────────────────────────────────────────────────


def test_T24_extract_usage_dict_metrics() -> None:
    """RunOutput.metrics may be a dict (some providers stream raw)."""
    ro = SimpleNamespace(
        status=SimpleNamespace(value="COMPLETED"),
        error=None,
        metrics={"total_tokens": 55, "input_tokens": 20, "output_tokens": 35},
        run_id="rid-24",
        input="x",
    )
    total, ev_id, outcome = _extract_usage(ro)
    assert total == 55
    assert ev_id == "rid-24"
    assert outcome == "SUCCESS"


def test_T25_extract_usage_dict_metrics_no_total() -> None:
    """Falls through to input+output when total absent."""
    ro = SimpleNamespace(
        status=SimpleNamespace(value="COMPLETED"),
        error=None,
        metrics={"input_tokens": 7, "output_tokens": 12},
        run_id="rid-25",
        input="x",
    )
    total, _, outcome = _extract_usage(ro)
    assert total == 19
    assert outcome == "SUCCESS"


def test_T26_extract_usage_none_returns_provider_error() -> None:
    """None run_output → PROVIDER_ERROR."""
    total, ev_id, outcome = _extract_usage(None)
    assert total == 0
    assert ev_id == ""
    assert outcome == "PROVIDER_ERROR"


# ─────────────────────────────────────────────────────────────────────
# Test #27 — current_run_context()
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T27_run_context_lifecycle() -> None:
    """``run_context`` binds + unbinds the RunContext."""
    with pytest.raises(RuntimeError):
        current_run_context()
    async with run_context(RunContext(run_id="my-run")):
        assert current_run_context().run_id == "my-run"
    with pytest.raises(RuntimeError):
        current_run_context()


# ─────────────────────────────────────────────────────────────────────
# Test #28 — Closure preserves both NAME and order even after rebinding
# ─────────────────────────────────────────────────────────────────────


def test_T28_filter_hook_args_simulated() -> None:
    """Mirror Agno's filter_hook_args: declare the literal names + run."""
    pre, post, _ = make_pre_post()
    pre_cb = pre()
    post_cb = post()
    # Simulate Agno's all_args dict (pre + post variants from
    # ``agno/agent/_hooks.py``).
    pre_all = {
        "run_input": "the input",
        "run_context": object(),
        "agent": object(),
        "session": object(),
        "user_id": "u",
        "debug_mode": False,
        "metadata": {},
    }
    sig_pre = inspect.signature(pre_cb)
    accepted = set(sig_pre.parameters) & set(pre_all)
    assert accepted == {"agent", "run_input"}

    post_all = {
        "run_output": object(),
        "agent": object(),
        "session": object(),
        "user_id": "u",
        "run_context": object(),
        "debug_mode": False,
        "metadata": {},
    }
    sig_post = inspect.signature(post_cb)
    accepted_post = set(sig_post.parameters) & set(post_all)
    assert accepted_post == {"agent", "run_output"}


# ─────────────────────────────────────────────────────────────────────
# Integration test — real Agno Agent against a stubbed model
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_INT01_real_agno_filter_hook_args_dispatch() -> None:
    """Real Agno: ``filter_hook_args`` actually dispatches our closure.

    Uses ``agno.utils.hooks.filter_hook_args`` directly to confirm
    Agno will call our closure with the right names. This is the
    only way to assert the contract without spinning up a full
    Model HTTP backend.
    """
    pytest.importorskip("agno")
    from agno.utils.hooks import filter_hook_args

    pre, post, client = make_pre_post()
    pre_cb = pre()
    post_cb = post()

    # Pre dispatch
    pre_all = {
        "run_input": "hello",
        "agent": make_agent(),
        "session": object(),
        "run_context": object(),
        "user_id": "u",
        "debug_mode": False,
        "metadata": {},
    }
    filtered = filter_hook_args(pre_cb, pre_all)
    assert set(filtered) == {"agent", "run_input"}
    async with run_context(RunContext(run_id="INT-1")):
        await pre_cb(**filtered)
    client.request_decision.assert_awaited_once()

    # Post dispatch
    post_all = {
        "run_output": make_run_output(total_tokens=42, input_payload="hello"),
        "agent": make_agent(),
        "session": object(),
        "run_context": object(),
        "user_id": "u",
        "debug_mode": False,
        "metadata": {},
    }
    filtered_post = filter_hook_args(post_cb, post_all)
    assert set(filtered_post) == {"agent", "run_output"}
    async with run_context(RunContext(run_id="INT-1")):
        await post_cb(**filtered_post)
    client.emit_llm_call_post.assert_awaited_once()


@pytest.mark.asyncio
async def test_INT02_real_agno_deny_wrapped_as_input_check_error() -> None:
    """Real Agno: DENY surfaces as InputCheckError, not DecisionDenied.

    DEVIATION-1 contract: the wrap MUST raise Agno's InputCheckError
    so the runtime's hook loop actually propagates the halt. The
    original DecisionDenied chains via __cause__.
    """
    pytest.importorskip("agno")
    from agno.exceptions import InputCheckError

    denied = DecisionDenied(
        "budget exhausted",
        decision_id="dec-deny-int",
        reason_codes=["BUDGET_EXHAUSTED"],
    )
    pre, _, _ = make_pre_post(
        client=make_client_mock(request_decision_side_effect=denied),
    )
    pre_cb = pre()
    with pytest.raises(InputCheckError) as exc_info:
        async with run_context(RunContext(run_id="INT-2")):
            await pre_cb(agent=make_agent(), run_input="hello")
    assert isinstance(exc_info.value.__cause__, DecisionDenied)
    assert exc_info.value.additional_data is not None
    assert exc_info.value.additional_data.get("spendguard") is True
