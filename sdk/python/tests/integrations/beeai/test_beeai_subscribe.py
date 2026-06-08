# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S101, S106, S107
"""COV_D23 — pytest unit tests for the BeeAI ``subscribe_spendguard``.

Mocks ``SpendGuardClient`` (Tier 1) and uses ``SimpleNamespace`` stubs
for BeeAI ``Emitter`` / ``EventMeta`` shapes so the suite runs without
the ``[beeai]`` extra installed (the test imports the ``_hook`` module
directly via package-namespace bypass — same pattern as the
``agent_real_beeai`` demo driver and the existing ``agno`` /
``dspy`` test suites use).

Per ``docs/specs/coverage/D23_beeai/review-standards.md`` §10 the
suite must cover at least:
  - Construction validation (S, L, R contracts)
  - Reserve-before-provider-HTTP (S1)
  - DENY propagation unchanged (S2)
  - Idempotency key derivation kwargs (D1)
  - ``llm_call_id`` vs ``decision_id`` distinct scopes (D2)
  - Stable per-call key strips trailing segment only (D3)
  - Lifecycle correctness — start/success/error pairing (L1-L4)
  - run_context contract (R1-R3)
  - Public surface stability (P1-P3)

This file ships ≥18 unit cases.
"""

from __future__ import annotations

import asyncio
import importlib
import logging
import sys
import types
from collections.abc import Callable
from types import SimpleNamespace
from typing import Any
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

# ─────────────────────────────────────────────────────────────────────
# Package-namespace bypass — the BeeAI ``__init__.py`` raises
# ImportError when ``beeai-framework`` is missing. The test suite
# bypasses that gate by registering a synthetic package whose
# ``__path__`` points at the integration source dir, then importing
# the inner ``_hook`` / ``_options`` / ``_errors`` modules directly.
# This is the same pattern the ``agent_real_beeai`` demo driver uses
# in the CI smoke path.
# ─────────────────────────────────────────────────────────────────────

_PKG_NAME = "spendguard.integrations.beeai"
if _PKG_NAME not in sys.modules or not hasattr(sys.modules[_PKG_NAME], "_hook"):
    # If the real __init__ already loaded (because beeai-framework IS
    # installed) leave it alone; otherwise install the bypass.
    try:
        importlib.import_module(_PKG_NAME)
    except ImportError:
        from pathlib import Path as _P

        ns = types.ModuleType(_PKG_NAME)
        ns.__path__ = [
            str(
                _P(__file__).resolve().parents[3]
                / "src/spendguard/integrations/beeai"
            )
        ]
        sys.modules[_PKG_NAME] = ns

_hook = importlib.import_module("spendguard.integrations.beeai._hook")
_options = importlib.import_module("spendguard.integrations.beeai._options")
_errors = importlib.import_module("spendguard.integrations.beeai._errors")

from spendguard._proto.spendguard.common.v1 import common_pb2  # noqa: E402
from spendguard.errors import ApprovalRequired, DecisionDenied, DecisionStopped  # noqa: E402

BeeAiStartEvent = _hook.BeeAiStartEvent
RunContext = _options.RunContext
SpendGuardBeeAIOptions = _options.SpendGuardBeeAIOptions
SpendGuardConfigError = _errors.SpendGuardConfigError
current_run_context = _hook.current_run_context
run_context = _hook.run_context
subscribe_spendguard = _hook.subscribe_spendguard
_SHARED_INFLIGHT = _hook._SHARED_INFLIGHT
_InflightMap = _hook._InflightMap
_INFLIGHT_MAX = _hook._INFLIGHT_MAX
_stable_call_key = _hook._stable_call_key
_extract_usage_success = _hook._extract_usage_success


# ─────────────────────────────────────────────────────────────────────
# Fixtures + helpers
# ─────────────────────────────────────────────────────────────────────


@pytest.fixture(autouse=True)
def _reset_shared_inflight() -> None:
    """Clear the module-shared inflight between tests."""
    _SHARED_INFLIGHT.clear()
    yield
    _SHARED_INFLIGHT.clear()


def _claim(amount: int = 100):
    return common_pb2.BudgetClaim(
        budget_id="b1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        amount_atomic=str(amount),
        direction=common_pb2.BudgetClaim.DEBIT,
        window_instance_id="w1",
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
    """Build an AsyncMock shaped like a connected SpendGuardClient."""
    client = MagicMock()
    client.tenant_id = tenant_id
    client.session_id = session_id

    outcome = SimpleNamespace(
        decision_id=decision_id,
        reservation_ids=list(reservation_ids),
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


class _FakeEmitter:
    """Duck-typed ``Emitter`` whose ``match()`` returns a cleanup fn."""

    def __init__(self) -> None:
        self.predicates: list[Callable[[Any], bool]] = []
        self.callbacks: list[Callable[..., Any]] = []
        self.unsubscribe_calls = 0

    def match(
        self, matcher: Callable[[Any], bool], callback: Callable[..., Any]
    ) -> Callable[[], None]:
        self.predicates.append(matcher)
        self.callbacks.append(callback)
        idx = len(self.predicates) - 1

        def _unsub() -> None:
            self.unsubscribe_calls += 1
            # Mark slot as gone so further emits don't dispatch.
            self.predicates[idx] = lambda _ev: False

        return _unsub

    async def emit(self, name: str, data: Any, path: str) -> None:
        meta = SimpleNamespace(name=name, path=path, id=f"evt-{name}")
        for pred, cb in zip(self.predicates, self.callbacks, strict=False):
            try:
                if pred(meta):
                    await cb(data, meta)
            except DecisionDenied:
                # BeeAI's real Emitter wraps as EmitterError; for tests
                # we propagate the raw DecisionDenied so assertions
                # can target the exact type.
                raise


def make_agent() -> SimpleNamespace:
    return SimpleNamespace(emitter=_FakeEmitter())


def make_kwargs(client: MagicMock | None = None) -> dict[str, Any]:
    return dict(
        client=client or make_client_mock(),
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        claim_estimator=lambda ev: [_claim(100)],
    )


# ─────────────────────────────────────────────────────────────────────
# T01 — Construction happy path returns unsubscribe callable
# ─────────────────────────────────────────────────────────────────────


def test_T01_subscribe_returns_unsubscribe() -> None:
    agent = make_agent()
    unsub = subscribe_spendguard(agent=agent, **make_kwargs())
    assert callable(unsub)
    assert len(agent.emitter.predicates) == 1


# ─────────────────────────────────────────────────────────────────────
# T02 — Construction validation: empty unit_id rejected
# ─────────────────────────────────────────────────────────────────────


def test_T02_rejects_empty_unit_id() -> None:
    agent = make_agent()
    with pytest.raises(SpendGuardConfigError, match="unit.unit_id"):
        subscribe_spendguard(
            agent=agent,
            client=make_client_mock(),
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id=""),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        )


# ─────────────────────────────────────────────────────────────────────
# T03 — Construction validation: missing budget_id rejected
# ─────────────────────────────────────────────────────────────────────


def test_T03_rejects_missing_budget_id() -> None:
    agent = make_agent()
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        subscribe_spendguard(
            agent=agent,
            client=make_client_mock(),
            budget_id="",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        )


# ─────────────────────────────────────────────────────────────────────
# T04 — Construction validation: agent.emitter missing rejected
# ─────────────────────────────────────────────────────────────────────


def test_T04_rejects_agent_without_emitter() -> None:
    bad_agent = SimpleNamespace()  # no .emitter
    with pytest.raises(SpendGuardConfigError, match="emitter"):
        subscribe_spendguard(agent=bad_agent, **make_kwargs())


# ─────────────────────────────────────────────────────────────────────
# T05 — PRE calls request_decision once with LLM_CALL_PRE
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T05_pre_calls_request_decision_once_with_llm_call_pre() -> None:
    agent = make_agent()
    client = make_client_mock()
    subscribe_spendguard(agent=agent, **make_kwargs(client=client))
    async with run_context(RunContext(run_id="r-5")):
        await agent.emitter.emit(
            "start",
            SimpleNamespace(input=["hi"], modelId="gpt-4o-mini"),
            "agent.react.llm.001.start",
        )
    client.request_decision.assert_awaited_once()
    kw = client.request_decision.call_args.kwargs
    assert kw["trigger"] == "LLM_CALL_PRE"
    assert kw["route"] == "llm.call"
    assert kw["run_id"] == "r-5"
    assert len(kw["projected_claims"]) == 1


# ─────────────────────────────────────────────────────────────────────
# T06 — DENY raises DecisionDenied unchanged (security S2)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T06_pre_propagates_decision_denied() -> None:
    """DENY surfaces as DecisionDenied; the start handler MUST NOT swallow."""
    denied = DecisionDenied(
        "budget exhausted",
        decision_id="dec-deny",
        reason_codes=["BUDGET_EXHAUSTED"],
    )
    agent = make_agent()
    client = make_client_mock(request_decision_side_effect=denied)
    subscribe_spendguard(agent=agent, **make_kwargs(client=client))
    with pytest.raises(DecisionDenied):
        async with run_context(RunContext(run_id="r-6")):
            await agent.emitter.emit(
                "start",
                SimpleNamespace(input=["hi"], modelId="gpt-4o-mini"),
                "agent.react.llm.002.start",
            )


# ─────────────────────────────────────────────────────────────────────
# T07 — STOP_RUN_PROJECTION (DecisionStopped) propagates unchanged
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T07_pre_propagates_decision_stopped() -> None:
    stopped = DecisionStopped(
        "stop run projection",
        decision_id="dec-stop",
        reason_codes=["STOP_RUN_PROJECTION"],
    )
    agent = make_agent()
    client = make_client_mock(request_decision_side_effect=stopped)
    subscribe_spendguard(agent=agent, **make_kwargs(client=client))
    with pytest.raises((DecisionStopped, DecisionDenied)):
        async with run_context(RunContext(run_id="r-7")):
            await agent.emitter.emit(
                "start",
                SimpleNamespace(input=["hi"], modelId="gpt-4o-mini"),
                "agent.react.llm.003.start",
            )


# ─────────────────────────────────────────────────────────────────────
# T08 — PRE records inflight slot keyed by stripped path
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T08_pre_records_inflight_keyed_by_stripped_path() -> None:
    agent = make_agent()
    subscribe_spendguard(agent=agent, **make_kwargs())
    async with run_context(RunContext(run_id="r-8")):
        await agent.emitter.emit(
            "start",
            SimpleNamespace(input=["hi"], modelId="gpt-4o-mini"),
            "agent.react.llm.42.start",
        )
    assert len(_SHARED_INFLIGHT) == 1
    # The inflight key is the path with `.start` stripped.
    assert "agent.react.llm.42" in _SHARED_INFLIGHT


# ─────────────────────────────────────────────────────────────────────
# T09 — Missing run_context raises clear RuntimeError (R2)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T09_missing_run_context_raises_runtime_error() -> None:
    agent = make_agent()
    subscribe_spendguard(agent=agent, **make_kwargs())
    with pytest.raises(RuntimeError, match="run_context"):
        await agent.emitter.emit(
            "start",
            SimpleNamespace(input=["hi"], modelId="gpt-4o-mini"),
            "agent.react.llm.10.start",
        )


# ─────────────────────────────────────────────────────────────────────
# T10 — SUCCESS commits with total_tokens + provider_event_id (L1)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T10_success_commits_with_total_tokens() -> None:
    agent = make_agent()
    client = make_client_mock()
    subscribe_spendguard(agent=agent, **make_kwargs(client=client))
    async with run_context(RunContext(run_id="r-10")):
        await agent.emitter.emit(
            "start",
            SimpleNamespace(input=["hi"], modelId="gpt-4o-mini"),
            "agent.react.llm.99.start",
        )
        await agent.emitter.emit(
            "success",
            SimpleNamespace(
                usage={"total_tokens": 87},
                id="chatcmpl-stub-1",
            ),
            "agent.react.llm.99.success",
        )
    client.emit_llm_call_post.assert_awaited_once()
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "SUCCESS"
    assert kw["estimated_amount_atomic"] == "87"
    assert kw["provider_event_id"] == "chatcmpl-stub-1"


# ─────────────────────────────────────────────────────────────────────
# T11 — ERROR commits PROVIDER_ERROR (L1)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T11_error_commits_provider_error() -> None:
    agent = make_agent()
    client = make_client_mock()
    subscribe_spendguard(agent=agent, **make_kwargs(client=client))
    async with run_context(RunContext(run_id="r-11")):
        await agent.emitter.emit(
            "start",
            SimpleNamespace(input=["hi"], modelId="gpt-4o-mini"),
            "agent.react.llm.99.start",
        )
        await agent.emitter.emit(
            "error",
            SimpleNamespace(message="provider 500"),
            "agent.react.llm.99.error",
        )
    client.emit_llm_call_post.assert_awaited_once()
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "PROVIDER_ERROR"
    assert kw["estimated_amount_atomic"] == "0"


# ─────────────────────────────────────────────────────────────────────
# T12 — SUCCESS without matching start no-ops + warns (L2)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T12_success_without_start_noops(caplog) -> None:
    agent = make_agent()
    client = make_client_mock()
    subscribe_spendguard(agent=agent, **make_kwargs(client=client))
    caplog.set_level(logging.WARNING, logger="spendguard.integrations.beeai")
    async with run_context(RunContext(run_id="r-12")):
        await agent.emitter.emit(
            "success",
            SimpleNamespace(usage={"total_tokens": 87}),
            "agent.react.llm.999.success",
        )
    client.emit_llm_call_post.assert_not_awaited()
    assert any(
        "success event without matching start" in rec.message
        for rec in caplog.records
    )


# ─────────────────────────────────────────────────────────────────────
# T13 — Predicate filters non-llm paths (review §3 L1 anti-scope)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T13_predicate_filters_non_llm_paths() -> None:
    agent = make_agent()
    client = make_client_mock()
    subscribe_spendguard(agent=agent, **make_kwargs(client=client))
    async with run_context(RunContext(run_id="r-13")):
        # tool.start should NOT fire the subscriber (no llm in path).
        await agent.emitter.emit(
            "start",
            SimpleNamespace(input=["tool"], modelId=""),
            "agent.react.tool.001.start",
        )
    client.request_decision.assert_not_awaited()


# ─────────────────────────────────────────────────────────────────────
# T14 — Predicate filters non-{start,success,error} events
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T14_predicate_filters_newtoken_partial_update() -> None:
    agent = make_agent()
    client = make_client_mock()
    subscribe_spendguard(agent=agent, **make_kwargs(client=client))
    async with run_context(RunContext(run_id="r-14")):
        await agent.emitter.emit(
            "newToken",
            SimpleNamespace(token="hi"),
            "agent.react.llm.1.newToken",
        )
        await agent.emitter.emit(
            "partialUpdate",
            SimpleNamespace(delta="hi"),
            "agent.react.llm.1.partialUpdate",
        )
    client.request_decision.assert_not_awaited()
    client.emit_llm_call_post.assert_not_awaited()


# ─────────────────────────────────────────────────────────────────────
# T15 — Stable per-call key strips ONLY trailing segment (D3)
# ─────────────────────────────────────────────────────────────────────


def test_T15_stable_call_key_strips_only_trailing_segment() -> None:
    assert _stable_call_key("a.b.c.start") == "a.b.c"
    assert _stable_call_key("a.b.c.success") == "a.b.c"
    assert _stable_call_key("a.b.c.error") == "a.b.c"
    # Edge: single segment unchanged.
    assert _stable_call_key("start") == "start"
    # Edge: deep path strips ONLY the trailing segment.
    assert _stable_call_key("agent.react.llm.uuid-1.start") == \
        "agent.react.llm.uuid-1"


# ─────────────────────────────────────────────────────────────────────
# T16 — llm_call_id vs decision_id derived from DISTINCT scopes (D2)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T16_llm_call_id_distinct_from_decision_id() -> None:
    agent = make_agent()
    client = make_client_mock()
    subscribe_spendguard(agent=agent, **make_kwargs(client=client))
    async with run_context(RunContext(run_id="r-16")):
        await agent.emitter.emit(
            "start",
            SimpleNamespace(input=["hi"], modelId="gpt-4o-mini"),
            "agent.react.llm.501.start",
        )
    kw = client.request_decision.call_args.kwargs
    assert kw["llm_call_id"] != kw["decision_id"], (
        "llm_call_id and decision_id must derive from DISTINCT scopes"
    )
    # Both UUID-shaped (8-4-4-4-12).
    for v in (kw["llm_call_id"], kw["decision_id"]):
        assert len(v) == 36
        assert v.count("-") == 4


# ─────────────────────────────────────────────────────────────────────
# T17 — derive_idempotency_key kwargs all present (D1)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T17_idempotency_key_derivation_kwargs() -> None:
    with patch(
        "spendguard.integrations.beeai._hook.derive_idempotency_key",
        wraps=__import__(
            "spendguard.ids", fromlist=["derive_idempotency_key"]
        ).derive_idempotency_key,
    ) as spy:
        agent = make_agent()
        client = make_client_mock()
        subscribe_spendguard(agent=agent, **make_kwargs(client=client))
        async with run_context(RunContext(run_id="r-17")):
            await agent.emitter.emit(
                "start",
                SimpleNamespace(input=["hi"], modelId="gpt-4o-mini"),
                "agent.react.llm.601.start",
            )
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
# T18 — unsubscribe() actually unhooks (L3)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T18_unsubscribe_actually_unhooks() -> None:
    agent = make_agent()
    client = make_client_mock()
    unsub = subscribe_spendguard(agent=agent, **make_kwargs(client=client))
    unsub()
    assert agent.emitter.unsubscribe_calls == 1
    # After unsubscribe, further emits MUST NOT call the subscriber.
    async with run_context(RunContext(run_id="r-18")):
        await agent.emitter.emit(
            "start",
            SimpleNamespace(input=["hi"], modelId="gpt-4o-mini"),
            "agent.react.llm.701.start",
        )
    client.request_decision.assert_not_awaited()


# ─────────────────────────────────────────────────────────────────────
# T19 — FIFO eviction at capacity (L4)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T19_fifo_eviction_at_capacity() -> None:
    """When the map exceeds _INFLIGHT_MAX, the oldest entry evicts."""
    inflight = _InflightMap(capacity=2)
    agent = make_agent()
    client = make_client_mock()
    subscribe_spendguard(
        agent=agent,
        client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        claim_estimator=lambda ev: [_claim(100)],
        inflight=inflight,
    )
    async with run_context(RunContext(run_id="r-19")):
        for i in range(4):
            await agent.emitter.emit(
                "start",
                SimpleNamespace(input=[f"hi-{i}"], modelId="gpt-4o-mini"),
                f"agent.react.llm.{i}.start",
            )
    assert len(inflight) == 2
    # Earliest entries evicted.
    assert "agent.react.llm.0" not in inflight
    assert "agent.react.llm.1" not in inflight
    assert "agent.react.llm.2" in inflight
    assert "agent.react.llm.3" in inflight


# ─────────────────────────────────────────────────────────────────────
# T20 — run_context contract: bind + unbind via async-with (R2)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T20_run_context_lifecycle() -> None:
    with pytest.raises(RuntimeError):
        current_run_context()
    async with run_context(RunContext(run_id="my-run-1")):
        assert current_run_context().run_id == "my-run-1"
    with pytest.raises(RuntimeError):
        current_run_context()


# ─────────────────────────────────────────────────────────────────────
# T21 — Public surface (review §5 P3): __init__ would expose this list
# ─────────────────────────────────────────────────────────────────────


def test_T21_public_surface_locked() -> None:
    """The shipping __all__ list MUST contain the locked symbols."""
    # We can't import the real __init__ (requires beeai-framework
    # installed); instead we read the __all__ in _hook + _options +
    # _errors and assert the unified surface matches the spec.
    surface = set(_hook.__all__) | set(_options.__all__) | set(_errors.__all__)
    for required in (
        "BeeAiStartEvent",
        "ClaimEstimator",
        "CallSignatureFn",
        "RunContext",
        "SpendGuardBeeAIOptions",
        "current_run_context",
        "run_context",
        "subscribe_spendguard",
        "DecisionDenied",
        "SpendGuardConfigError",
        "SpendGuardError",
    ):
        assert required in surface, f"missing public surface symbol: {required}"


# ─────────────────────────────────────────────────────────────────────
# T22 — Inflight slot carries reservation, decision, llm_call, step
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T22_inflight_slot_captures_all_ids() -> None:
    agent = make_agent()
    client = make_client_mock(
        reservation_ids=("res-99",),
        decision_id="dec-99",
    )
    subscribe_spendguard(agent=agent, **make_kwargs(client=client))
    async with run_context(RunContext(run_id="r-22")):
        await agent.emitter.emit(
            "start",
            SimpleNamespace(input=["hi"], modelId="gpt-4o-mini"),
            "agent.react.llm.801.start",
        )
    key = "agent.react.llm.801"
    slot = _SHARED_INFLIGHT.get(key)
    assert slot is not None
    assert slot.reservation_ids == ["res-99"]
    assert slot.decision_id == "dec-99"
    assert slot.run_id == "r-22"
    assert slot.step_id.startswith("r-22:beeai:")


# ─────────────────────────────────────────────────────────────────────
# T23 — Concurrent runs keep independent inflight slots (R1)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T23_concurrent_runs_independent_slots() -> None:
    agent = make_agent()
    subscribe_spendguard(agent=agent, **make_kwargs())

    async def _do(rid: str, path: str) -> None:
        async with run_context(RunContext(run_id=rid)):
            await agent.emitter.emit(
                "start",
                SimpleNamespace(input=[f"hi-{rid}"], modelId="gpt-4o-mini"),
                path,
            )

    await asyncio.gather(
        _do("r-A", "agent.react.llm.A.start"),
        _do("r-B", "agent.react.llm.B.start"),
        _do("r-C", "agent.react.llm.C.start"),
    )
    # Distinct inflight slots per call_key — and slot.run_id carries
    # the right binding for each.
    rids = {
        _SHARED_INFLIGHT.get(k).run_id  # type: ignore[union-attr]
        for k in (
            "agent.react.llm.A",
            "agent.react.llm.B",
            "agent.react.llm.C",
        )
    }
    assert rids == {"r-A", "r-B", "r-C"}


# ─────────────────────────────────────────────────────────────────────
# T24 — _extract_usage_success: dict shape with completion+prompt
# ─────────────────────────────────────────────────────────────────────


def test_T24_extract_usage_dict_completion_prompt() -> None:
    total, pid = _extract_usage_success(
        SimpleNamespace(
            usage={"prompt_tokens": 8, "completion_tokens": 14},
            id="chatcmpl-x",
        )
    )
    assert total == 22
    assert pid == "chatcmpl-x"


def test_T25_extract_usage_object_total() -> None:
    """``usage`` as object with ``total_tokens`` attr."""
    total, pid = _extract_usage_success(
        SimpleNamespace(
            usage=SimpleNamespace(total_tokens=55),
            response_id="resp-x",
        )
    )
    assert total == 55
    assert pid == "resp-x"


def test_T26_extract_usage_nested_under_output() -> None:
    """``usage`` wrapped under ``data.output.usage`` (agent-level event)."""
    total, pid = _extract_usage_success(
        SimpleNamespace(
            output=SimpleNamespace(
                usage={"total_tokens": 99},
            ),
            id="resp-nested",
        )
    )
    assert total == 99
    assert pid == "resp-nested"


def test_T27_extract_usage_none_returns_zero() -> None:
    total, pid = _extract_usage_success(None)
    assert total == 0
    assert pid == ""


# ─────────────────────────────────────────────────────────────────────
# T28 — Custom claim_estimator overrides default (P1)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T28_custom_claim_estimator_overrides_default() -> None:
    """An explicit ``claim_estimator`` MUST short-circuit the default."""
    seen: list[BeeAiStartEvent] = []

    def custom(ev: BeeAiStartEvent) -> list[Any]:
        seen.append(ev)
        return [_claim(42)]

    agent = make_agent()
    client = make_client_mock()
    subscribe_spendguard(
        agent=agent,
        client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        claim_estimator=custom,
    )
    async with run_context(RunContext(run_id="r-28")):
        await agent.emitter.emit(
            "start",
            SimpleNamespace(input=["hi"], modelId="gpt-4o-mini"),
            "agent.react.llm.x.start",
        )
    claims = client.request_decision.call_args.kwargs["projected_claims"]
    assert claims[0].amount_atomic == "42"
    assert len(seen) == 1
    assert seen[0].model_id == "gpt-4o-mini"


# ─────────────────────────────────────────────────────────────────────
# T29 — decision_context tags integration=beeai + backend type
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T29_decision_context_tags_beeai() -> None:
    """Per design.md §5: decision_context tags integration=beeai."""
    agent = make_agent()
    client = make_client_mock()
    subscribe_spendguard(agent=agent, **make_kwargs(client=client))

    class OpenAIChatModel:
        pass

    payload = OpenAIChatModel()
    payload.input = ["hi"]  # type: ignore[attr-defined]
    payload.modelId = "gpt-4o-mini"  # type: ignore[attr-defined]

    async with run_context(RunContext(run_id="r-29")):
        await agent.emitter.emit(
            "start",
            payload,
            "agent.react.llm.zz.start",
        )
    ctx = client.request_decision.call_args.kwargs["decision_context_json"]
    assert ctx["integration"] == "beeai"
    assert ctx["model_backend"] == "OpenAIChatModel"
    assert ctx["model_id"] == "gpt-4o-mini"


# ─────────────────────────────────────────────────────────────────────
# T30 — ApprovalRequired propagates unchanged (S2 sibling)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_T30_approval_required_propagates() -> None:
    approval = ApprovalRequired(
        "review needed",
        decision_id="dec-app",
        approval_request_id="appr-1",
        reason_codes=["APPROVAL_REQUIRED"],
    )
    agent = make_agent()
    client = make_client_mock(request_decision_side_effect=approval)
    subscribe_spendguard(agent=agent, **make_kwargs(client=client))
    with pytest.raises(ApprovalRequired):
        async with run_context(RunContext(run_id="r-30")):
            await agent.emitter.emit(
                "start",
                SimpleNamespace(input=["hi"], modelId="gpt-4o-mini"),
                "agent.react.llm.appr.start",
            )
    # No inflight slot was created (ApprovalRequired raised before put).
    assert len(_SHARED_INFLIGHT) == 0


# ─────────────────────────────────────────────────────────────────────
# T31 — SpendGuardBeeAIOptions validation
# ─────────────────────────────────────────────────────────────────────


def test_T31_options_validates_required_fields() -> None:
    with pytest.raises(SpendGuardConfigError, match="tenant_id"):
        SpendGuardBeeAIOptions(
            tenant_id="",
            budget_id="b1",
            window_instance_id="w1",
        )
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        SpendGuardBeeAIOptions(
            tenant_id="t1",
            budget_id="",
            window_instance_id="w1",
        )
    with pytest.raises(SpendGuardConfigError, match="window_instance_id"):
        SpendGuardBeeAIOptions(
            tenant_id="t1",
            budget_id="b1",
            window_instance_id="",
        )


# ─────────────────────────────────────────────────────────────────────
# T32 — RunContext validates non-empty run_id
# ─────────────────────────────────────────────────────────────────────


def test_T32_run_context_validates_non_empty_run_id() -> None:
    with pytest.raises(SpendGuardConfigError, match="run_id"):
        RunContext(run_id="")
    with pytest.raises(SpendGuardConfigError, match="run_id"):
        RunContext(run_id="   ")


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
    """TP-01 — ``SpendGuardBeeAIOptions(unit_id=...)`` constructs."""
    opts = SpendGuardBeeAIOptions(
        tenant_id="t1",
        budget_id="b1",
        window_instance_id="w1",
        unit_id=_UNIT_ID_FIXTURE,
    )
    assert opts.unit_id == _UNIT_ID_FIXTURE


@pytest.mark.asyncio
async def test_TP02_unit_id_threads_to_wire_claim() -> None:
    """TP-02 — operator binds ``options.unit_id`` to the proto ``UnitRef``;
    the resulting wire ``BudgetClaim.unit.unit_id`` carries it verbatim.
    """
    opts = SpendGuardBeeAIOptions(
        tenant_id="t1",
        budget_id="b1",
        window_instance_id="w1",
        unit_id=_UNIT_ID_FIXTURE,
    )
    agent = make_agent()
    client = make_client_mock()
    subscribe_spendguard(
        agent=agent,
        client=client,
        budget_id=opts.budget_id,
        window_instance_id=opts.window_instance_id,
        unit=common_pb2.UnitRef(unit_id=opts.unit_id or ""),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        claim_estimator=lambda ev: [
            common_pb2.BudgetClaim(
                budget_id="b1",
                unit=common_pb2.UnitRef(unit_id=opts.unit_id or ""),
                amount_atomic="100",
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id="w1",
            )
        ],
    )
    async with run_context(RunContext(run_id="r-tp02")):
        await agent.emitter.emit(
            "start",
            SimpleNamespace(input=["hi"], modelId="gpt-4o-mini"),
            "agent.react.llm.001.start",
        )
    kw = client.request_decision.call_args.kwargs
    assert kw["projected_claims"][0].unit.unit_id == _UNIT_ID_FIXTURE


def test_TP03_options_without_unit_id_constructs() -> None:
    """TP-03 — backward compat: omitting ``unit_id`` keeps default None."""
    opts = SpendGuardBeeAIOptions(
        tenant_id="t1",
        budget_id="b1",
        window_instance_id="w1",
    )
    assert opts.unit_id is None
