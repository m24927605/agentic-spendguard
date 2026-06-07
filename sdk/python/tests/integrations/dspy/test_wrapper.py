# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S101, S106, S107
"""COV_D21 — pytest unit tests for the DSPy SpendGuard callback.

Mocks ``SpendGuardClient`` and uses ``SimpleNamespace`` stubs for the
DSPy ``LMResponse`` / ``BaseCallback`` surface so the suite runs without
``dspy-ai`` installed. Covers U01..U18 from
``docs/specs/coverage/D21_dspy/tests.md`` plus the negative test matrix
from ``acceptance.md`` §2.

Strategy:
  * Direct-imports the ``_wrapper`` module via package path (bypassing
    the install-hint ``ImportError`` guard in ``__init__.py`` so unit
    tests don't require the ``[dspy]`` extra at runtime).
  * Each test cleans ``_PENDING`` + asserts ``_SHIM_IN_FLIGHT.get() ==
    False`` at teardown via the ``dspy_pending_clean`` fixture.
"""

from __future__ import annotations

import asyncio
import importlib
import sys
import types as _stdlib_types
from pathlib import Path
from types import SimpleNamespace
from typing import Any
from unittest.mock import AsyncMock, MagicMock

import pytest

from spendguard._proto.spendguard.common.v1 import common_pb2
from spendguard.errors import DecisionDenied, SpendGuardError

# ─────────────────────────────────────────────────────────────────────
# Load _wrapper bypassing the install-hint ImportError in __init__.
# This way the unit suite runs without dspy-ai installed.
# ─────────────────────────────────────────────────────────────────────

_DSPY_PKG_NAME = "spendguard.integrations.dspy"
if _DSPY_PKG_NAME not in sys.modules:
    _dspy_pkg_path = (
        Path(__file__).resolve().parents[3]
        / "src"
        / "spendguard"
        / "integrations"
        / "dspy"
    )
    ns = _stdlib_types.ModuleType(_DSPY_PKG_NAME)
    ns.__path__ = [str(_dspy_pkg_path)]
    sys.modules[_DSPY_PKG_NAME] = ns

wrapper_mod = importlib.import_module(
    "spendguard.integrations.dspy._wrapper"
)
options_mod = importlib.import_module(
    "spendguard.integrations.dspy._options"
)
errors_mod = importlib.import_module(
    "spendguard.integrations.dspy._errors"
)

SpendGuardDSPyCallback = wrapper_mod.SpendGuardDSPyCallback
_PENDING = wrapper_mod._PENDING
_PENDING_TTL_SECONDS = wrapper_mod._PENDING_TTL_SECONDS
_SHIM_IN_FLIGHT = wrapper_mod._SHIM_IN_FLIGHT
_signature_from_inputs = wrapper_mod._signature_from_inputs
_extract_total_tokens = wrapper_mod._extract_total_tokens
_extract_provider_event_id = wrapper_mod._extract_provider_event_id
_classify_exception = wrapper_mod._classify_exception
BudgetBinding = options_mod.BudgetBinding
RunContext = options_mod.RunContext
SpendGuardDSPyOptions = options_mod.SpendGuardDSPyOptions
SpendGuardDegradeBlocked = errors_mod.SpendGuardDegradeBlocked
SpendGuardConfigError = errors_mod.SpendGuardConfigError
SidecarUnavailable = errors_mod.SidecarUnavailable


# ─────────────────────────────────────────────────────────────────────
# Shared fixtures
# ─────────────────────────────────────────────────────────────────────


@pytest.fixture(autouse=True)
def dspy_pending_clean():
    """Test-isolation fixture.

    Resets ``_PENDING`` and ``_SHIM_IN_FLIGHT`` BEFORE and AFTER each
    test so cross-test bleed-through never produces a stuck contextvar
    or orphan stash entry (review-standards §3 "Test isolation").
    """
    _PENDING.clear()
    token = _SHIM_IN_FLIGHT.set(False)
    try:
        yield
    finally:
        _PENDING.clear()
        try:
            _SHIM_IN_FLIGHT.reset(token)
        except (ValueError, LookupError):
            # If a test set/reset the contextvar via a different
            # token, just drop a fresh False so the next test starts
            # clean.
            _SHIM_IN_FLIGHT.set(False)
        assert _PENDING == {}


# ─────────────────────────────────────────────────────────────────────
# Shape stubs (mirror DSPy's surface)
# ─────────────────────────────────────────────────────────────────────


def make_lm_instance(*, model: str = "openai/gpt-4o-mini"):
    """SimpleNamespace shaped like ``dspy.LM`` (only ``.model`` is read)."""
    return SimpleNamespace(model=model)


def make_lm_response(
    *,
    total_tokens: int | None = None,
    prompt_tokens: int | None = None,
    completion_tokens: int | None = None,
    input_tokens: int | None = None,
    output_tokens: int | None = None,
    response_id: str = "resp-1",
):
    """SimpleNamespace shaped like a DSPy ``LMResponse``."""
    usage: dict[str, Any] = {}
    if total_tokens is not None:
        usage["total_tokens"] = total_tokens
    if prompt_tokens is not None:
        usage["prompt_tokens"] = prompt_tokens
    if completion_tokens is not None:
        usage["completion_tokens"] = completion_tokens
    if input_tokens is not None:
        usage["input_tokens"] = input_tokens
    if output_tokens is not None:
        usage["output_tokens"] = output_tokens
    return SimpleNamespace(usage=usage, id=response_id)


def make_client_mock(
    *,
    tenant_id: str = "tenant-1",
    session_id: str = "session-1",
    decision_id: str = "dec-1",
    reservation_ids: tuple[str, ...] = ("res-1",),
    decision: str = "CONTINUE",
    request_decision_side_effect: Any = None,
    emit_post_side_effect: Any = None,
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
    if emit_post_side_effect is not None:
        client.emit_llm_call_post = AsyncMock(
            side_effect=emit_post_side_effect
        )
    else:
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


def _binding():
    return BudgetBinding(
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
    )


def make_callback(
    *,
    client: MagicMock | None = None,
    budget_resolver=None,
    claim_estimator: Any = None,
    claim_reconciler: Any = None,
    run_context_factory=None,
    fail_closed: bool = True,
):
    """Build a ``SpendGuardDSPyCallback`` with sane test defaults."""
    if client is None:
        client = make_client_mock()
    if budget_resolver is None:
        budget_resolver = lambda model: _binding()  # noqa: E731
    if claim_reconciler is None:
        claim_reconciler = lambda outputs: [_claim(42)]  # noqa: E731
    return SpendGuardDSPyCallback(
        client=client,
        budget_resolver=budget_resolver,
        claim_estimator=claim_estimator,
        claim_reconciler=claim_reconciler,
        run_context_factory=run_context_factory,
        fail_closed=fail_closed,
    )


# ─────────────────────────────────────────────────────────────────────
# U01 — Import-hint ImportError when dspy missing
# ─────────────────────────────────────────────────────────────────────


def test_U01_import_error_message_when_dspy_missing() -> None:
    """Package barrel ``__init__.py`` carries the install-hint string."""
    barrel_path = (
        Path(__file__).resolve().parents[3]
        / "src"
        / "spendguard"
        / "integrations"
        / "dspy"
        / "__init__.py"
    )
    assert barrel_path.exists()
    source = barrel_path.read_text(encoding="utf-8")
    assert "pip install 'spendguard-sdk[dspy]'" in source
    assert "from dspy.utils.callback" in source
    assert "except ImportError" in source
    assert "raise ImportError" in source


# ─────────────────────────────────────────────────────────────────────
# U02 — RunContext default factory mints UUIDv7 per call
# ─────────────────────────────────────────────────────────────────────


def test_U02_run_context_default_factory_emits_uuid7() -> None:
    """When ``run_context_factory`` is None, factory mints fresh UUIDv7."""
    cb = make_callback()
    rc1 = cb._run_context_factory()
    rc2 = cb._run_context_factory()
    assert isinstance(rc1, RunContext)
    assert isinstance(rc2, RunContext)
    assert rc1.run_id != rc2.run_id, "consecutive RunContexts must differ"


# ─────────────────────────────────────────────────────────────────────
# U03 — TTL sweep drops stale entries
# ─────────────────────────────────────────────────────────────────────


def test_U03_pending_registry_ttl_sweep_drops_old_entries(caplog) -> None:
    """Stale entries older than TTL are swept on next on_lm_start."""
    cb = make_callback()
    inst = make_lm_instance()

    # Inject a stale entry artificially.
    import time

    _PENDING["stale-call-id"] = options_mod._CallState(
        decision_id="d-old",
        reservation_id="r-old",
        llm_call_id="lcid-old",
        step_id="dspy:old",
        run_id="run-old",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        inputs_signature="sig-old",
        estimator_amount_atomic="50",
        started_at=time.monotonic() - (_PENDING_TTL_SECONDS + 60),
    )
    assert "stale-call-id" in _PENDING

    with caplog.at_level("WARNING", logger="spendguard.integrations.dspy"):
        cb.on_lm_start("fresh-call-1", inst, {"prompt": "hi"})

    assert "stale-call-id" not in _PENDING
    assert any(
        "TTL-sweeping" in r.getMessage() for r in caplog.records
    )
    # The fresh call SHOULD now be stashed.
    assert "fresh-call-1" in _PENDING
    # Clean up the fresh entry.
    cb.on_lm_end("fresh-call-1", make_lm_response(total_tokens=10), None)


# ─────────────────────────────────────────────────────────────────────
# U04 — Shared contextvar object identity with D12 (G13)
# ─────────────────────────────────────────────────────────────────────


def test_U04_shared_contextvar_is_same_object_as_d12() -> None:
    """``spendguard._litellm_shim._IN_FLIGHT`` IS the same object the
    dspy wrapper reads (review-standards §1.4, G13 gate).
    """
    from spendguard._litellm_shim import _IN_FLIGHT as canonical

    assert canonical is _SHIM_IN_FLIGHT


# ─────────────────────────────────────────────────────────────────────
# U05 — on_lm_start calls request_decision (happy path)
# ─────────────────────────────────────────────────────────────────────


def test_U05_on_lm_start_calls_request_decision() -> None:
    """Fake sidecar records exactly one RequestDecision with the
    expected ``trigger`` + ``route``."""
    client = make_client_mock()
    cb = make_callback(client=client)
    inst = make_lm_instance()
    inputs = {"messages": [{"role": "user", "content": "hi"}]}

    cb.on_lm_start("call-u5", inst, inputs)

    client.request_decision.assert_awaited_once()
    kwargs = client.request_decision.call_args.kwargs
    assert kwargs["trigger"] == "LLM_CALL_PRE"
    assert kwargs["route"] == "llm.call"
    assert len(kwargs["projected_claims"]) == 1
    ctx = kwargs["decision_context_json"]
    assert ctx["integration"] == "dspy"
    assert ctx["lm_model"] == "openai/gpt-4o-mini"
    # Inputs dict identity untouched (U-inputs immutability).
    assert inputs == {"messages": [{"role": "user", "content": "hi"}]}

    # Clean up.
    cb.on_lm_end("call-u5", make_lm_response(total_tokens=10), None)


# ─────────────────────────────────────────────────────────────────────
# U06 — on_lm_start records pending state keyed by call_id
# ─────────────────────────────────────────────────────────────────────


def test_U06_on_lm_start_records_pending_state() -> None:
    """``_PENDING[call_id]`` carries a ``_CallState`` with the
    reservation_id from the outcome."""
    client = make_client_mock(reservation_ids=("res-u6",))
    cb = make_callback(client=client)
    inst = make_lm_instance()

    cb.on_lm_start("call-u6", inst, {"prompt": "hello"})

    assert "call-u6" in _PENDING
    state = _PENDING["call-u6"]
    assert state.reservation_id == "res-u6"
    assert state.run_id, "run_id must be set"
    assert state.step_id.startswith("dspy:")
    assert state.shim_token is not None

    # Clean up.
    cb.on_lm_end("call-u6", make_lm_response(total_tokens=10), None)


# ─────────────────────────────────────────────────────────────────────
# U07 — on_lm_start sets _SHIM_IN_FLIGHT contextvar (D12 coexistence)
# ─────────────────────────────────────────────────────────────────────


def test_U07_on_lm_start_sets_in_flight_contextvar() -> None:
    """Inside on_lm_start, _SHIM_IN_FLIGHT.get() == True; on_lm_end
    resets via the captured token."""
    client = make_client_mock()
    cb = make_callback(client=client)
    inst = make_lm_instance()

    assert _SHIM_IN_FLIGHT.get() is False
    cb.on_lm_start("call-u7", inst, {"prompt": "hi"})
    # Between start and end the contextvar must be True (blocks D12).
    assert _SHIM_IN_FLIGHT.get() is True
    cb.on_lm_end("call-u7", make_lm_response(total_tokens=10), None)
    assert _SHIM_IN_FLIGHT.get() is False


# ─────────────────────────────────────────────────────────────────────
# U08 — Constructor doc string mentions "MUST appear FIRST"
# ─────────────────────────────────────────────────────────────────────


def test_U08_callback_first_in_dspy_callbacks_list_documented() -> None:
    """Class docstring references "MUST appear FIRST" — guards against
    accidental doc drift (operator-facing ordering contract)."""
    doc = SpendGuardDSPyCallback.__doc__ or ""
    assert "MUST appear FIRST" in doc, (
        "callback class docstring must document the FIRST-in-list "
        "ordering contract (operator responsibility per design.md §8 #1)"
    )


# ─────────────────────────────────────────────────────────────────────
# U09 — Reserve fires BEFORE LM provider call (load-bearing — INV-2)
# ─────────────────────────────────────────────────────────────────────


def test_U09_reserve_fires_before_lm_provider_call() -> None:
    """Test instrumentation records a list of events: reserve event must
    be recorded BEFORE the simulated provider HTTP. Order MUST equal
    ``["reserve", "provider"]``. Failing means D21 thesis is broken."""
    events: list[str] = []
    client = make_client_mock()

    async def record_reserve(**kwargs):
        events.append("reserve")
        return SimpleNamespace(
            decision_id="d-u9",
            reservation_ids=("r-u9",),
            audit_decision_event_id="audit-u9",
            decision="CONTINUE",
        )

    client.request_decision = AsyncMock(side_effect=record_reserve)
    cb = make_callback(client=client)

    cb.on_lm_start("call-u9", make_lm_instance(), {"prompt": "hi"})
    events.append("provider")  # simulate the LM dispatching HTTP

    assert events == ["reserve", "provider"], (
        "INV-2 violated: reserve must precede provider HTTP"
    )
    cb.on_lm_end("call-u9", make_lm_response(total_tokens=10), None)


# ─────────────────────────────────────────────────────────────────────
# U10 — DENY blocks provider call (load-bearing — INV-1)
# ─────────────────────────────────────────────────────────────────────


def test_U10_deny_blocks_provider_call() -> None:
    """Sidecar returns DENY → ``DecisionDenied`` raised → provider not
    called AND _PENDING empty AND _SHIM_IN_FLIGHT False."""
    provider_hits = {"count": 0}
    client = make_client_mock(
        request_decision_side_effect=DecisionDenied(
            "deny", decision_id="d-deny", reason_codes=["budget_exhausted"],
        ),
    )
    cb = make_callback(client=client)

    with pytest.raises(DecisionDenied):
        cb.on_lm_start("call-u10", make_lm_instance(), {"prompt": "hi"})
        provider_hits["count"] += 1  # SHOULD never execute

    assert provider_hits["count"] == 0, "INV-1: provider must not be called"
    assert _PENDING == {}, "no stash on DENY"
    assert _SHIM_IN_FLIGHT.get() is False, "contextvar reset on DENY"


# ─────────────────────────────────────────────────────────────────────
# U11 — DEGRADE fails closed by default
# ─────────────────────────────────────────────────────────────────────


def test_U11_degrade_fails_closed() -> None:
    """Sidecar returns DEGRADE → ``SpendGuardDegradeBlocked`` raised."""
    client = make_client_mock(decision="DEGRADE")
    cb = make_callback(client=client)

    with pytest.raises(SpendGuardDegradeBlocked):
        cb.on_lm_start("call-u11", make_lm_instance(), {"prompt": "hi"})

    assert _PENDING == {}
    assert _SHIM_IN_FLIGHT.get() is False


# ─────────────────────────────────────────────────────────────────────
# U12 — Sync-in-async raises SyncInAsyncContext
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U12_async_context_raises() -> None:
    """Calling on_lm_start from inside a running loop → SyncInAsyncContext."""
    client = make_client_mock()
    cb = make_callback(client=client)
    with pytest.raises(SpendGuardDSPyCallback.SyncInAsyncContext) as exc_info:
        cb.on_lm_start("call-u12", make_lm_instance(), {"prompt": "hi"})
    # Hint must name the function + point at sync entrypoint (Major).
    msg = str(exc_info.value)
    assert "on_lm_start" in msg
    assert "sync entrypoint" in msg
    # Stash + contextvar still clean.
    assert _PENDING == {}
    assert _SHIM_IN_FLIGHT.get() is False


# ─────────────────────────────────────────────────────────────────────
# U13 — on_lm_end commits with real usage
# ─────────────────────────────────────────────────────────────────────


def test_U13_on_lm_end_commits_with_real_usage() -> None:
    """outputs[0].usage carries total_tokens=42; commit row should
    record estimated_amount_atomic="42" + SUCCESS outcome."""
    client = make_client_mock()
    cb = make_callback(client=client)
    cb.on_lm_start("call-u13", make_lm_instance(), {"prompt": "hi"})

    outputs = [make_lm_response(total_tokens=42)]
    cb.on_lm_end("call-u13", outputs, None)

    client.emit_llm_call_post.assert_awaited_once()
    kwargs = client.emit_llm_call_post.call_args.kwargs
    assert kwargs["outcome"] == "SUCCESS"
    assert kwargs["estimated_amount_atomic"] == "42"
    assert _PENDING == {}
    assert _SHIM_IN_FLIGHT.get() is False


# ─────────────────────────────────────────────────────────────────────
# U14 — on_lm_end failure outcome propagates
# ─────────────────────────────────────────────────────────────────────


def test_U14_on_lm_end_failure_outcome_propagates() -> None:
    """outputs=None + exception=ConnectionError → outcome=FAILURE +
    _PENDING cleared + contextvar reset."""
    client = make_client_mock()
    cb = make_callback(client=client)
    cb.on_lm_start("call-u14", make_lm_instance(), {"prompt": "hi"})

    cb.on_lm_end("call-u14", None, ConnectionError("boom"))

    kwargs = client.emit_llm_call_post.call_args.kwargs
    assert kwargs["outcome"] == "FAILURE"
    assert _PENDING == {}
    assert _SHIM_IN_FLIGHT.get() is False


# ─────────────────────────────────────────────────────────────────────
# U15 — on_lm_end with CancelledError → CANCELLED outcome
# ─────────────────────────────────────────────────────────────────────


def test_U15_on_lm_end_cancellation_outcome() -> None:
    """exception=asyncio.CancelledError → outcome=CANCELLED."""
    client = make_client_mock()
    cb = make_callback(client=client)
    cb.on_lm_start("call-u15", make_lm_instance(), {"prompt": "hi"})

    cb.on_lm_end("call-u15", None, asyncio.CancelledError())

    kwargs = client.emit_llm_call_post.call_args.kwargs
    assert kwargs["outcome"] == "CANCELLED"
    assert _PENDING == {}


# ─────────────────────────────────────────────────────────────────────
# U16 — on_lm_end without matching start logs + returns
# ─────────────────────────────────────────────────────────────────────


def test_U16_on_lm_end_without_start_logs_and_returns(caplog) -> None:
    """unknown call_id → WARN log + no commit fires + no exception."""
    client = make_client_mock()
    cb = make_callback(client=client)

    with caplog.at_level("WARNING", logger="spendguard.integrations.dspy"):
        cb.on_lm_end("never-started-call", None, None)

    client.emit_llm_call_post.assert_not_awaited()
    assert any("on_lm_end" in r.getMessage() for r in caplog.records)


# ─────────────────────────────────────────────────────────────────────
# U17 — Custom LM subclass missing .usage → estimator fallback
# ─────────────────────────────────────────────────────────────────────


def test_U17_custom_lm_subclass_no_usage_falls_back() -> None:
    """Bare string outputs (no .usage) → _extract_total_tokens returns 0
    → commit fires with estimated_amount_atomic="<estimator snapshot>".
    NEVER raises."""
    client = make_client_mock()
    cb = make_callback(
        client=client,
        # Reconciler returns no claims so the fallback kicks in.
        claim_reconciler=lambda outputs: [],
    )
    cb.on_lm_start("call-u17", make_lm_instance(), {"prompt": "x" * 200})

    # Bare-string outputs (some custom dspy.LM subclasses do this).
    cb.on_lm_end("call-u17", ["just a string", "another"], None)

    kwargs = client.emit_llm_call_post.call_args.kwargs
    assert kwargs["outcome"] == "SUCCESS"
    # Falls back to estimator snapshot (chars/4 from inputs) — non-zero.
    assert int(kwargs["estimated_amount_atomic"]) > 0


# ─────────────────────────────────────────────────────────────────────
# U18 — _SHIM_IN_FLIGHT reset is durable across start/end pair
# ─────────────────────────────────────────────────────────────────────


def test_U18_in_flight_reset_after_on_lm_end() -> None:
    """After multiple consecutive start/end pairs, _SHIM_IN_FLIGHT
    remains False — token reset durable."""
    client = make_client_mock()
    cb = make_callback(client=client)

    for i in range(3):
        cb.on_lm_start(f"call-u18-{i}", make_lm_instance(), {"prompt": "hi"})
        assert _SHIM_IN_FLIGHT.get() is True
        cb.on_lm_end(f"call-u18-{i}", make_lm_response(total_tokens=5), None)
        assert _SHIM_IN_FLIGHT.get() is False


# ─────────────────────────────────────────────────────────────────────
# Helpers: extraction tolerance + classifier + signature
# ─────────────────────────────────────────────────────────────────────


def test_helper_extract_total_tokens_handles_none_list_dict() -> None:
    """``_extract_total_tokens`` never raises and returns 0 for empty /
    None / malformed shapes."""
    assert _extract_total_tokens(None) == 0
    assert _extract_total_tokens([]) == 0
    assert _extract_total_tokens([SimpleNamespace()]) == 0
    assert _extract_total_tokens([SimpleNamespace(usage="not a dict")]) == 0
    # Real OpenAI shape
    assert (
        _extract_total_tokens([SimpleNamespace(usage={"total_tokens": 11})])
        == 11
    )
    # Anthropic shape
    assert (
        _extract_total_tokens([SimpleNamespace(
            usage={"input_tokens": 5, "output_tokens": 7}
        )])
        == 12
    )


def test_helper_extract_provider_event_id_handles_none_and_missing() -> None:
    """``_extract_provider_event_id`` returns ""  for None / missing."""
    assert _extract_provider_event_id(None) == ""
    assert _extract_provider_event_id([]) == ""
    assert _extract_provider_event_id([SimpleNamespace()]) == ""
    assert (
        _extract_provider_event_id([SimpleNamespace(id="abc")])
        == "abc"
    )


def test_helper_signature_is_deterministic() -> None:
    """Same inputs → same signature; different inputs → different."""
    s1 = _signature_from_inputs({"messages": [{"role": "user", "content": "hi"}]})
    s2 = _signature_from_inputs({"messages": [{"role": "user", "content": "hi"}]})
    s3 = _signature_from_inputs({"messages": [{"role": "user", "content": "bye"}]})
    assert s1 == s2
    assert s1 != s3


def test_helper_signature_handles_unserializable() -> None:
    """Non-JSON-serializable inputs still produce a stable hash via repr."""

    class Weird:
        def __repr__(self):
            return "WeirdInstance"

    s = _signature_from_inputs({"obj": Weird()})
    assert isinstance(s, str) and len(s) == 32


def test_classify_exception_order_matters() -> None:
    """CancelledError check MUST precede generic Exception check."""
    assert _classify_exception(None) == "SUCCESS"
    assert _classify_exception(asyncio.CancelledError()) == "CANCELLED"
    assert _classify_exception(ConnectionError("x")) == "FAILURE"
    # Subclass of CancelledError should still classify as CANCELLED.

    class MyCancel(asyncio.CancelledError):
        pass

    assert _classify_exception(MyCancel()) == "CANCELLED"


# ─────────────────────────────────────────────────────────────────────
# Constructor validation surface
# ─────────────────────────────────────────────────────────────────────


def test_construct_rejects_none_client() -> None:
    with pytest.raises(SpendGuardConfigError, match="client"):
        SpendGuardDSPyCallback(
            client=None,
            budget_resolver=lambda m: _binding(),
            claim_reconciler=lambda o: [_claim()],
        )


def test_construct_rejects_none_budget_resolver() -> None:
    with pytest.raises(SpendGuardConfigError, match="budget_resolver"):
        SpendGuardDSPyCallback(
            client=make_client_mock(),
            budget_resolver=None,
            claim_reconciler=lambda o: [_claim()],
        )


def test_construct_rejects_none_reconciler() -> None:
    with pytest.raises(SpendGuardConfigError, match="claim_reconciler"):
        SpendGuardDSPyCallback(
            client=make_client_mock(),
            budget_resolver=lambda m: _binding(),
            claim_reconciler=None,
        )


def test_options_validates_required_fields() -> None:
    with pytest.raises(SpendGuardConfigError, match="tenant_id"):
        SpendGuardDSPyOptions(tenant_id="")
    with pytest.raises(SpendGuardConfigError, match="sidecar_socket_path"):
        SpendGuardDSPyOptions(
            tenant_id="t", sidecar_socket_path=""
        )


# ─────────────────────────────────────────────────────────────────────
# Fail-open env flag honored
# ─────────────────────────────────────────────────────────────────────


def test_fail_open_env_flag_allows_degrade(monkeypatch) -> None:
    """SPENDGUARD_DSPY_FAIL_OPEN=1 + DEGRADE → no raise, no commit."""
    monkeypatch.setenv("SPENDGUARD_DSPY_FAIL_OPEN", "1")
    client = make_client_mock(decision="DEGRADE")
    cb = make_callback(client=client, fail_closed=True)

    cb.on_lm_start("call-foe", make_lm_instance(), {"prompt": "hi"})
    # No raise. on_lm_end should be a no-op (reservation_id None).
    cb.on_lm_end("call-foe", make_lm_response(total_tokens=5), None)
    # No commit fires under fail-open DEGRADE (no reservation).
    client.emit_llm_call_post.assert_not_awaited()
    assert _SHIM_IN_FLIGHT.get() is False


# ─────────────────────────────────────────────────────────────────────
# Sidecar error during pre-call is fail-closed
# ─────────────────────────────────────────────────────────────────────


def test_sidecar_error_fail_closed_raises_sidecar_unavailable() -> None:
    """Non-DENY SpendGuardError during reserve → SidecarUnavailable;
    contextvar still reset."""
    client = make_client_mock(
        request_decision_side_effect=SpendGuardError("transport gone"),
    )
    cb = make_callback(client=client)

    with pytest.raises(SidecarUnavailable):
        cb.on_lm_start("call-sue", make_lm_instance(), {"prompt": "hi"})

    assert _PENDING == {}
    assert _SHIM_IN_FLIGHT.get() is False


# ─────────────────────────────────────────────────────────────────────
# Run-context bridging
# ─────────────────────────────────────────────────────────────────────


def test_custom_run_context_factory_threads_run_id() -> None:
    """When ``run_context_factory`` is supplied, the PRE call uses its
    ``run_id`` (cross-framework bridging)."""
    client = make_client_mock()
    cb = make_callback(
        client=client,
        run_context_factory=lambda: RunContext(run_id="bridged-run-id"),
    )
    cb.on_lm_start("call-rcf", make_lm_instance(), {"prompt": "hi"})
    pre_kwargs = client.request_decision.call_args.kwargs
    assert pre_kwargs["run_id"] == "bridged-run-id"
    cb.on_lm_end("call-rcf", make_lm_response(total_tokens=5), None)


# ─────────────────────────────────────────────────────────────────────
# emit_llm_call_post failure under FAILURE outcome is swallowed
# ─────────────────────────────────────────────────────────────────────


def test_emit_post_swallows_errors_on_failure_path() -> None:
    """emit_llm_call_post raising SpendGuardError on a FAILURE path must
    NOT mask the original exception path (logs + swallows)."""
    client = make_client_mock(
        emit_post_side_effect=SpendGuardError("post failed"),
    )
    cb = make_callback(client=client)
    cb.on_lm_start("call-eps", make_lm_instance(), {"prompt": "hi"})

    # Should not raise even though emit_llm_call_post errors.
    cb.on_lm_end("call-eps", None, ConnectionError("upstream gone"))

    assert _PENDING == {}
    assert _SHIM_IN_FLIGHT.get() is False


# ─────────────────────────────────────────────────────────────────────
# Empty call_id is rejected
# ─────────────────────────────────────────────────────────────────────


def test_empty_call_id_rejected() -> None:
    """DSPy 2.6+ guarantees call_id; empty string → SpendGuardConfigError."""
    client = make_client_mock()
    cb = make_callback(client=client)
    with pytest.raises(SpendGuardConfigError, match="call_id"):
        cb.on_lm_start("", make_lm_instance(), {"prompt": "hi"})


# ─────────────────────────────────────────────────────────────────────
# budget_resolver returning None is rejected
# ─────────────────────────────────────────────────────────────────────


def test_resolver_returning_none_rejected() -> None:
    """budget_resolver -> None is a contract violation."""
    client = make_client_mock()
    cb = make_callback(
        client=client,
        budget_resolver=lambda model: None,
    )
    with pytest.raises(SpendGuardConfigError, match="budget_resolver"):
        cb.on_lm_start("call-bnr", make_lm_instance(), {"prompt": "hi"})
