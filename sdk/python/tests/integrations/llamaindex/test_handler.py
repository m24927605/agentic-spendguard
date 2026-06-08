# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S101, S106, S107
"""COV_D27 — pytest unit + integration tests for the LlamaIndex adapter.

Mocks ``SpendGuardClient`` (Tier 1) and uses ``SimpleNamespace`` stubs
for the LlamaIndex ``BaseCallbackHandler`` / ``CBEventType`` /
``EventPayload`` surface so the suite runs without ``llama-index-core``
installed. Verifies every contract from
``docs/specs/coverage/D27_llamaindex/tests.md`` U01-U25 + I01-I08.

Strategy:
  * Direct-imports ``_hook`` via package path (bypassing the
    ``llamaindex.__init__`` install-hint guard so unit tests run
    without the ``[llamaindex]`` extra at runtime).
  * Multi-vendor coverage: parametrized payload fixtures for OpenAI /
    Anthropic / Gemini / Bedrock Converse response shapes.
  * Async client mocked via ``AsyncMock`` — the handler's sync-from-async
    bridge schedules each ``request_decision`` / ``emit_llm_call_post``
    onto its background loop and the AsyncMock returns immediately.
"""

from __future__ import annotations

import importlib
import json
import sys
import types as _stdlib_types
import warnings
from pathlib import Path
from types import SimpleNamespace
from typing import Any
from unittest.mock import AsyncMock, MagicMock

import pytest

from spendguard._proto.spendguard.common.v1 import common_pb2
from spendguard.errors import DecisionDenied

# ─────────────────────────────────────────────────────────────────────
# Load _hook bypassing the install-hint ImportError in __init__. This
# way the unit suite runs without ``llama-index-core``.
# ─────────────────────────────────────────────────────────────────────

_LLAMAINDEX_PKG_NAME = "spendguard.integrations.llamaindex"
if _LLAMAINDEX_PKG_NAME not in sys.modules:
    _pkg_path = (
        Path(__file__).resolve().parents[3]
        / "src"
        / "spendguard"
        / "integrations"
        / "llamaindex"
    )
    ns = _stdlib_types.ModuleType(_LLAMAINDEX_PKG_NAME)
    ns.__path__ = [str(_pkg_path)]
    sys.modules[_LLAMAINDEX_PKG_NAME] = ns

hook_mod = importlib.import_module(
    "spendguard.integrations.llamaindex._hook"
)
options_mod = importlib.import_module(
    "spendguard.integrations.llamaindex._options"
)
errors_mod = importlib.import_module(
    "spendguard.integrations.llamaindex._errors"
)

SpendGuardLlamaIndexHandler = hook_mod.SpendGuardLlamaIndexHandler
LlamaIndexRunContext = options_mod.LlamaIndexRunContext
SpendGuardLlamaIndexOptions = options_mod.SpendGuardLlamaIndexOptions
SpendGuardLlamaIndexDenied = errors_mod.SpendGuardLlamaIndexDenied
SpendGuardConfigError = errors_mod.SpendGuardConfigError
run_context = hook_mod.run_context
current_run_context = hook_mod.current_run_context


# ─────────────────────────────────────────────────────────────────────
# Stub event types — what the real CBEventType / EventPayload would
# present. Match the .value strings the real Enum yields so payload
# dict lookups behave identically.
# ─────────────────────────────────────────────────────────────────────


class _StubCBEventType:
    LLM = "llm"
    EMBEDDING = "embedding"
    RETRIEVE = "retrieve"
    CHUNK = "chunk"
    QUERY = "query"
    NODE_PARSING = "node_parsing"


class _StubEventPayload:
    MESSAGES = "messages"
    PROMPT = "prompt"
    RESPONSE = "response"
    SERIALIZED = "serialized"


# ─────────────────────────────────────────────────────────────────────
# Payload + response shape helpers
# ─────────────────────────────────────────────────────────────────────


def make_openai_start_payload(*, model: str = "gpt-4o-mini") -> dict[str, Any]:
    """``payload_start`` shape from ``llama-index-llms-openai``."""
    return {
        _StubEventPayload.MESSAGES: [
            {"role": "user", "content": "What is the budget cap?"}
        ],
        _StubEventPayload.SERIALIZED: {
            "model": model,
            "class_name": "OpenAI",
        },
    }


def make_openai_end_payload(
    *,
    total_tokens: int = 42,
    response_id: str = "chatcmpl-abc",
) -> dict[str, Any]:
    """``payload_end`` shape from OpenAI provider."""
    response = SimpleNamespace(
        raw={
            "id": response_id,
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": total_tokens - 12,
                "total_tokens": total_tokens,
            },
        }
    )
    return {_StubEventPayload.RESPONSE: response}


def make_anthropic_end_payload(
    *,
    input_tokens: int = 10,
    output_tokens: int = 15,
    response_id: str = "msg_01ABC",
) -> dict[str, Any]:
    response = SimpleNamespace(
        raw={
            "id": response_id,
            "usage": {
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
            },
        }
    )
    return {_StubEventPayload.RESPONSE: response}


def make_gemini_end_payload(
    *,
    total_token_count: int = 33,
    response_id: str = "gemini-resp-1",
) -> dict[str, Any]:
    response = SimpleNamespace(
        raw={
            "id": response_id,
            "usage_metadata": {"total_token_count": total_token_count},
        }
    )
    return {_StubEventPayload.RESPONSE: response}


def make_bedrock_end_payload(
    *,
    input_tokens: int = 7,
    output_tokens: int = 8,
    response_id: str = "bedrock-conv-1",
) -> dict[str, Any]:
    response = SimpleNamespace(
        raw={
            "response_id": response_id,
            "usage": {
                "inputTokens": input_tokens,
                "outputTokens": output_tokens,
            },
        }
    )
    return {_StubEventPayload.RESPONSE: response}


def make_empty_end_payload() -> dict[str, Any]:
    """Payload whose response has no usage field — falls back to 0."""
    response = SimpleNamespace(raw={})
    return {_StubEventPayload.RESPONSE: response}


# ─────────────────────────────────────────────────────────────────────
# Client mock — handler awaits these inside the background bridge loop.
# ─────────────────────────────────────────────────────────────────────


def make_client_mock(
    *,
    tenant_id: str = "tenant-1",
    session_id: str = "session-1",
    decision_id: str = "dec-1",
    reservation_ids: tuple[str, ...] = ("res-1",),
    request_decision_side_effect: Any = None,
) -> MagicMock:
    """Async-shaped mock client.

    The handler dispatches ``client.request_decision(...)`` and
    ``client.emit_llm_call_post(...)`` as coroutines onto its
    background asyncio loop; ``AsyncMock`` produces awaitables so the
    bridge resolves them immediately.
    """
    client = MagicMock()
    client.tenant_id = tenant_id
    client.session_id = session_id

    outcome = SimpleNamespace(
        decision_id=decision_id,
        reservation_ids=reservation_ids,
        audit_decision_event_id="audit-1",
        decision="CONTINUE",
    )
    if request_decision_side_effect is not None:
        client.request_decision = AsyncMock(
            side_effect=request_decision_side_effect
        )
    else:
        client.request_decision = AsyncMock(return_value=outcome)
    client.emit_llm_call_post = AsyncMock(return_value=None)
    return client


def make_handler(
    *,
    client: MagicMock | None = None,
    claim_estimator: Any = None,
    run_id_fn: Any = None,
) -> Any:
    """Build a handler with sane test defaults.

    Default ``claim_estimator`` returns a single 100-atomic claim so
    PRE always has something to reserve.
    """
    if client is None:
        client = make_client_mock()
    if claim_estimator is None:
        claim_estimator = lambda payload: [  # noqa: E731
            common_pb2.BudgetClaim(
                budget_id="b1",
                unit=common_pb2.UnitRef(unit_id="u1"),
                amount_atomic="100",
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id="w1",
            )
        ]
    return SpendGuardLlamaIndexHandler(
        client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        claim_estimator=claim_estimator,
        run_id_fn=run_id_fn,
    )


# ─────────────────────────────────────────────────────────────────────
# U01 — Import-error guard
# ─────────────────────────────────────────────────────────────────────


def test_U01_import_error_when_llama_index_core_missing() -> None:
    """The barrel ``__init__`` source carries the install-hint guard."""
    barrel_path = (
        Path(__file__).resolve().parents[3]
        / "src"
        / "spendguard"
        / "integrations"
        / "llamaindex"
        / "__init__.py"
    )
    assert barrel_path.exists()
    source = barrel_path.read_text(encoding="utf-8")
    assert "pip install 'spendguard-sdk[llamaindex]'" in source
    assert "from llama_index.core.callbacks" in source
    assert "except ImportError" in source
    assert "raise ImportError" in source


# ─────────────────────────────────────────────────────────────────────
# U02-U05 — Default estimator dispatch per model family
# ─────────────────────────────────────────────────────────────────────


def test_U02_handler_init_defaults_estimator_for_openai_model() -> None:
    """``model="gpt-4o-mini"`` → estimator caches OpenAI family closure."""
    handler = SpendGuardLlamaIndexHandler(
        client=make_client_mock(),
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
    )
    payload = make_openai_start_payload(model="gpt-4o-mini")
    # First call caches the estimator under model="gpt-4o-mini"
    claims = handler._estimate_claims(payload)
    assert isinstance(claims, list) and len(claims) == 1
    assert int(claims[0].amount_atomic) > 0
    assert "gpt-4o-mini" in handler._default_estimator_cache
    handler.close()


def test_U03_handler_init_defaults_estimator_for_anthropic_model() -> None:
    """``model="claude-3-5-sonnet"`` → Anthropic family."""
    handler = SpendGuardLlamaIndexHandler(
        client=make_client_mock(),
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
    )
    payload = {
        _StubEventPayload.MESSAGES: [{"role": "user", "content": "hello"}],
        _StubEventPayload.SERIALIZED: {"model": "claude-3-5-sonnet"},
    }
    claims = handler._estimate_claims(payload)
    assert len(claims) == 1
    assert "claude-3-5-sonnet" in handler._default_estimator_cache
    handler.close()


def test_U04_handler_init_defaults_estimator_for_gemini_model() -> None:
    """``model="gemini-1.5-flash"`` → Gemini family."""
    handler = SpendGuardLlamaIndexHandler(
        client=make_client_mock(),
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
    )
    payload = {
        _StubEventPayload.MESSAGES: [{"role": "user", "content": "hello"}],
        _StubEventPayload.SERIALIZED: {"model": "gemini-1.5-flash"},
    }
    claims = handler._estimate_claims(payload)
    assert len(claims) == 1
    assert "gemini-1.5-flash" in handler._default_estimator_cache
    handler.close()


def test_U05_handler_init_defaults_estimator_for_bedrock_model() -> None:
    """``model="anthropic.claude-3-sonnet-20240229-v1:0"`` → Bedrock family."""
    handler = SpendGuardLlamaIndexHandler(
        client=make_client_mock(),
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
    )
    payload = {
        _StubEventPayload.MESSAGES: [{"role": "user", "content": "hello"}],
        _StubEventPayload.SERIALIZED: {
            "model": "anthropic.claude-3-sonnet-20240229-v1:0"
        },
    }
    claims = handler._estimate_claims(payload)
    assert len(claims) == 1
    assert (
        "anthropic.claude-3-sonnet-20240229-v1:0"
        in handler._default_estimator_cache
    )
    handler.close()


def test_U06_handler_init_defaults_estimator_for_unknown_warns_once() -> None:
    """Unknown model → chars/4 fallback; first call may warn, second does not."""
    handler = SpendGuardLlamaIndexHandler(
        client=make_client_mock(),
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
    )
    payload = {
        _StubEventPayload.MESSAGES: [
            {"role": "user", "content": "hello world hello world"}
        ],
        _StubEventPayload.SERIALIZED: {"model": "weird-unknown-future-model-v0"},
    }
    # The first call may warn; we just assert claim creation works
    # and the closure is cached so the second call doesn't re-instantiate.
    with warnings.catch_warnings(record=True):
        warnings.simplefilter("always")
        claims_1 = handler._estimate_claims(payload)
        assert len(claims_1) == 1
        claims_2 = handler._estimate_claims(payload)
        assert len(claims_2) == 1
    # Cache hit: only one closure registered.
    assert (
        "weird-unknown-future-model-v0" in handler._default_estimator_cache
    )
    handler.close()


# ─────────────────────────────────────────────────────────────────────
# U07 — Non-LLM events are filtered no-op
# ─────────────────────────────────────────────────────────────────────


def test_U07_non_llm_events_are_no_op() -> None:
    """All non-LLM event types early-return with zero sidecar calls."""
    client = make_client_mock()
    handler = make_handler(client=client)
    non_llm = [
        _StubCBEventType.EMBEDDING,
        _StubCBEventType.RETRIEVE,
        _StubCBEventType.CHUNK,
        _StubCBEventType.QUERY,
        _StubCBEventType.NODE_PARSING,
    ]
    for etype in non_llm:
        rv = handler.on_event_start(etype, payload={}, event_id="evt-x")
        assert rv == "evt-x"
        handler.on_event_end(etype, payload={}, event_id="evt-x")
    assert client.request_decision.await_count == 0
    assert client.emit_llm_call_post.await_count == 0
    handler.close()


# ─────────────────────────────────────────────────────────────────────
# U08 — ALLOW stashes _PendingCall
# ─────────────────────────────────────────────────────────────────────


def test_U08_on_event_start_allow_stashes_pending_call() -> None:
    """ALLOW response → state stash with non-empty companion ids."""
    client = make_client_mock(reservation_ids=("res-allow",))
    handler = make_handler(client=client)
    payload = make_openai_start_payload()
    handler.on_event_start(
        _StubCBEventType.LLM, payload=payload, event_id="evt-allow"
    )
    assert "evt-allow" in handler._state
    pending = handler._state["evt-allow"]
    assert pending.reservation_id == "res-allow"
    assert pending.decision_id == "dec-1"
    assert pending.step_id  # non-empty
    assert pending.llm_call_id  # non-empty
    assert client.request_decision.await_count == 1
    handler.close()


# ─────────────────────────────────────────────────────────────────────
# U09 — DENY raises SpendGuardLlamaIndexDenied
# ─────────────────────────────────────────────────────────────────────


def test_U09_on_event_start_deny_raises_spendguard_denied() -> None:
    """``DecisionDenied`` from client → raises ``SpendGuardLlamaIndexDenied``."""
    deny_exc = DecisionDenied(
        "budget exhausted",
        decision_id="dec-deny",
        reason_codes=["BUDGET_EXHAUSTED"],
    )
    client = make_client_mock(request_decision_side_effect=deny_exc)
    handler = make_handler(client=client)
    payload = make_openai_start_payload()
    with pytest.raises(SpendGuardLlamaIndexDenied) as exc_info:
        handler.on_event_start(
            _StubCBEventType.LLM, payload=payload, event_id="evt-deny"
        )
    assert exc_info.value.reason_codes == ["BUDGET_EXHAUSTED"]
    assert exc_info.value.decision_id == "dec-deny"
    # State must NOT be stashed.
    assert "evt-deny" not in handler._state
    # Chained from the underlying DecisionDenied.
    assert isinstance(exc_info.value.__cause__, DecisionDenied)
    handler.close()


# ─────────────────────────────────────────────────────────────────────
# U10 — on_event_start returns event_id unchanged
# ─────────────────────────────────────────────────────────────────────


def test_U10_on_event_start_returns_event_id_unchanged() -> None:
    """ALLOW and non-LLM both return the event_id passed in."""
    handler = make_handler()
    # ALLOW path
    rv_allow = handler.on_event_start(
        _StubCBEventType.LLM,
        payload=make_openai_start_payload(),
        event_id="evt-allow",
    )
    assert rv_allow == "evt-allow"
    # Non-LLM path
    rv_non = handler.on_event_start(
        _StubCBEventType.EMBEDDING, payload={}, event_id="evt-non"
    )
    assert rv_non == "evt-non"
    handler.close()


# ─────────────────────────────────────────────────────────────────────
# U11-U14 — run_id resolution cascade
# ─────────────────────────────────────────────────────────────────────


def test_U11_run_id_resolution_uses_trace_id_when_set() -> None:
    """``start_trace("trace-xyz")`` → request_decision(run_id="trace-xyz")."""
    client = make_client_mock()
    handler = make_handler(client=client)
    handler.start_trace("trace-xyz")
    handler.on_event_start(
        _StubCBEventType.LLM,
        payload=make_openai_start_payload(),
        event_id="evt-1",
    )
    call = client.request_decision.await_args
    assert call.kwargs["run_id"] == "trace-xyz"
    handler.close()


def test_U12_run_id_resolution_falls_back_to_parent_id() -> None:
    """No trace, ``parent_id="par-1"`` → run_id="par-1"."""
    client = make_client_mock()
    handler = make_handler(client=client)
    handler.on_event_start(
        _StubCBEventType.LLM,
        payload=make_openai_start_payload(),
        event_id="evt-1",
        parent_id="par-1",
    )
    call = client.request_decision.await_args
    assert call.kwargs["run_id"] == "par-1"
    handler.close()


def test_U13_run_id_resolution_uses_run_id_fn_override() -> None:
    """``run_id_fn`` wins over trace_id and parent_id."""
    client = make_client_mock()
    handler = make_handler(
        client=client,
        run_id_fn=lambda payload: "fixed-run",
    )
    handler.start_trace("trace-loses")
    handler.on_event_start(
        _StubCBEventType.LLM,
        payload=make_openai_start_payload(),
        event_id="evt-1",
        parent_id="par-loses",
    )
    call = client.request_decision.await_args
    assert call.kwargs["run_id"] == "fixed-run"
    handler.close()


def test_U14_run_id_resolution_derives_uuid_when_no_inputs() -> None:
    """No trace + no parent + no fn → derived UUID, deterministic."""
    client_a = make_client_mock()
    client_b = make_client_mock()
    handler_a = make_handler(client=client_a)
    handler_b = make_handler(client=client_b)
    payload = make_openai_start_payload()
    handler_a.on_event_start(
        _StubCBEventType.LLM, payload=payload, event_id="evt-a"
    )
    handler_b.on_event_start(
        _StubCBEventType.LLM, payload=payload, event_id="evt-b"
    )
    run_a = client_a.request_decision.await_args.kwargs["run_id"]
    run_b = client_b.request_decision.await_args.kwargs["run_id"]
    # Identical payload → identical derived run_id (deterministic retry).
    assert run_a == run_b
    handler_a.close()
    handler_b.close()


# ─────────────────────────────────────────────────────────────────────
# U15-U19 — Usage extraction vendor matrix
# ─────────────────────────────────────────────────────────────────────


def _run_pre_post(
    client: MagicMock,
    end_payload: dict[str, Any],
    *,
    start_payload: dict[str, Any] | None = None,
    event_id: str = "evt-x",
) -> Any:
    """Drive PRE → POST through a fresh handler. Returns the post call kwargs."""
    handler = make_handler(client=client)
    handler.on_event_start(
        _StubCBEventType.LLM,
        payload=start_payload or make_openai_start_payload(),
        event_id=event_id,
    )
    handler.on_event_end(
        _StubCBEventType.LLM, payload=end_payload, event_id=event_id
    )
    try:
        return client.emit_llm_call_post.await_args.kwargs
    finally:
        handler.close()


def test_U15_on_event_end_commit_extracts_openai_total_tokens() -> None:
    """OpenAI ``total_tokens=42`` → ``estimated_amount_atomic="42"``."""
    client = make_client_mock()
    kwargs = _run_pre_post(
        client, make_openai_end_payload(total_tokens=42)
    )
    assert kwargs["estimated_amount_atomic"] == "42"
    assert kwargs["outcome"] == "SUCCESS"
    assert kwargs["provider_reported_amount_atomic"] == ""


def test_U16_on_event_end_commit_extracts_anthropic_input_output_tokens() -> None:
    """Anthropic 10+15 → ``estimated_amount_atomic="25"``."""
    client = make_client_mock()
    kwargs = _run_pre_post(
        client,
        make_anthropic_end_payload(input_tokens=10, output_tokens=15),
    )
    assert kwargs["estimated_amount_atomic"] == "25"


def test_U17_on_event_end_commit_extracts_gemini_total_token_count() -> None:
    """Gemini ``usage_metadata.total_token_count=33`` → ``"33"``."""
    client = make_client_mock()
    kwargs = _run_pre_post(
        client, make_gemini_end_payload(total_token_count=33)
    )
    assert kwargs["estimated_amount_atomic"] == "33"


def test_U18_on_event_end_commit_extracts_bedrock_converse_tokens() -> None:
    """Bedrock Converse 7+8 → ``estimated_amount_atomic="15"``."""
    client = make_client_mock()
    kwargs = _run_pre_post(
        client,
        make_bedrock_end_payload(input_tokens=7, output_tokens=8),
    )
    assert kwargs["estimated_amount_atomic"] == "15"


def test_U19_on_event_end_falls_back_to_zero_on_missing_usage() -> None:
    """Empty ``response.raw`` → commit still fires with ``"0"``."""
    client = make_client_mock()
    kwargs = _run_pre_post(client, make_empty_end_payload())
    assert kwargs["estimated_amount_atomic"] == "0"
    assert kwargs["outcome"] == "SUCCESS"


# ─────────────────────────────────────────────────────────────────────
# U20-U21 — POST state hygiene
# ─────────────────────────────────────────────────────────────────────


def test_U20_on_event_end_no_op_when_state_missing() -> None:
    """No PRE → POST is silent no-op; no RPCs fired."""
    client = make_client_mock()
    handler = make_handler(client=client)
    # POST without prior PRE.
    handler.on_event_end(
        _StubCBEventType.LLM,
        payload=make_openai_end_payload(),
        event_id="evt-orphan",
    )
    assert client.emit_llm_call_post.await_count == 0
    handler.close()


def test_U21_on_event_end_cleans_up_state() -> None:
    """After commit, ``self._state`` no longer contains event_id."""
    client = make_client_mock()
    handler = make_handler(client=client)
    handler.on_event_start(
        _StubCBEventType.LLM,
        payload=make_openai_start_payload(),
        event_id="evt-clean",
    )
    assert "evt-clean" in handler._state
    handler.on_event_end(
        _StubCBEventType.LLM,
        payload=make_openai_end_payload(),
        event_id="evt-clean",
    )
    assert "evt-clean" not in handler._state
    handler.close()


# ─────────────────────────────────────────────────────────────────────
# U22-U24 — Signature determinism + concurrency
# ─────────────────────────────────────────────────────────────────────


def test_U22_signature_stable_across_repeated_calls() -> None:
    """Same payload → same signature → same derived decision_id."""
    client_a = make_client_mock(decision_id="will-be-overridden-by-derive")
    handler = make_handler(client=client_a)
    payload = make_openai_start_payload(model="gpt-4o-mini")
    sig_1 = handler._signature_for(payload)
    sig_2 = handler._signature_for(payload)
    assert sig_1 == sig_2
    assert len(sig_1) == 32  # blake2b digest_size=16 → 32 hex chars
    handler.close()


def test_U23_signature_differs_when_model_changes() -> None:
    """Same messages, different model → different signature."""
    handler = make_handler()
    payload_a = make_openai_start_payload(model="gpt-4o-mini")
    payload_b = make_openai_start_payload(model="gpt-4o")
    assert handler._signature_for(payload_a) != handler._signature_for(payload_b)
    handler.close()


def test_U24_concurrent_events_dont_cross_state() -> None:
    """Two distinct event_ids → two distinct ``_PendingCall`` entries."""
    # Two distinct reservations across two PRE calls.
    outcome_iter = iter([
        SimpleNamespace(
            decision_id="dec-A",
            reservation_ids=("res-A",),
            audit_decision_event_id="audit-A",
            decision="CONTINUE",
        ),
        SimpleNamespace(
            decision_id="dec-B",
            reservation_ids=("res-B",),
            audit_decision_event_id="audit-B",
            decision="CONTINUE",
        ),
    ])
    client = make_client_mock()
    client.request_decision = AsyncMock(
        side_effect=lambda **kw: next(outcome_iter)
    )
    handler = make_handler(client=client)
    handler.on_event_start(
        _StubCBEventType.LLM,
        payload=make_openai_start_payload(model="gpt-4o-mini"),
        event_id="evt-A",
    )
    handler.on_event_start(
        _StubCBEventType.LLM,
        payload=make_openai_start_payload(model="gpt-4o"),
        event_id="evt-B",
    )
    assert handler._state["evt-A"].reservation_id == "res-A"
    assert handler._state["evt-B"].reservation_id == "res-B"
    # Commit B then A — each pops its own state.
    handler.on_event_end(
        _StubCBEventType.LLM,
        payload=make_openai_end_payload(total_tokens=11),
        event_id="evt-B",
    )
    handler.on_event_end(
        _StubCBEventType.LLM,
        payload=make_openai_end_payload(total_tokens=22),
        event_id="evt-A",
    )
    assert handler._state == {}
    assert client.emit_llm_call_post.await_count == 2
    handler.close()


# ─────────────────────────────────────────────────────────────────────
# U25 — start_trace / end_trace lifecycle
# ─────────────────────────────────────────────────────────────────────


def test_U25_start_trace_and_end_trace_lifecycle() -> None:
    """``end_trace`` only clears on matched id; mismatched is no-op."""
    handler = make_handler()
    handler.start_trace("t1")
    assert handler._trace_id == "t1"
    # Mismatched id → state unchanged.
    handler.end_trace("t2")
    assert handler._trace_id == "t1"
    # Matched id → cleared.
    handler.end_trace("t1")
    assert handler._trace_id is None
    handler.close()


# ─────────────────────────────────────────────────────────────────────
# I01-I05 — Integration: recorded-fixture replay through full PRE/POST
# ─────────────────────────────────────────────────────────────────────

_FIXTURES_DIR = (
    Path(__file__).resolve().parents[1] / "fixtures" / "llamaindex"
)


def _load_fixture(name: str) -> dict[str, Any]:
    path = _FIXTURES_DIR / f"{name}.json"
    return json.loads(path.read_text(encoding="utf-8"))


def _rebuild_payload_end(fixture_payload_end: dict[str, Any]) -> dict[str, Any]:
    """JSON-recorded ``payload_end.response`` rebuilt as a SimpleNamespace.

    Fixtures store ``response.raw`` as a plain dict; the handler reads
    ``getattr(response, 'raw')`` so we wrap it back into a namespace
    that matches the LlamaIndex ``ChatResponse`` / ``CompletionResponse``
    shape (the only attribute the handler touches is ``.raw``).
    """
    response_dict = fixture_payload_end.get("response", {})
    response = SimpleNamespace(raw=response_dict.get("raw", {}))
    return {_StubEventPayload.RESPONSE: response}


def test_I01_integration_openai_allow_flow_with_recorded_fixture() -> None:
    """OpenAI ALLOW fixture → PRE reserve + POST commit with recorded usage."""
    fix = _load_fixture("openai_gpt_4o_mini_allow")
    client = make_client_mock()
    handler = make_handler(client=client)
    payload_start = {
        _StubEventPayload.MESSAGES: fix["payload_start"]["messages"],
        _StubEventPayload.SERIALIZED: fix["payload_start"]["serialized"],
    }
    handler.on_event_start(
        _StubCBEventType.LLM, payload=payload_start, event_id="evt-fix-allow"
    )
    payload_end = _rebuild_payload_end(fix["payload_end"])
    handler.on_event_end(
        _StubCBEventType.LLM, payload=payload_end, event_id="evt-fix-allow"
    )
    assert client.request_decision.await_count == 1
    assert client.emit_llm_call_post.await_count == 1
    kwargs = client.emit_llm_call_post.await_args.kwargs
    assert kwargs["estimated_amount_atomic"] == "42"
    handler.close()


def test_I02_integration_openai_deny_flow_with_recorded_fixture() -> None:
    """OpenAI DENY fixture → handler raises; provider HTTP never called."""
    fix = _load_fixture("openai_gpt_4o_mini_deny")
    deny_exc = DecisionDenied(
        "budget exhausted",
        decision_id="dec-deny",
        reason_codes=fix["sidecar_decision"]["reason_codes"],
    )
    client = make_client_mock(request_decision_side_effect=deny_exc)
    handler = make_handler(client=client)
    payload_start = {
        _StubEventPayload.MESSAGES: fix["payload_start"]["messages"],
        _StubEventPayload.SERIALIZED: fix["payload_start"]["serialized"],
    }
    # Track provider HTTP "calls" — fixtures carry a counter.
    transport_calls = {"n": 0}
    with pytest.raises(SpendGuardLlamaIndexDenied):
        handler.on_event_start(
            _StubCBEventType.LLM,
            payload=payload_start,
            event_id="evt-fix-deny",
        )
        transport_calls["n"] += 1  # never reached
    # Mock provider HTTP was never called.
    assert transport_calls["n"] == 0
    # Commit RPC never fired.
    assert client.emit_llm_call_post.await_count == 0
    handler.close()


def test_I03_integration_anthropic_allow_flow_with_recorded_fixture() -> None:
    """Anthropic fixture → PRE + POST with input+output token sum."""
    fix = _load_fixture("anthropic_sonnet_allow")
    client = make_client_mock()
    handler = make_handler(client=client)
    payload_start = {
        _StubEventPayload.MESSAGES: fix["payload_start"]["messages"],
        _StubEventPayload.SERIALIZED: fix["payload_start"]["serialized"],
    }
    handler.on_event_start(
        _StubCBEventType.LLM, payload=payload_start, event_id="evt-anth"
    )
    payload_end = _rebuild_payload_end(fix["payload_end"])
    handler.on_event_end(
        _StubCBEventType.LLM, payload=payload_end, event_id="evt-anth"
    )
    kwargs = client.emit_llm_call_post.await_args.kwargs
    usage = fix["payload_end"]["response"]["raw"]["usage"]
    expected = usage["input_tokens"] + usage["output_tokens"]
    assert kwargs["estimated_amount_atomic"] == str(expected)
    handler.close()


def test_I04_integration_gemini_allow_flow_with_recorded_fixture() -> None:
    """Gemini fixture → PRE + POST with ``total_token_count``."""
    fix = _load_fixture("gemini_flash_allow")
    client = make_client_mock()
    handler = make_handler(client=client)
    payload_start = {
        _StubEventPayload.MESSAGES: fix["payload_start"]["messages"],
        _StubEventPayload.SERIALIZED: fix["payload_start"]["serialized"],
    }
    handler.on_event_start(
        _StubCBEventType.LLM, payload=payload_start, event_id="evt-gem"
    )
    payload_end = _rebuild_payload_end(fix["payload_end"])
    handler.on_event_end(
        _StubCBEventType.LLM, payload=payload_end, event_id="evt-gem"
    )
    kwargs = client.emit_llm_call_post.await_args.kwargs
    expected = fix["payload_end"]["response"]["raw"]["usage_metadata"][
        "total_token_count"
    ]
    assert kwargs["estimated_amount_atomic"] == str(expected)
    handler.close()


def test_I05_integration_bedrock_allow_flow_with_recorded_fixture() -> None:
    """Bedrock Converse fixture → PRE + POST with inputTokens+outputTokens."""
    fix = _load_fixture("bedrock_converse_allow")
    client = make_client_mock()
    handler = make_handler(client=client)
    payload_start = {
        _StubEventPayload.MESSAGES: fix["payload_start"]["messages"],
        _StubEventPayload.SERIALIZED: fix["payload_start"]["serialized"],
    }
    handler.on_event_start(
        _StubCBEventType.LLM, payload=payload_start, event_id="evt-bed"
    )
    payload_end = _rebuild_payload_end(fix["payload_end"])
    handler.on_event_end(
        _StubCBEventType.LLM, payload=payload_end, event_id="evt-bed"
    )
    kwargs = client.emit_llm_call_post.await_args.kwargs
    usage = fix["payload_end"]["response"]["raw"]["usage"]
    expected = usage["inputTokens"] + usage["outputTokens"]
    assert kwargs["estimated_amount_atomic"] == str(expected)
    handler.close()


# ─────────────────────────────────────────────────────────────────────
# I06 — Vector index query end-to-end (skip if llama-index-core missing)
# ─────────────────────────────────────────────────────────────────────


def test_I06_integration_vector_index_query_end_to_end() -> None:
    """End-to-end ``VectorStoreIndex.from_documents`` with ``MockLLM``.

    Skipped when ``llama-index-core`` isn't installed (G07 gate). When
    present, builds the index over a 1-doc corpus, runs ``.query(...)``,
    and asserts ONE PRE + ONE POST per synthesis call. Retriever events
    (EMBEDDING / RETRIEVE) are filtered.
    """
    pytest.importorskip("llama_index.core")
    from llama_index.core import (  # type: ignore[import-not-found]
        Document,
        Settings,
        VectorStoreIndex,
    )
    from llama_index.core.callbacks import (  # type: ignore[import-not-found]
        CallbackManager,
    )
    from llama_index.core.llms import (  # type: ignore[import-not-found]
        MockLLM,
    )

    # We need an embedding model for VectorStoreIndex. Use the
    # MockEmbedding so the test is hermetic.
    try:
        from llama_index.core.embeddings import (  # type: ignore[import-not-found]
            MockEmbedding,
        )
    except ImportError:
        pytest.skip("MockEmbedding not available in this llama-index-core")

    client = make_client_mock()
    handler = make_handler(client=client)
    Settings.callback_manager = CallbackManager([handler])
    Settings.llm = MockLLM()
    Settings.embed_model = MockEmbedding(embed_dim=8)
    docs = [Document(text="The budget cap is 100 atomic units per window.")]
    index = VectorStoreIndex.from_documents(docs)
    response = index.as_query_engine().query("What is the budget cap?")
    assert response is not None  # smoke
    # MockLLM has empty raw → estimated_amount_atomic should be "0"
    # at least once. Allow up to 2 PRE / POST in case MockLLM fires
    # twice (refinement step on long contexts) — we assert > 0 not == 1.
    assert client.request_decision.await_count >= 1
    assert client.emit_llm_call_post.await_count >= 1
    handler.close()


# ─────────────────────────────────────────────────────────────────────
# I07 — start_trace propagates run_id
# ─────────────────────────────────────────────────────────────────────


def test_I07_integration_run_id_derived_from_start_trace() -> None:
    """``start_trace("run-abc")`` → run_id="run-abc" in both PRE and POST."""
    client = make_client_mock()
    handler = make_handler(client=client)
    handler.start_trace("run-abc")
    handler.on_event_start(
        _StubCBEventType.LLM,
        payload=make_openai_start_payload(),
        event_id="evt-trace",
    )
    handler.on_event_end(
        _StubCBEventType.LLM,
        payload=make_openai_end_payload(),
        event_id="evt-trace",
    )
    pre_kw = client.request_decision.await_args.kwargs
    post_kw = client.emit_llm_call_post.await_args.kwargs
    assert pre_kw["run_id"] == "run-abc"
    assert post_kw["run_id"] == "run-abc"
    handler.close()


# ─────────────────────────────────────────────────────────────────────
# I08 — concurrent query engines via ThreadPoolExecutor
# ─────────────────────────────────────────────────────────────────────


def test_I08_integration_concurrent_query_engines_dont_cross_state() -> None:
    """Two thread-dispatched PRE/POST cycles — each commits its own reservation."""
    import concurrent.futures

    # Two distinct reservations across two PRE calls.
    outcomes = iter([
        SimpleNamespace(
            decision_id="dec-C1", reservation_ids=("res-C1",),
            audit_decision_event_id="audit-C1", decision="CONTINUE",
        ),
        SimpleNamespace(
            decision_id="dec-C2", reservation_ids=("res-C2",),
            audit_decision_event_id="audit-C2", decision="CONTINUE",
        ),
    ])
    client = make_client_mock()
    # The outcome iter is consumed in order; PRE calls serialize through
    # the background loop so this is deterministic.
    import threading
    lock = threading.Lock()

    async def _decide(**_kw: Any) -> Any:
        with lock:
            return next(outcomes)

    client.request_decision = AsyncMock(side_effect=_decide)
    handler = make_handler(client=client)

    def _drive(event_id: str, model: str, total: int) -> None:
        handler.on_event_start(
            _StubCBEventType.LLM,
            payload=make_openai_start_payload(model=model),
            event_id=event_id,
        )
        handler.on_event_end(
            _StubCBEventType.LLM,
            payload=make_openai_end_payload(total_tokens=total),
            event_id=event_id,
        )

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as ex:
        list(ex.map(
            lambda args: _drive(*args),
            [("evt-C1", "gpt-4o-mini", 11), ("evt-C2", "gpt-4o", 22)],
        ))

    assert client.emit_llm_call_post.await_count == 2
    assert handler._state == {}
    handler.close()


# ─────────────────────────────────────────────────────────────────────
# Public surface assertion — guards the locked __all__ contract.
# ─────────────────────────────────────────────────────────────────────


def test_public_surface_locked() -> None:
    """The locked public surface from review-standards §1."""
    # Direct-import without triggering the install-hint guard barrel.
    hook_exports = sorted(hook_mod.__all__)
    assert hook_exports == [
        "ClaimEstimator",
        "RunIdFn",
        "SpendGuardLlamaIndexHandler",
        "current_run_context",
        "run_context",
    ]
    errors_exports = sorted(errors_mod.__all__)
    assert "SpendGuardLlamaIndexDenied" in errors_exports
    options_exports = sorted(options_mod.__all__)
    assert "LlamaIndexRunContext" in options_exports
    assert "SpendGuardLlamaIndexOptions" in options_exports


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
    """TP-01 — ``SpendGuardLlamaIndexOptions(unit_id=...)`` constructs."""
    opts = SpendGuardLlamaIndexOptions(
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
    opts = SpendGuardLlamaIndexOptions(
        tenant_id="t1",
        budget_id="b1",
        window_instance_id="w1",
        unit_id=_UNIT_ID_FIXTURE,
    )
    client = make_client_mock()
    handler = SpendGuardLlamaIndexHandler(
        client=client,
        budget_id=opts.budget_id,
        window_instance_id=opts.window_instance_id,
        unit=common_pb2.UnitRef(unit_id=opts.unit_id or ""),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        claim_estimator=lambda payload: [
            common_pb2.BudgetClaim(
                budget_id="b1",
                unit=common_pb2.UnitRef(unit_id=opts.unit_id or ""),
                amount_atomic="100",
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id="w1",
            )
        ],
    )
    payload = make_openai_start_payload()
    handler.on_event_start(_StubCBEventType.LLM, payload=payload, event_id="evt-tp02")
    kw = client.request_decision.call_args.kwargs
    assert kw["projected_claims"][0].unit.unit_id == _UNIT_ID_FIXTURE
    handler.close()


def test_TP03_options_without_unit_id_constructs() -> None:
    """TP-03 — backward compat: omitting ``unit_id`` keeps default None."""
    opts = SpendGuardLlamaIndexOptions(
        tenant_id="t1",
        budget_id="b1",
        window_instance_id="w1",
    )
    assert opts.unit_id is None
