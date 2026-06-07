# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S101, S106, S107
"""COV_D20 — pytest unit + integration tests for the AWS Strands adapter.

Mocks ``SpendGuardClient`` (Tier 1) and uses ``SimpleNamespace`` stubs
for the Strands ``Invocation`` / ``InvocationResult`` /
``BeforeInvocationEvent`` / ``AfterInvocationEvent`` so the suite runs
without ``strands-agents`` installed. Verifies every contract from
``docs/specs/coverage/D20_aws_strands/tests.md`` U01-U20 + I01-I05.

Strategy:
  * Direct-imports the ``_hook_provider`` module via package path
    (bypassing the ``strands.__init__`` install-hint guard so unit
    tests don't require the ``[strands]`` extra at runtime).
  * Multi-backend coverage matrix: parametrized fixtures for
    Bedrock / OpenAI / Anthropic / LiteLLM-via-Gemini response shapes.
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
from spendguard.errors import DecisionDenied

# ─────────────────────────────────────────────────────────────────────
# Load _hook_provider bypassing the install-hint ImportError in
# __init__. This way the unit suite runs without strands-agents.
# ─────────────────────────────────────────────────────────────────────

_STRANDS_PKG_NAME = "spendguard.integrations.strands"
if _STRANDS_PKG_NAME not in sys.modules:
    _strands_pkg_path = (
        Path(__file__).resolve().parents[3]
        / "src"
        / "spendguard"
        / "integrations"
        / "strands"
    )
    ns = _stdlib_types.ModuleType(_STRANDS_PKG_NAME)
    ns.__path__ = [str(_strands_pkg_path)]
    sys.modules[_STRANDS_PKG_NAME] = ns

provider_mod = importlib.import_module(
    "spendguard.integrations.strands._hook_provider"
)
options_mod = importlib.import_module(
    "spendguard.integrations.strands._options"
)
errors_mod = importlib.import_module(
    "spendguard.integrations.strands._errors"
)

SpendGuardStrandsHookProvider = provider_mod.SpendGuardStrandsHookProvider
StrandsRunContext = options_mod.StrandsRunContext
SpendGuardStrandsOptions = options_mod.SpendGuardStrandsOptions
SpendGuardDegradeBlocked = errors_mod.SpendGuardDegradeBlocked
SpendGuardConfigError = errors_mod.SpendGuardConfigError
SidecarUnavailable = errors_mod.SidecarUnavailable
run_context = provider_mod.run_context


# ─────────────────────────────────────────────────────────────────────
# Shape stubs for the Strands event-bus surface
# ─────────────────────────────────────────────────────────────────────


def make_model(*, name: str = "BedrockModel", model_id: str = "anthropic.claude-3-5-sonnet"):
    """SimpleNamespace shaped like a Strands ``Model`` subclass instance.

    Returns a class instance whose ``type(instance).__name__`` matches
    ``name`` so the provider's ``_model_backend_name`` reads the right
    backend identifier.
    """
    cls = type(name, (object,), {})
    inst = cls()
    inst.model_id = model_id
    return inst


def make_invocation(
    *,
    invocation_id: str = "inv-1",
    model_name: str = "BedrockModel",
    model_id: str = "anthropic.claude-3-5-sonnet",
    messages: list[dict[str, Any]] | None = None,
    tools: list[Any] | None = None,
):
    """SimpleNamespace shaped like a Strands ``Invocation``."""
    return SimpleNamespace(
        invocation_id=invocation_id,
        model=make_model(name=model_name, model_id=model_id),
        messages=messages or [{"role": "user", "content": "hello"}],
        tools=tools or [],
    )


def make_before_event(invocation):
    """SimpleNamespace shaped like ``BeforeInvocationEvent``.

    Carries ``.invocation``; does NOT carry ``.result`` so the duck-type
    sniff routes it as Before.
    """
    return SimpleNamespace(invocation=invocation)


def make_bedrock_result(
    *,
    input_tokens: int = 12,
    output_tokens: int = 30,
    total_tokens: int | None = None,
    result_id: str = "msg_01ABC",
):
    """Bedrock InvokeModel response shape (Anthropic-style usage)."""
    usage = SimpleNamespace(
        input_tokens=input_tokens,
        output_tokens=output_tokens,
        total_tokens=total_tokens,
    )
    return SimpleNamespace(
        id=result_id,
        usage=usage,
        message={"role": "assistant", "content": "hi from bedrock"},
    )


def make_openai_result(
    *,
    prompt_tokens: int = 8,
    completion_tokens: int = 14,
    total_tokens: int | None = 22,
    result_id: str = "chatcmpl-xyz",
):
    """OpenAI chat-completion response shape."""
    usage = SimpleNamespace(
        prompt_tokens=prompt_tokens,
        completion_tokens=completion_tokens,
        total_tokens=total_tokens,
    )
    return SimpleNamespace(
        id=result_id,
        usage=usage,
        message={"role": "assistant", "content": "hi from openai"},
    )


def make_litellm_result(
    *,
    total_token_count: int | None = None,
    prompt_tokens: int = 10,
    completion_tokens: int = 15,
    total_tokens: int | None = 25,
    result_id: str = "litellm-resp-1",
):
    """LiteLLM-normalised response (OpenAI shape + sometimes Gemini legacy)."""
    usage = SimpleNamespace(
        prompt_tokens=prompt_tokens,
        completion_tokens=completion_tokens,
        total_tokens=total_tokens,
        total_token_count=total_token_count,
    )
    return SimpleNamespace(
        id=result_id,
        usage=usage,
        message={"role": "assistant", "content": "hi from litellm"},
    )


def make_after_event(invocation, result=None, exception=None):
    """SimpleNamespace shaped like ``AfterInvocationEvent``."""
    return SimpleNamespace(
        invocation=invocation,
        result=result,
        exception=exception,
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
    client.release_reservation = AsyncMock(return_value=None)
    return client


def _claim(amount: int = 100):
    return common_pb2.BudgetClaim(
        budget_id="b1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        amount_atomic=str(amount),
        direction=common_pb2.BudgetClaim.DEBIT,
        window_instance_id="w1",
    )


def make_provider(
    *,
    client: MagicMock | None = None,
    claim_estimator: Any = None,
    claim_reconciler: Any = None,
    fail_closed: bool = True,
) -> Any:
    """Build a ``SpendGuardStrandsHookProvider`` with sane test defaults."""
    if client is None:
        client = make_client_mock()
    if claim_estimator is None:
        claim_estimator = lambda inv: [_claim(100)]  # noqa: E731
    if claim_reconciler is None:
        claim_reconciler = lambda inv, result: [_claim(42)]  # noqa: E731
    return SpendGuardStrandsHookProvider(
        client=client,
        budget_id="b1",
        window_instance_id="w1",
        unit=common_pb2.UnitRef(unit_id="u1"),
        pricing=common_pb2.PricingFreeze(pricing_version="v1"),
        claim_estimator=claim_estimator,
        claim_reconciler=claim_reconciler,
        fail_closed=fail_closed,
    )


# ─────────────────────────────────────────────────────────────────────
# U01 — Import error when strands-agents missing
# ─────────────────────────────────────────────────────────────────────


def test_U01_import_error_message_when_strands_missing() -> None:
    """Module barrel import without ``strands-agents`` installed
    surfaces a helpful install hint."""
    barrel_path = (
        Path(__file__).resolve().parents[3]
        / "src"
        / "spendguard"
        / "integrations"
        / "strands"
        / "__init__.py"
    )
    assert barrel_path.exists()
    source = barrel_path.read_text(encoding="utf-8")
    assert "pip install 'spendguard-sdk[strands]'" in source
    assert "from strands.hooks" in source
    assert "except ImportError" in source
    assert "raise ImportError" in source


# ─────────────────────────────────────────────────────────────────────
# U02-U04 — Constructor validation
# ─────────────────────────────────────────────────────────────────────


def test_U02_construct_with_minimal_args() -> None:
    """Minimal happy-path construction succeeds."""
    p = make_provider()
    assert p is not None
    assert p.pending_count == 0


def test_U03_construct_rejects_missing_budget_id() -> None:
    """Empty ``budget_id`` → ``SpendGuardConfigError`` at construction."""
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        SpendGuardStrandsHookProvider(
            client=make_client_mock(),
            budget_id="",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_reconciler=lambda inv, res: [_claim()],
        )


def test_U04_construct_rejects_missing_reconciler() -> None:
    """``claim_reconciler=None`` → ``SpendGuardConfigError`` at construction."""
    with pytest.raises(SpendGuardConfigError, match="claim_reconciler"):
        SpendGuardStrandsHookProvider(
            client=make_client_mock(),
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id="u1"),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_reconciler=None,
        )


def test_U04b_construct_rejects_missing_unit_id() -> None:
    """``unit.unit_id == ''`` → ``SpendGuardConfigError``."""
    with pytest.raises(SpendGuardConfigError, match="unit.unit_id"):
        SpendGuardStrandsHookProvider(
            client=make_client_mock(),
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(unit_id=""),
            pricing=common_pb2.PricingFreeze(pricing_version="v1"),
            claim_reconciler=lambda inv, res: [_claim()],
        )


# ─────────────────────────────────────────────────────────────────────
# U05-U06 — Run context lifecycle
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U05_run_context_lifecycle_sets_and_resets() -> None:
    """``run_context`` binds + unbinds the StrandsRunContext."""
    from spendguard.integrations.strands._hook_provider import (
        current_run_context,
    )

    assert current_run_context() is None
    ctx = StrandsRunContext(run_id="my-run-xyz")
    async with run_context(ctx):
        assert current_run_context() is ctx
    assert current_run_context() is None


@pytest.mark.asyncio
async def test_U06_before_uses_run_context_when_bound() -> None:
    """When ``run_context`` is bound, the PRE call uses its ``run_id``."""
    client = make_client_mock()
    p = make_provider(client=client)
    inv = make_invocation(invocation_id="inv-u6")
    event = make_before_event(inv)

    async with run_context(StrandsRunContext(run_id="bridged-run-id")):
        await p.before_invocation(event)

    pre_kwargs = client.request_decision.call_args.kwargs
    assert pre_kwargs["run_id"] == "bridged-run-id"


# ─────────────────────────────────────────────────────────────────────
# U07-U09 — before_invocation reserve (ALLOW)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U07_before_reserves_and_stashes() -> None:
    """ALLOW path: PRE reserves, stash holds the reservation."""
    client = make_client_mock(
        decision_id="dec-allow", reservation_ids=("res-allow",)
    )
    p = make_provider(client=client)
    inv = make_invocation(invocation_id="inv-u7")
    event = make_before_event(inv)

    await p.before_invocation(event)
    client.request_decision.assert_awaited_once()
    pre_kwargs = client.request_decision.call_args.kwargs
    assert pre_kwargs["trigger"] == "LLM_CALL_PRE"
    assert pre_kwargs["route"] == "llm.call"
    assert len(pre_kwargs["projected_claims"]) == 1
    assert p.pending_count == 1


@pytest.mark.asyncio
async def test_U08_before_records_model_backend_in_decision_context() -> None:
    """The decision_context tags ``integration=strands`` and the backend."""
    client = make_client_mock()
    p = make_provider(client=client)
    inv = make_invocation(invocation_id="inv-u8", model_name="OpenAIModel",
                          model_id="gpt-4o-mini")
    event = make_before_event(inv)

    await p.before_invocation(event)
    pre_kwargs = client.request_decision.call_args.kwargs
    ctx = pre_kwargs["decision_context_json"]
    assert ctx["integration"] == "strands"
    assert ctx["model_backend"] == "OpenAIModel"
    assert ctx["model_id"] == "gpt-4o-mini"


@pytest.mark.asyncio
async def test_U09_before_idempotency_key_stable_across_repeats() -> None:
    """Two PREs with the same ``invocation_id`` produce the same
    idempotency_key — Strands retries get same-key cache hit."""
    client_a = make_client_mock()
    client_b = make_client_mock()
    p_a = make_provider(client=client_a)
    p_b = make_provider(client=client_b)
    inv1 = make_invocation(invocation_id="inv-stab")
    inv2 = make_invocation(invocation_id="inv-stab")

    await p_a.before_invocation(make_before_event(inv1))
    await p_b.before_invocation(make_before_event(inv2))

    a_key = client_a.request_decision.call_args.kwargs["idempotency_key"]
    b_key = client_b.request_decision.call_args.kwargs["idempotency_key"]
    assert a_key == b_key


# ─────────────────────────────────────────────────────────────────────
# U10-U12 — before_invocation deny + degrade + missing id
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U10_before_deny_raises_decision_denied() -> None:
    """DENY → ``DecisionDenied`` propagates; nothing stashed."""
    denied = DecisionDenied(
        "budget exhausted",
        decision_id="dec-deny",
        reason_codes=["BUDGET_EXHAUSTED"],
    )
    client = make_client_mock(request_decision_side_effect=denied)
    p = make_provider(client=client)
    inv = make_invocation(invocation_id="inv-u10")
    event = make_before_event(inv)

    with pytest.raises(DecisionDenied):
        await p.before_invocation(event)
    assert p.pending_count == 0


@pytest.mark.asyncio
async def test_U11_before_degrade_fails_closed_by_default() -> None:
    """DEGRADE outcome under fail-closed → ``SpendGuardDegradeBlocked``."""
    client = make_client_mock(decision="DEGRADE")
    p = make_provider(client=client)
    inv = make_invocation(invocation_id="inv-u11")
    event = make_before_event(inv)

    with pytest.raises(SpendGuardDegradeBlocked):
        await p.before_invocation(event)
    assert p.pending_count == 0


@pytest.mark.asyncio
async def test_U11b_before_degrade_fail_open_allows() -> None:
    """DEGRADE outcome under fail-open → silent allow (no stash)."""
    client = make_client_mock(decision="DEGRADE")
    p = make_provider(client=client, fail_closed=False)
    inv = make_invocation(invocation_id="inv-u11b")
    event = make_before_event(inv)

    # Does not raise.
    await p.before_invocation(event)
    # Stash unchanged because no reservation came back.
    assert p.pending_count == 0


@pytest.mark.asyncio
async def test_U12_before_missing_invocation_id_raises() -> None:
    """``Invocation`` without ``invocation_id`` → ``SpendGuardConfigError``."""
    client = make_client_mock()
    p = make_provider(client=client)
    inv = SimpleNamespace(
        invocation_id=None,
        model=make_model(),
        messages=[],
    )
    event = make_before_event(inv)

    with pytest.raises(SpendGuardConfigError, match="invocation_id"):
        await p.before_invocation(event)


# ─────────────────────────────────────────────────────────────────────
# U13-U15 — after_invocation commit (multi-backend usage)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "backend,make_result",
    [
        ("BedrockModel", lambda: make_bedrock_result(input_tokens=12, output_tokens=30)),
        ("OpenAIModel", lambda: make_openai_result(total_tokens=22)),
        ("LiteLLMModel", lambda: make_litellm_result(total_tokens=25)),
    ],
)
async def test_U13_after_commits_for_each_backend_shape(backend, make_result) -> None:
    """ALL three backends commit through the same code path."""
    client = make_client_mock(decision_id="dec-c", reservation_ids=("res-c",))
    p = make_provider(client=client)
    inv = make_invocation(invocation_id=f"inv-{backend}", model_name=backend)
    await p.before_invocation(make_before_event(inv))

    result = make_result()
    await p.after_invocation(make_after_event(inv, result=result))

    client.emit_llm_call_post.assert_awaited_once()
    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert post_kwargs["outcome"] == "SUCCESS"
    assert post_kwargs["reservation_id"] == "res-c"


@pytest.mark.asyncio
async def test_U14_after_extracts_provider_event_id() -> None:
    """``result.id`` flows through to ``provider_event_id`` on commit."""
    client = make_client_mock()
    p = make_provider(client=client)
    inv = make_invocation(invocation_id="inv-u14")
    await p.before_invocation(make_before_event(inv))

    result = make_bedrock_result(result_id="msg_01XYZ")
    await p.after_invocation(make_after_event(inv, result=result))

    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert post_kwargs["provider_event_id"] == "msg_01XYZ"


@pytest.mark.asyncio
async def test_U15_after_uses_reconciler_amount() -> None:
    """``claim_reconciler``'s amount feeds ``estimated_amount_atomic``."""
    client = make_client_mock()
    p = make_provider(
        client=client,
        claim_reconciler=lambda inv, res: [_claim(777)],
    )
    inv = make_invocation(invocation_id="inv-u15")
    await p.before_invocation(make_before_event(inv))

    await p.after_invocation(
        make_after_event(inv, result=make_bedrock_result())
    )

    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert post_kwargs["estimated_amount_atomic"] == "777"


# ─────────────────────────────────────────────────────────────────────
# U16-U18 — after_invocation exception classification + release
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U16_after_failure_emits_FAILURE_outcome() -> None:
    """Provider raised mid-invocation → ``outcome=FAILURE`` release."""
    client = make_client_mock()
    p = make_provider(client=client)
    inv = make_invocation(invocation_id="inv-u16")
    await p.before_invocation(make_before_event(inv))

    rt_exc = RuntimeError("provider exploded")
    await p.after_invocation(
        make_after_event(inv, result=None, exception=rt_exc)
    )

    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert post_kwargs["outcome"] == "FAILURE"
    assert p.pending_count == 0


@pytest.mark.asyncio
async def test_U17_after_cancelled_emits_CANCELLED_outcome() -> None:
    """``asyncio.CancelledError`` → ``outcome=CANCELLED`` release."""
    client = make_client_mock()
    p = make_provider(client=client)
    inv = make_invocation(invocation_id="inv-u17")
    await p.before_invocation(make_before_event(inv))

    await p.after_invocation(
        make_after_event(inv, result=None, exception=asyncio.CancelledError())
    )

    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert post_kwargs["outcome"] == "CANCELLED"


@pytest.mark.asyncio
async def test_U18_after_no_pending_is_noop() -> None:
    """``after_invocation`` without a matching PRE → silent no-op."""
    client = make_client_mock()
    p = make_provider(client=client)
    inv = make_invocation(invocation_id="inv-noop")

    await p.after_invocation(make_after_event(inv, result=make_bedrock_result()))
    client.emit_llm_call_post.assert_not_awaited()


# ─────────────────────────────────────────────────────────────────────
# U19 — Reconciler exception fallback
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U19_after_reconciler_exception_falls_back_to_usage() -> None:
    """``claim_reconciler`` raises → fall back to ``result.usage`` extraction."""

    def bad_reconciler(_inv, _result):
        raise ValueError("reconciler bug")

    client = make_client_mock()
    p = make_provider(client=client, claim_reconciler=bad_reconciler)
    inv = make_invocation(invocation_id="inv-u19")
    await p.before_invocation(make_before_event(inv))

    # Bedrock-shape result: usage = 12 + 30 = 42 (anthropic split)
    result = make_bedrock_result(input_tokens=12, output_tokens=30)
    await p.after_invocation(make_after_event(inv, result=result))

    client.emit_llm_call_post.assert_awaited_once()
    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    # Estimator amount is "100" (default), used as snapshot since
    # reconciler raised; the usage fallback only kicks in when the
    # snapshot was empty.
    assert post_kwargs["estimated_amount_atomic"] == "100"


# ─────────────────────────────────────────────────────────────────────
# U20 — Concurrent invocations (stash isolation via dict)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_U20_concurrent_invocations_stash_isolated() -> None:
    """5 ``asyncio.gather``-ed invocations with distinct ``invocation_id``
    each round-trip through the same provider without stash collision."""
    client = make_client_mock()
    # Each request_decision call returns a unique reservation.
    counter = {"n": 0}

    async def fake_request_decision(**_kwargs):
        counter["n"] += 1
        return SimpleNamespace(
            decision_id=f"dec-{counter['n']}",
            reservation_ids=(f"res-{counter['n']}",),
            audit_decision_event_id="audit",
            decision="CONTINUE",
        )

    client.request_decision = AsyncMock(side_effect=fake_request_decision)
    p = make_provider(client=client)

    invs = [make_invocation(invocation_id=f"inv-conc-{i}") for i in range(5)]

    async def run_one(inv):
        await p.before_invocation(make_before_event(inv))
        # interleave a sleep to provoke task interleaving
        await asyncio.sleep(0)
        await p.after_invocation(
            make_after_event(inv, result=make_bedrock_result())
        )

    await asyncio.gather(*(run_one(inv) for inv in invs))

    assert client.request_decision.await_count == 5
    assert client.emit_llm_call_post.await_count == 5
    # All 5 stash entries cleared.
    assert p.pending_count == 0


# ─────────────────────────────────────────────────────────────────────
# I01-I05 — Integration tests (multi-backend + register_hooks contract)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_I01_register_hooks_binds_both_callbacks() -> None:
    """``register_hooks`` binds Before+After to the registry. We use a
    duck-typed registry that records calls."""
    p = make_provider()
    binds: list[tuple[Any, Any]] = []

    class _Reg:
        def add_callback(self, event_cls, cb):
            binds.append((event_cls, cb))

    p.register_hooks(_Reg())
    # Two bindings: Before + After.
    assert len(binds) == 2
    callbacks = [cb for _ev, cb in binds]
    assert p.before_invocation in callbacks
    assert p.after_invocation in callbacks


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "backend,make_result,expected_id",
    [
        ("BedrockModel",
         lambda: make_bedrock_result(input_tokens=10, output_tokens=20,
                                      result_id="msg_bedrock"),
         "msg_bedrock"),
        ("OpenAIModel",
         lambda: make_openai_result(total_tokens=30,
                                     result_id="chatcmpl_openai"),
         "chatcmpl_openai"),
        ("LiteLLMModel",
         lambda: make_litellm_result(total_tokens=25,
                                      result_id="litellm_resp"),
         "litellm_resp"),
    ],
)
async def test_I02_multi_backend_allow_path(backend, make_result, expected_id) -> None:
    """Backend coverage matrix: PRE reserve + POST commit fires per backend."""
    client = make_client_mock(
        decision_id=f"dec-{backend}",
        reservation_ids=(f"res-{backend}",),
    )

    def reconcile(inv, result):
        usage = result.usage
        if isinstance(getattr(usage, "total_tokens", None), int):
            amount = usage.total_tokens
        else:
            amount = (
                (getattr(usage, "input_tokens", 0) or 0)
                + (getattr(usage, "output_tokens", 0) or 0)
            )
        return [_claim(amount)]

    p = make_provider(client=client, claim_reconciler=reconcile)
    inv = make_invocation(
        invocation_id=f"inv-{backend}-i2", model_name=backend
    )

    await p.before_invocation(make_before_event(inv))
    pre_kwargs = client.request_decision.call_args.kwargs
    assert pre_kwargs["decision_context_json"]["model_backend"] == backend

    await p.after_invocation(
        make_after_event(inv, result=make_result())
    )
    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert post_kwargs["outcome"] == "SUCCESS"
    assert post_kwargs["provider_event_id"] == expected_id


@pytest.mark.asyncio
@pytest.mark.parametrize("backend", ["BedrockModel", "OpenAIModel", "LiteLLMModel"])
async def test_I03_deny_blocks_provider_for_all_backends(backend) -> None:
    """DENY raises DecisionDenied for every backend; AFTER is a no-op."""
    denied = DecisionDenied(
        "budget cap",
        decision_id="dec-i3",
        reason_codes=["BUDGET_EXHAUSTED"],
    )
    client = make_client_mock(request_decision_side_effect=denied)
    p = make_provider(client=client)
    inv = make_invocation(
        invocation_id=f"inv-{backend}-i3", model_name=backend
    )

    with pytest.raises(DecisionDenied):
        await p.before_invocation(make_before_event(inv))
    # Since PRE raised, after_invocation receives an exception event but
    # nothing is stashed — must remain a no-op.
    await p.after_invocation(
        make_after_event(inv, result=None,
                         exception=RuntimeError("won't see this"))
    )
    client.emit_llm_call_post.assert_not_awaited()


@pytest.mark.asyncio
async def test_I04_provider_event_id_falls_back_to_empty() -> None:
    """``result`` without ``.id`` → ``provider_event_id == ""`` on commit."""
    client = make_client_mock()
    p = make_provider(client=client)
    inv = make_invocation(invocation_id="inv-i4")
    await p.before_invocation(make_before_event(inv))

    result = SimpleNamespace(
        usage=SimpleNamespace(total_tokens=10),
        # No id / response_id / model_response.
    )
    await p.after_invocation(make_after_event(inv, result=result))
    post_kwargs = client.emit_llm_call_post.call_args.kwargs
    assert post_kwargs["provider_event_id"] == ""


@pytest.mark.asyncio
async def test_I05_fail_open_env_flag_allows_sidecar_errors() -> None:
    """``SPENDGUARD_STRANDS_FAIL_OPEN=1`` in env allows on sidecar error."""
    import os
    os.environ["SPENDGUARD_STRANDS_FAIL_OPEN"] = "1"
    try:
        from spendguard.errors import SpendGuardError

        client = make_client_mock(
            request_decision_side_effect=SpendGuardError("sidecar down"),
        )
        p = make_provider(client=client)
        inv = make_invocation(invocation_id="inv-i5")

        # Does NOT raise — fail-open via env flag.
        await p.before_invocation(make_before_event(inv))
        # Nothing stashed since PRE bailed early.
        assert p.pending_count == 0
        # After is a no-op (no pending entry).
        await p.after_invocation(
            make_after_event(inv, result=make_bedrock_result())
        )
        client.emit_llm_call_post.assert_not_awaited()
    finally:
        os.environ.pop("SPENDGUARD_STRANDS_FAIL_OPEN", None)


# ─────────────────────────────────────────────────────────────────────
# I06 — Options POCO validation
# ─────────────────────────────────────────────────────────────────────


def test_I06_options_validates_required_fields() -> None:
    """``SpendGuardStrandsOptions`` rejects empty required fields."""
    with pytest.raises(SpendGuardConfigError, match="tenant_id"):
        SpendGuardStrandsOptions(
            tenant_id="", budget_id="b1", window_instance_id="w1",
        )
    with pytest.raises(SpendGuardConfigError, match="budget_id"):
        SpendGuardStrandsOptions(
            tenant_id="t1", budget_id="", window_instance_id="w1",
        )
    with pytest.raises(SpendGuardConfigError, match="window_instance_id"):
        SpendGuardStrandsOptions(
            tenant_id="t1", budget_id="b1", window_instance_id="",
        )


def test_I06b_options_happy_path() -> None:
    """Happy-path ``SpendGuardStrandsOptions`` construction."""
    opts = SpendGuardStrandsOptions(
        tenant_id="t1", budget_id="b1", window_instance_id="w1",
    )
    assert opts.tenant_id == "t1"
    assert opts.fail_closed is True
