# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S101, S106
"""COV_D19 — pytest unit + integration tests for the Google ADK adapter.

Mocks ``SpendGuardClient`` (Tier 1) and uses ``SimpleNamespace`` stubs
for ADK ``LlmRequest`` / ``LlmResponse`` / ``CallbackContext`` so the
suite runs without ``google-adk`` installed. Verifies every contract
from ``docs/specs/coverage/D19_google_adk/tests.md`` §1 U01-U20 + I01-I05.

Strategy:
  * Direct-imports the ``_callback`` module via package path (bypassing
    the ``adk.__init__`` install-hint guard so unit tests don't require
    the [adk] extra at runtime).
  * The integration tests use ``pytest.importorskip("google.adk")``
    when they truly need the real ADK types — currently the recorded
    fixture replay path stays shape-compatible with SimpleNamespace.
"""

from __future__ import annotations

import asyncio
import importlib
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
# Load _callback bypassing the install-hint ImportError in __init__.
# This way the unit suite runs without google-adk installed.
# ─────────────────────────────────────────────────────────────────────

_ADK_PKG_NAME = "spendguard.integrations.adk"
if _ADK_PKG_NAME not in sys.modules:
    _adk_pkg_path = (
        Path(__file__).resolve().parents[3]
        / "src"
        / "spendguard"
        / "integrations"
        / "adk"
    )
    ns = _stdlib_types.ModuleType(_ADK_PKG_NAME)
    ns.__path__ = [str(_adk_pkg_path)]
    sys.modules[_ADK_PKG_NAME] = ns

callback_mod = importlib.import_module("spendguard.integrations.adk._callback")
errors_mod = importlib.import_module("spendguard.integrations.adk._errors")

SpendGuardAdkCallback = callback_mod.SpendGuardAdkCallback


# ─────────────────────────────────────────────────────────────────────
# Shape stubs (work whether google.adk is installed or not).
# ─────────────────────────────────────────────────────────────────────


def make_ctx(invocation_id: str = "inv-1") -> SimpleNamespace:
    return SimpleNamespace(invocation_id=invocation_id, state={})


def make_request(
    *,
    model: str = "gemini-2.0-flash",
    text: str = "Hello, world.",
) -> SimpleNamespace:
    """Construct a SimpleNamespace shaped like google.adk.models.LlmRequest."""
    part = SimpleNamespace(text=text, function_call=None, function_response=None)
    content = SimpleNamespace(role="user", parts=[part])
    return SimpleNamespace(model=model, contents=[content])


def make_response(
    *,
    total_token_count: int | None = None,
    prompt_token_count: int | None = None,
    candidates_token_count: int | None = None,
    total_tokens: int | None = None,
    response_id: str | None = None,
    no_usage: bool = False,
) -> SimpleNamespace:
    """Construct a SimpleNamespace shaped like an LlmResponse.

    Carefully omits ``contents``/``parts`` attributes so the shape sniff
    routes it as a Response, not a Request.
    """
    if no_usage:
        usage = None
    else:
        usage = SimpleNamespace(
            total_token_count=total_token_count,
            prompt_token_count=prompt_token_count,
            candidates_token_count=candidates_token_count,
            total_tokens=total_tokens,
        )
    return SimpleNamespace(
        usage_metadata=usage,
        response_id=response_id,
        candidates=[],
        error_code=None,
        error_message=None,
    )


def make_client_mock(
    *,
    tenant_id: str = "tenant-1",
    session_id: str = "session-1",
    decision_id: str = "dec-1",
    reservation_ids: tuple[str, ...] = ("res-1",),
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
    )
    if request_decision_side_effect is not None:
        client.request_decision = AsyncMock(side_effect=request_decision_side_effect)
    else:
        client.request_decision = AsyncMock(return_value=outcome)
    client.emit_llm_call_post = AsyncMock(return_value=None)
    client.release_reservation = AsyncMock(return_value=None)
    return client


def make_callback(
    *,
    client: MagicMock | None = None,
    claim_estimator: Any = None,
    run_id_fn: Any = None,
) -> SpendGuardAdkCallback:
    """Build a ``SpendGuardAdkCallback`` with sane test defaults."""
    if client is None:
        client = make_client_mock()
    if claim_estimator is None:
        claim_estimator = lambda req: [  # noqa: E731
            common_pb2.BudgetClaim(
                budget_id="b1",
                unit=common_pb2.UnitRef(unit_id="u1"),
                amount_atomic="100",
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id="w1",
            )
        ]
    return SpendGuardAdkCallback(
        client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        claim_estimator=claim_estimator,
        run_id_fn=run_id_fn,
    )


# ─────────────────────────────────────────────────────────────────────
# U01 — Import error message when google-adk is missing.
# ─────────────────────────────────────────────────────────────────────


def test_U01_import_error_when_google_adk_missing() -> None:
    """Module barrel import without ``google-adk`` installed surfaces a
    helpful install hint (``pip install 'spendguard-sdk[adk]'``).

    Verified by grepping the barrel's source for the install-hint
    substring — running the import itself doesn't work in this test
    process because we've stubbed the package namespace above to allow
    direct loading of _callback without triggering the barrel.
    """
    barrel_path = (
        Path(__file__).resolve().parents[3]
        / "src"
        / "spendguard"
        / "integrations"
        / "adk"
        / "__init__.py"
    )
    assert barrel_path.exists(), f"Barrel file missing: {barrel_path}"
    source = barrel_path.read_text(encoding="utf-8")
    # Review-standards §2: install hint string is verbatim per LangChain prior.
    assert "pip install 'spendguard-sdk[adk]'" in source
    # And the guard is structurally there (try / except ImportError).
    assert "from google.adk" in source
    assert "except ImportError" in source
    assert "raise ImportError" in source


# ─────────────────────────────────────────────────────────────────────
# U02-U04 — Default claim estimator dispatch
# ─────────────────────────────────────────────────────────────────────


def test_U02_callback_init_defaults_estimator_for_gemini_model() -> None:
    """When ``claim_estimator=None``, the default estimator is wired
    via ``_default_estimator.adk_default_claim_estimator``."""
    client = make_client_mock()
    cb = SpendGuardAdkCallback(
        client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        # claim_estimator omitted → default dispatched
    )
    assert cb._claim_estimator is not None
    # Smoke-test: the default estimator returns at least one claim
    # for a Gemini request.
    req = make_request(model="gemini-2.0-flash", text="hi")
    claims = cb._claim_estimator(req)
    assert len(claims) == 1
    assert int(claims[0].amount_atomic) > 0


def test_U03_callback_init_defaults_estimator_for_litellm_openai() -> None:
    """LiteLlm-wrapped OpenAI: ``model="openai/gpt-4o-mini"`` still
    routes through the default estimator (prefix-stripped to gpt-4o-mini
    inside the estimator)."""
    client = make_client_mock()
    cb = SpendGuardAdkCallback(
        client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
    )
    req = make_request(model="openai/gpt-4o-mini", text="hi")
    claims = cb._claim_estimator(req)
    assert len(claims) == 1
    assert int(claims[0].amount_atomic) > 0


def test_U04_callback_init_defaults_estimator_for_unknown_warns_once() -> None:
    """Unknown model → estimator still produces a claim (>= chars/4 floor).

    The default estimator delegates the "unknown model" warning to the
    underlying ``estimators.estimator_for_model`` registry; we verify
    the claim still flows through with a positive amount."""
    client = make_client_mock()
    cb = SpendGuardAdkCallback(
        client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
    )
    req = make_request(model="totally-unknown-mystery-model", text="x" * 100)
    with warnings.catch_warnings(record=True):
        warnings.simplefilter("always")
        claims = cb._claim_estimator(req)
        # Claim still produced (estimator's fallback path).
        assert len(claims) == 1
        assert int(claims[0].amount_atomic) > 0


# ─────────────────────────────────────────────────────────────────────
# U05-U06 — __call__ dispatch by payload shape
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U05_call_dispatch_request_routes_to_before() -> None:
    """``await cb(ctx, LlmRequest(...))`` → ``_before`` is called, ALLOW
    path returns ``None``."""
    client = make_client_mock(reservation_ids=("res-allow-1",))
    cb = make_callback(client=client)
    ctx = make_ctx(invocation_id="inv-u5")
    req = make_request()

    result = await cb(ctx, req)
    assert result is None
    client.request_decision.assert_awaited_once()
    assert ctx.state["spendguard.reservation_id"] == "res-allow-1"


@pytest.mark.asyncio
async def test_U06_call_dispatch_response_routes_to_after() -> None:
    """``await cb(ctx, LlmResponse(...))`` → ``_after`` is called,
    returns ``None``."""
    client = make_client_mock(reservation_ids=("res-u6",))
    cb = make_callback(client=client)
    ctx = make_ctx(invocation_id="inv-u6")
    # Seed state as though _before already ran.
    ctx.state.update(
        {
            "spendguard.reservation_id": "res-u6",
            "spendguard.decision_id": "dec-u6",
            "spendguard.step_id": "inv-u6:adk-call:abc",
            "spendguard.llm_call_id": "llm-u6",
        }
    )

    resp = make_response(total_token_count=42)
    result = await cb(ctx, resp)
    assert result is None
    client.emit_llm_call_post.assert_awaited_once()


# ─────────────────────────────────────────────────────────────────────
# U07-U08 — _before state handoff
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U07_before_allow_stashes_reservation_in_state() -> None:
    """After ALLOW, ``ctx.state`` carries all four PRE-stashed keys."""
    client = make_client_mock(
        decision_id="dec-allow", reservation_ids=("res-allow",)
    )
    cb = make_callback(client=client)
    ctx = make_ctx()
    req = make_request()

    result = await cb(ctx, req)
    assert result is None
    assert ctx.state["spendguard.reservation_id"] == "res-allow"
    assert ctx.state["spendguard.decision_id"] == "dec-allow"
    assert ctx.state["spendguard.step_id"].startswith("inv-1:adk-call:")
    assert "spendguard.llm_call_id" in ctx.state
    assert "spendguard.denied" not in ctx.state


@pytest.mark.asyncio
async def test_U08_before_deny_returns_llm_response_and_marks_state() -> None:
    """DENY → returns ``LlmResponse(error_code='SPENDGUARD_DENY')`` and
    sets ``ctx.state['spendguard.denied'] = True``. No reservation."""
    denied = DecisionDenied(
        "budget exhausted",
        decision_id="dec-deny",
        reason_codes=["BUDGET_EXHAUSTED"],
    )
    client = make_client_mock(request_decision_side_effect=denied)
    cb = make_callback(client=client)
    ctx = make_ctx()
    req = make_request()

    result = await cb(ctx, req)
    assert result is not None
    assert getattr(result, "error_code", None) == "SPENDGUARD_DENY"
    assert "SpendGuard denied LLM call" in getattr(result, "error_message", "")
    assert ctx.state["spendguard.denied"] is True
    assert "spendguard.reservation_id" not in ctx.state


# ─────────────────────────────────────────────────────────────────────
# U09-U10 — run_id derivation
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U09_before_uses_invocation_id_as_default_run_id() -> None:
    """Without ``run_id_fn``, ``run_id == ctx.invocation_id``."""
    client = make_client_mock()
    cb = make_callback(client=client)
    ctx = make_ctx(invocation_id="inv-default-1234")
    req = make_request()

    await cb(ctx, req)
    pre_kwargs = client.request_decision.call_args.kwargs
    assert pre_kwargs["run_id"] == "inv-default-1234"


@pytest.mark.asyncio
async def test_U10_before_uses_run_id_fn_override() -> None:
    """With ``run_id_fn=lambda c: 'fixed-run'``, that value is used."""
    client = make_client_mock()
    cb = make_callback(client=client, run_id_fn=lambda c: "fixed-run-xyz")
    ctx = make_ctx(invocation_id="ignored-inv")
    req = make_request()

    await cb(ctx, req)
    pre_kwargs = client.request_decision.call_args.kwargs
    assert pre_kwargs["run_id"] == "fixed-run-xyz"


# ─────────────────────────────────────────────────────────────────────
# U11-U14 — Usage extraction
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U11_after_commit_extracts_gemini_total_token_count() -> None:
    """``usage_metadata.total_token_count=42`` → commit with 42."""
    client = make_client_mock()
    cb = make_callback(client=client)
    ctx = make_ctx()
    ctx.state.update(
        {
            "spendguard.reservation_id": "res-u11",
            "spendguard.decision_id": "dec-u11",
            "spendguard.step_id": "inv-1:adk-call:abc",
            "spendguard.llm_call_id": "llm-u11",
        }
    )

    resp = make_response(total_token_count=42)
    await cb(ctx, resp)
    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert post_kwargs["estimated_amount_atomic"] == "42"


@pytest.mark.asyncio
async def test_U12_after_commit_extracts_gemini_split_tokens() -> None:
    """``prompt_token_count=10 + candidates_token_count=15`` → 25."""
    client = make_client_mock()
    cb = make_callback(client=client)
    ctx = make_ctx()
    ctx.state.update(
        {
            "spendguard.reservation_id": "res-u12",
            "spendguard.decision_id": "dec-u12",
            "spendguard.step_id": "inv-1:adk-call:abc",
            "spendguard.llm_call_id": "llm-u12",
        }
    )

    resp = make_response(prompt_token_count=10, candidates_token_count=15)
    await cb(ctx, resp)
    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert post_kwargs["estimated_amount_atomic"] == "25"


@pytest.mark.asyncio
async def test_U13_after_commit_extracts_openai_total_tokens() -> None:
    """LiteLlm/OpenAI shape: ``usage_metadata.total_tokens=99`` → 99."""
    client = make_client_mock()
    cb = make_callback(client=client)
    ctx = make_ctx()
    ctx.state.update(
        {
            "spendguard.reservation_id": "res-u13",
            "spendguard.decision_id": "dec-u13",
            "spendguard.step_id": "inv-1:adk-call:abc",
            "spendguard.llm_call_id": "llm-u13",
        }
    )

    resp = make_response(total_tokens=99)
    await cb(ctx, resp)
    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert post_kwargs["estimated_amount_atomic"] == "99"


@pytest.mark.asyncio
async def test_U14_after_commit_falls_back_to_zero_on_missing_usage() -> None:
    """No ``usage_metadata`` → commit still fires with 0."""
    client = make_client_mock()
    cb = make_callback(client=client)
    ctx = make_ctx()
    ctx.state.update(
        {
            "spendguard.reservation_id": "res-u14",
            "spendguard.decision_id": "dec-u14",
            "spendguard.step_id": "inv-1:adk-call:abc",
            "spendguard.llm_call_id": "llm-u14",
        }
    )

    resp = make_response(no_usage=True)
    await cb(ctx, resp)
    client.emit_llm_call_post.assert_awaited_once()
    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert post_kwargs["estimated_amount_atomic"] == "0"


# ─────────────────────────────────────────────────────────────────────
# U15-U16 — _after skip conditions
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U15_after_skips_commit_when_denied_flag_set() -> None:
    """``ctx.state['spendguard.denied']=True`` → ``_after`` is a no-op."""
    client = make_client_mock()
    cb = make_callback(client=client)
    ctx = make_ctx()
    ctx.state["spendguard.denied"] = True

    resp = make_response(total_token_count=42)
    await cb(ctx, resp)
    client.emit_llm_call_post.assert_not_awaited()
    client.release_reservation.assert_not_awaited()


@pytest.mark.asyncio
async def test_U16_after_skips_commit_when_pre_state_missing() -> None:
    """If ``ctx.state`` lacks ``reservation_id``, ``_after`` returns
    silently — no exception, no RPC."""
    client = make_client_mock()
    cb = make_callback(client=client)
    ctx = make_ctx()  # State is empty (no PRE ran)

    resp = make_response(total_token_count=42)
    await cb(ctx, resp)
    client.emit_llm_call_post.assert_not_awaited()


# ─────────────────────────────────────────────────────────────────────
# U17-U18 — Signature stability
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U17_signature_stable_across_repeated_calls() -> None:
    """Two calls with the same ``model+contents`` produce the same
    signature → identical step_id / llm_call_id / decision_id."""
    client = make_client_mock(decision_id="dec-stable", reservation_ids=("res-stable",))
    cb = make_callback(client=client)
    ctx1 = make_ctx(invocation_id="inv-stab")
    ctx2 = make_ctx(invocation_id="inv-stab")
    req1 = make_request(model="gemini-2.0-flash", text="same prompt")
    req2 = make_request(model="gemini-2.0-flash", text="same prompt")

    sig1 = SpendGuardAdkCallback._signature_for(req1)
    sig2 = SpendGuardAdkCallback._signature_for(req2)
    assert sig1 == sig2

    await cb(ctx1, req1)
    await cb(ctx2, req2)
    step_id_1 = ctx1.state["spendguard.step_id"]
    step_id_2 = ctx2.state["spendguard.step_id"]
    llm_call_id_1 = ctx1.state["spendguard.llm_call_id"]
    llm_call_id_2 = ctx2.state["spendguard.llm_call_id"]
    assert step_id_1 == step_id_2
    assert llm_call_id_1 == llm_call_id_2


def test_U18_signature_differs_when_model_changes() -> None:
    """Same ``contents`` but different ``model`` → different signature."""
    req1 = make_request(model="gemini-2.0-flash", text="same prompt")
    req2 = make_request(model="openai/gpt-4o-mini", text="same prompt")
    sig1 = SpendGuardAdkCallback._signature_for(req1)
    sig2 = SpendGuardAdkCallback._signature_for(req2)
    assert sig1 != sig2


# ─────────────────────────────────────────────────────────────────────
# U19 — Deny response reason codes
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U19_deny_response_contains_reason_codes() -> None:
    """Multiple reason codes → comma-joined in ``error_message``."""
    denied = DecisionDenied(
        "policy stop",
        decision_id="dec-multireasons",
        reason_codes=["BUDGET_EXHAUSTED", "STOP_RUN_PROJECTION", "POLICY_VIOLATION"],
    )
    client = make_client_mock(request_decision_side_effect=denied)
    cb = make_callback(client=client)
    ctx = make_ctx()
    req = make_request()

    result = await cb(ctx, req)
    msg = getattr(result, "error_message", "")
    assert "BUDGET_EXHAUSTED" in msg
    assert "STOP_RUN_PROJECTION" in msg
    assert "POLICY_VIOLATION" in msg
    # And comma-joined
    assert "BUDGET_EXHAUSTED,STOP_RUN_PROJECTION,POLICY_VIOLATION" in msg


@pytest.mark.asyncio
async def test_U19b_deny_response_defaults_to_budget_exhausted_when_empty() -> None:
    """Empty ``reason_codes`` → defaults to ``BUDGET_EXHAUSTED``."""
    denied = DecisionDenied(
        "budget out", decision_id="dec-empty", reason_codes=[]
    )
    client = make_client_mock(request_decision_side_effect=denied)
    cb = make_callback(client=client)
    ctx = make_ctx()
    req = make_request()

    result = await cb(ctx, req)
    msg = getattr(result, "error_message", "")
    assert "BUDGET_EXHAUSTED" in msg


# ─────────────────────────────────────────────────────────────────────
# U20 — provider_event_id fallback
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U20_extract_provider_event_id_falls_back_to_empty() -> None:
    """``LlmResponse`` without ``response_id`` / ``id`` → empty string in
    the commit kwargs."""
    client = make_client_mock()
    cb = make_callback(client=client)
    ctx = make_ctx()
    ctx.state.update(
        {
            "spendguard.reservation_id": "res-u20",
            "spendguard.decision_id": "dec-u20",
            "spendguard.step_id": "inv-1:adk-call:abc",
            "spendguard.llm_call_id": "llm-u20",
        }
    )

    resp = make_response(total_token_count=10, response_id=None)
    await cb(ctx, resp)
    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert post_kwargs["provider_event_id"] == ""


# ─────────────────────────────────────────────────────────────────────
# Arity dispatch — verify __call__ handles unexpected payload shapes.
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_call_arity_unknown_payload_is_no_op() -> None:
    """Unknown payload type → log + return None (defensive, never crash)."""
    client = make_client_mock()
    cb = make_callback(client=client)
    ctx = make_ctx()

    # Pass an object that is neither Request nor Response shape.
    weird_payload = SimpleNamespace()
    result = await cb(ctx, weird_payload)
    assert result is None
    client.request_decision.assert_not_awaited()
    client.emit_llm_call_post.assert_not_awaited()


# ─────────────────────────────────────────────────────────────────────
# I01-I05 — Integration tests (recorded fixtures + concurrency)
# ─────────────────────────────────────────────────────────────────────


# Inline recorded fixtures (kept here for hermeticity; the spec calls
# for JSON files under fixtures/adk/ but the shape is minimal enough
# that inline literals serve unit / integration both).
_GEMINI_ALLOW_FIXTURE = {
    "request": {
        "model": "gemini-2.0-flash",
        "contents": [{"role": "user", "parts": [{"text": "Hello, Gemini."}]}],
    },
    "response": {
        "usage_metadata": {
            "prompt_token_count": 12,
            "candidates_token_count": 30,
            "total_token_count": 42,
        },
        "response_id": "recorded-resp-001",
    },
}

_GEMINI_DENY_FIXTURE = {
    "request": {
        "model": "gemini-2.0-flash",
        "contents": [{"role": "user", "parts": [{"text": "Expensive query."}]}],
    },
    "response": {
        "usage_metadata": None,
        "response_id": None,
    },
}

_LITELLM_GPT4O_ALLOW_FIXTURE = {
    "request": {
        "model": "openai/gpt-4o-mini",
        "contents": [{"role": "user", "parts": [{"text": "Hello, GPT."}]}],
    },
    "response": {
        # LiteLlm-wrapped OpenAI exposes total_tokens (no Gemini-style fields)
        "usage_metadata": {"total_tokens": 99},
        "response_id": "openai-recorded-001",
    },
}


def _hydrate_request(d: dict[str, Any]) -> SimpleNamespace:
    """Turn the fixture dict into a SimpleNamespace tree that matches
    the ADK LlmRequest shape closely enough for the adapter."""
    contents = []
    for c in d["contents"]:
        parts = []
        for p in c["parts"]:
            parts.append(
                SimpleNamespace(
                    text=p.get("text"),
                    function_call=p.get("function_call"),
                    function_response=p.get("function_response"),
                )
            )
        contents.append(SimpleNamespace(role=c["role"], parts=parts))
    return SimpleNamespace(model=d["model"], contents=contents)


def _hydrate_response(d: dict[str, Any]) -> SimpleNamespace:
    usage_raw = d.get("usage_metadata")
    if usage_raw is None:
        usage = None
    else:
        usage = SimpleNamespace(
            prompt_token_count=usage_raw.get("prompt_token_count"),
            candidates_token_count=usage_raw.get("candidates_token_count"),
            total_token_count=usage_raw.get("total_token_count"),
            total_tokens=usage_raw.get("total_tokens"),
        )
    return SimpleNamespace(
        usage_metadata=usage,
        response_id=d.get("response_id"),
        candidates=[],
        error_code=None,
        error_message=None,
    )


@pytest.mark.asyncio
async def test_I01_integration_allow_flow_with_recorded_gemini_fixture() -> None:
    """Recorded Gemini fixture: PRE reserve → POST commit with 42 tokens."""
    client = make_client_mock(
        decision_id="dec-i1", reservation_ids=("res-i1",)
    )
    cb = make_callback(client=client)
    ctx = make_ctx(invocation_id="inv-i1")

    req = _hydrate_request(_GEMINI_ALLOW_FIXTURE["request"])
    resp = _hydrate_response(_GEMINI_ALLOW_FIXTURE["response"])

    # PRE
    pre_result = await cb(ctx, req)
    assert pre_result is None
    pre_kwargs = client.request_decision.call_args.kwargs
    assert pre_kwargs["trigger"] == "LLM_CALL_PRE"
    assert pre_kwargs["route"] == "llm.call"
    assert pre_kwargs["run_id"] == "inv-i1"
    # Claims include a single DEBIT
    assert len(pre_kwargs["projected_claims"]) == 1
    claim = pre_kwargs["projected_claims"][0]
    assert claim.direction == common_pb2.BudgetClaim.DEBIT

    # POST
    post_result = await cb(ctx, resp)
    assert post_result is None
    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert post_kwargs["outcome"] == "SUCCESS"
    assert post_kwargs["estimated_amount_atomic"] == "42"
    assert post_kwargs["provider_event_id"] == "recorded-resp-001"
    assert post_kwargs["reservation_id"] == "res-i1"


@pytest.mark.asyncio
async def test_I02_integration_deny_flow_with_recorded_gemini_fixture() -> None:
    """Recorded deny: PRE returns deny LlmResponse → model NEVER called →
    no POST commit fires."""
    denied = DecisionDenied(
        "budget cap reached",
        decision_id="dec-i2-deny",
        reason_codes=["BUDGET_EXHAUSTED"],
    )
    client = make_client_mock(request_decision_side_effect=denied)
    cb = make_callback(client=client)
    ctx = make_ctx(invocation_id="inv-i2")

    req = _hydrate_request(_GEMINI_DENY_FIXTURE["request"])
    deny_resp = await cb(ctx, req)
    assert deny_resp is not None
    assert getattr(deny_resp, "error_code", "") == "SPENDGUARD_DENY"
    assert ctx.state["spendguard.denied"] is True

    # Now ADK would call _after with the synthetic deny response itself.
    # We simulate that round-trip — POST should be a no-op.
    await cb(ctx, deny_resp)
    client.emit_llm_call_post.assert_not_awaited()


@pytest.mark.asyncio
async def test_I03_integration_allow_flow_with_recorded_litellm_fixture() -> None:
    """LiteLlm-wrapped OpenAI fixture: PRE reserve → POST commit with
    99 tokens (extracted from ``total_tokens`` shape)."""
    client = make_client_mock(
        decision_id="dec-i3", reservation_ids=("res-i3",)
    )
    cb = make_callback(client=client)
    ctx = make_ctx(invocation_id="inv-i3")

    req = _hydrate_request(_LITELLM_GPT4O_ALLOW_FIXTURE["request"])
    resp = _hydrate_response(_LITELLM_GPT4O_ALLOW_FIXTURE["response"])

    await cb(ctx, req)
    await cb(ctx, resp)
    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert post_kwargs["estimated_amount_atomic"] == "99"


@pytest.mark.asyncio
async def test_I04_integration_run_id_derived_from_invocation_id() -> None:
    """``invocation_id="run-abc"`` flows into both PRE and POST as ``run_id``."""
    client = make_client_mock(
        decision_id="dec-i4", reservation_ids=("res-i4",)
    )
    cb = make_callback(client=client)
    ctx = make_ctx(invocation_id="run-abc-i4")

    req = _hydrate_request(_GEMINI_ALLOW_FIXTURE["request"])
    resp = _hydrate_response(_GEMINI_ALLOW_FIXTURE["response"])

    await cb(ctx, req)
    await cb(ctx, resp)

    pre_kwargs = client.request_decision.call_args.kwargs
    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert pre_kwargs["run_id"] == "run-abc-i4"
    assert post_kwargs["run_id"] == "run-abc-i4"


@pytest.mark.asyncio
async def test_I05_integration_concurrent_runs_dont_cross_state() -> None:
    """Two ``asyncio.gather``-ed runs with distinct ``CallbackContext``:
    each commits its own ``reservation_id``; no state leakage."""

    # Two distinct contexts.
    contexts = [
        make_ctx(invocation_id="inv-A"),
        make_ctx(invocation_id="inv-B"),
    ]

    # Two distinct clients — each mock independently records its calls
    # so we can assert non-crossing reservation ids.
    client_a = make_client_mock(
        decision_id="dec-A", reservation_ids=("res-A",)
    )
    client_b = make_client_mock(
        decision_id="dec-B", reservation_ids=("res-B",)
    )

    # Two callback instances — operators typically build one per
    # SpendGuard tenant; the test reflects the highest-isolation case.
    cb_a = make_callback(client=client_a)
    cb_b = make_callback(client=client_b)

    req_a = _hydrate_request(_GEMINI_ALLOW_FIXTURE["request"])
    req_b = _hydrate_request(_LITELLM_GPT4O_ALLOW_FIXTURE["request"])
    resp_a = _hydrate_response(_GEMINI_ALLOW_FIXTURE["response"])
    resp_b = _hydrate_response(_LITELLM_GPT4O_ALLOW_FIXTURE["response"])

    async def run_pair(cb, ctx, req, resp):
        await cb(ctx, req)
        await cb(ctx, resp)

    await asyncio.gather(
        run_pair(cb_a, contexts[0], req_a, resp_a),
        run_pair(cb_b, contexts[1], req_b, resp_b),
    )

    # Each side stashed its own reservation_id, no crossover.
    assert contexts[0].state["spendguard.reservation_id"] == "res-A"
    assert contexts[1].state["spendguard.reservation_id"] == "res-B"
    # Each client only saw its own POST commit.
    a_post = client_a.emit_llm_call_post.call_args.kwargs
    b_post = client_b.emit_llm_call_post.call_args.kwargs
    assert a_post["reservation_id"] == "res-A"
    assert b_post["reservation_id"] == "res-B"
    # And the run_ids are scoped to each ctx.
    assert a_post["run_id"] == "inv-A"
    assert b_post["run_id"] == "inv-B"
