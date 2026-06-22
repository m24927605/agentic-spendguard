"""SpendGuard deny-conformance harness (in-image adapters).

For every adapter whose framework is installed in the demo runner image, drive
its REAL public gating entry once with a budget-busting 2,000,000,000-atomic
claim. The sidecar's `hard-cap-deny` rule (claim > 1B) returns STOP, and the
adapter MUST raise a ``DecisionDenied`` (``DecisionStopped`` is a subclass)
BEFORE any provider call. No provider keys are needed: a hard-cap DENY
short-circuits the Reserve and never reaches the inner model.

SCOPE — this single container can only cover adapters whose framework ships in
``runtime/Dockerfile.adapter``: pydantic_ai, langchain, openai_agents, agt,
litellm. The other ~11 adapters (strands, adk, agno, beeai, letta, llamaindex,
smolagents, atomic_agents, autogen, dspy, agent_framework) pin mutually-
incompatible framework versions, so they CANNOT co-exist in one image — each is
deny-tested inside its own runner overlay (see deploy/demo/agent_real_*/ and the
``*_deny`` DEMO_MODE targets). This is by design, not a coverage gap.

Each shim is graded PASS (a DecisionDenied propagated), FAIL-NO-DENY (the gate
did not fire), or FAIL-SHIM (a wiring error). The shims construct the REAL
framework object (e.g. langchain ChatOpenAI, pydantic-ai TestModel) and call the
REAL public entry (ainvoke / request / get_response / evaluate / __call__) — not
SimpleNamespace fakes — so a green run proves the adapter's own gating path
fail-closes on a busted budget.
"""
from __future__ import annotations

from spendguard.errors import DecisionDenied, DecisionStopped

# 2B atomic units — twice the demo contract's 1B hard-cap-deny threshold.
_BUDGET_BUST = "2000000000"


def _huge_claim(budget_id, window_instance_id, unit):
    from spendguard._proto.spendguard.common.v1 import common_pb2

    return [
        common_pb2.BudgetClaim(
            budget_id=budget_id,
            unit=unit,
            amount_atomic=_BUDGET_BUST,
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=window_instance_id,
        )
    ]


# --------------------------------------------------------------------------- #
# pydantic_ai — SpendGuardModel.request() over a real pydantic-ai TestModel.    #
# --------------------------------------------------------------------------- #
async def drive_deny_pydantic_ai(client, *, budget_id, window_instance_id, unit, pricing):
    from pydantic_ai.messages import ModelRequest, UserPromptPart
    from pydantic_ai.models import ModelRequestParameters
    from pydantic_ai.models.test import TestModel

    from spendguard.integrations.pydantic_ai import (
        RunContext,
        SpendGuardModel,
        run_context,
    )

    guarded = SpendGuardModel(
        inner=TestModel(),
        client=client,
        budget_id=budget_id,
        window_instance_id=window_instance_id,
        unit=unit,
        pricing=pricing,
        claim_estimator=lambda *_a: _huge_claim(budget_id, window_instance_id, unit),
    )
    messages = [ModelRequest(parts=[UserPromptPart(content="deny conformance probe")])]
    params = ModelRequestParameters(function_tools=[], allow_text_result=True, result_tools=[])
    async with run_context(RunContext(run_id="deny-conf-pydantic_ai")):
        await guarded.request(messages, None, params)


# --------------------------------------------------------------------------- #
# langchain — SpendGuardChatModel.ainvoke() over a real ChatOpenAI inner.       #
# (The DENY fires before the inner provider call, so the key is never used.)    #
# --------------------------------------------------------------------------- #
async def drive_deny_langchain(client, *, budget_id, window_instance_id, unit, pricing):
    from langchain_core.messages import HumanMessage
    from langchain_openai import ChatOpenAI

    from spendguard.integrations.langchain import (
        RunContext,
        SpendGuardChatModel,
        run_context,
    )

    # A hard-cap DENY short-circuits before any OpenAI call, so the key is
    # never used — but langchain-openai requires *some* credential at
    # construction (and the container may export OPENAI_API_KEY="", which
    # os.environ.setdefault would not override). Pass a dummy explicitly.
    guarded = SpendGuardChatModel(
        inner=ChatOpenAI(model="gpt-4o-mini", api_key="sk-deny-conformance-no-provider-call"),
        client=client,
        budget_id=budget_id,
        window_instance_id=window_instance_id,
        unit=unit,
        pricing=pricing,
        claim_estimator=lambda *_a: _huge_claim(budget_id, window_instance_id, unit),
    )
    async with run_context(RunContext(run_id="deny-conf-langchain")):
        await guarded.ainvoke([HumanMessage(content="deny conformance probe")])


# --------------------------------------------------------------------------- #
# openai_agents — SpendGuardAgentsModel.get_response() over a bare inner Model.  #
# get_response() gates via request_decision BEFORE the inner model is touched.  #
# --------------------------------------------------------------------------- #
async def drive_deny_openai_agents(client, *, budget_id, window_instance_id, unit, pricing):
    from spendguard.integrations.openai_agents import (
        RunContext,
        SpendGuardAgentsModel,
        run_context,
    )

    class _BareInner:  # agents.Model is an ABC; never reached on a DENY.
        model = "gpt-4o-mini"

    guard = SpendGuardAgentsModel(
        inner=_BareInner(),
        client=client,
        budget_id=budget_id,
        window_instance_id=window_instance_id,
        unit=unit,
        pricing=pricing,
        claim_estimator=lambda *_a: _huge_claim(budget_id, window_instance_id, unit),
    )
    async with run_context(RunContext(run_id="deny-conf-openai_agents")):
        await guard.get_response(None, "deny conformance probe", None, [], None, None, None)


# --------------------------------------------------------------------------- #
# agt — SpendGuardCompositeEvaluator.evaluate() returns CompositeResult; a DENY #
# surfaces as allowed=False / reason~SPENDGUARD_DENY rather than an exception,  #
# so translate it into the uniform DecisionDenied the harness contract expects. #
# --------------------------------------------------------------------------- #
async def drive_deny_agt(client, *, budget_id, window_instance_id, unit, pricing):
    from agent_os.policies import (
        PolicyAction,
        PolicyDefaults,
        PolicyDocument,
        PolicyEvaluator,
    )

    from spendguard.integrations.agt import (
        RunContext,
        SpendGuardCompositeEvaluator,
        run_context,
    )

    agt_evaluator = PolicyEvaluator(
        policies=[
            PolicyDocument(
                name="deny-conformance",
                version="1.0",
                defaults=PolicyDefaults(action=PolicyAction.ALLOW),
                rules=[],
            )
        ]
    )
    composite = SpendGuardCompositeEvaluator(
        agt_evaluator=agt_evaluator,
        spendguard_client=client,
        budget_id=budget_id,
        window_instance_id=window_instance_id,
        unit=unit,
        pricing=pricing,
        claim_estimator=lambda *_a: _huge_claim(budget_id, window_instance_id, unit),
    )
    async with run_context(RunContext(run_id="deny-conf-agt")):
        result = await composite.evaluate(
            {"tool_name": "web_search", "tool_call_id": "deny-conf-agt"}
        )
    if not result.allowed and "SPENDGUARD_DENY" in (result.reason or ""):
        raise DecisionStopped(
            f"agt composite denied: {result.reason}",
            decision_id=getattr(result, "decision_id", "") or "",
            reason_codes=["BUDGET_EXHAUSTED"],
            matched_rule_ids=list(getattr(result, "matched_rule_ids", []) or []),
        )


# --------------------------------------------------------------------------- #
# litellm — SpendGuardDirectAcompletion.__call__() with an estimate override    #
# that forces a 2B claim; DENY raises before litellm.acompletion is invoked.    #
# --------------------------------------------------------------------------- #
async def drive_deny_litellm(client, *, budget_id, window_instance_id, unit, pricing):
    from spendguard.integrations.litellm import (
        BudgetBinding,
        SpendGuardDirectAcompletion,
    )

    binding = BudgetBinding(
        budget_id=budget_id,
        window_instance_id=window_instance_id,
        unit=unit,
        pricing=pricing,
    )
    wrapper = SpendGuardDirectAcompletion(
        client=client,
        budget_resolver=lambda _ctx: binding,
        claim_estimator=lambda _ctx: _huge_claim(budget_id, window_instance_id, unit),
        claim_reconciler=lambda _ctx, _resp: _huge_claim(budget_id, window_instance_id, unit),
    )
    await wrapper(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "deny conformance probe"}],
        spendguard_estimate_override=_BUDGET_BUST,
    )


REGISTRY = {
    "pydantic_ai": drive_deny_pydantic_ai,
    "langchain": drive_deny_langchain,
    "openai_agents": drive_deny_openai_agents,
    "agt": drive_deny_agt,
    "litellm": drive_deny_litellm,
}


async def run_conformance(client, *, budget_id, window_instance_id, unit, pricing):
    """Drive every in-image adapter through a DENY. Returns (passed, results)."""
    results = []
    for name, drive in REGISTRY.items():
        try:
            await drive(
                client,
                budget_id=budget_id,
                window_instance_id=window_instance_id,
                unit=unit,
                pricing=pricing,
            )
            results.append(
                (name, "FAIL-NO-DENY", "no exception raised — budget gate did NOT fire")
            )
        except DecisionDenied as e:
            results.append(
                (name, "PASS", f"DecisionDenied raised before provider call ({type(e).__name__})")
            )
        except Exception as e:  # noqa: BLE001
            results.append((name, "FAIL-SHIM", f"{type(e).__name__}: {e}".splitlines()[0][:160]))
    passed = sum(1 for _, st, _ in results if st == "PASS")
    return passed, results
