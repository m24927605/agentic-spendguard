"""SpendGuard deny-conformance harness (in-image adapters) — NO FAKES.

For every adapter whose framework ships in the demo runner image
(pydantic_ai, langchain, openai_agents, agt, litellm) this drives the REAL
framework through its REAL top-level API against a REAL counting HTTP provider
(deploy/demo run_demo `_start_counting_provider`, OpenAI /v1/chat/completions
shape). Each adapter is exercised twice:

  * ALLOW leg  — a small claim; the real framework call reaches the provider,
                 so the counting provider's hit counter increments by 1
                 (positive control: the gate is actually wired in-path).
  * DENY  leg  — a 2,000,000,000-atomic claim busts the demo contract's 1B
                 hard-cap-deny rule; the sidecar returns STOP and the adapter
                 raises DecisionDenied (DecisionStopped) BEFORE the provider
                 call, so the counter is UNCHANGED (live zero-provider-HTTP).

There are NO SimpleNamespace stand-ins, no bare inner models, no TestModel, no
direct private-method calls: every leg runs a genuine framework Agent/chain
(pydantic-ai Agent.run, openai-agents Runner.run, langchain ainvoke, agt
PolicyEvaluator.evaluate, litellm acompletion) so a green run proves the real
integration fail-closes on a busted budget.

SCOPE — the ~11 other adapters pin mutually-incompatible framework versions and
cannot co-exist in one image; each is deny-tested in its own runner overlay
(deploy/demo/agent_real_*/, the *_deny DEMO_MODE targets) against the same
counting-stub standard. This is by design, not a coverage gap.
"""
from __future__ import annotations

from spendguard.errors import DecisionDenied

_DUMMY_KEY = "demo-key-counting-provider"  # never used: a hard-cap DENY short-circuits before the provider
_BUDGET_BUST = "2000000000"  # 2B atomic — twice the demo contract's 1B hard-cap threshold
_ALLOW_AMOUNT = "50"  # well below the 1B cap


def _claim(amount, budget_id, window_instance_id, unit):
    from spendguard._proto.spendguard.common.v1 import common_pb2

    return [
        common_pb2.BudgetClaim(
            budget_id=budget_id,
            unit=unit,
            amount_atomic=amount,
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=window_instance_id,
        )
    ]


class ConformanceError(AssertionError):
    """A real ALLOW/DENY leg did not behave as required."""


# --------------------------------------------------------------------------- #
# pydantic_ai — real Agent.run over a real OpenAIModel pointed at the counter.  #
# --------------------------------------------------------------------------- #
async def drive_deny_pydantic_ai(client, *, budget_id, window_instance_id, unit, pricing, base_url, counting):
    from pydantic_ai import Agent
    from pydantic_ai.models.openai import OpenAIModel
    from pydantic_ai.providers.openai import OpenAIProvider

    from spendguard.integrations.pydantic_ai import (
        RunContext,
        SpendGuardModel,
        run_context,
    )

    def make_agent(amount):
        inner = OpenAIModel(
            "gpt-4o-mini",
            provider=OpenAIProvider(base_url=base_url, api_key=_DUMMY_KEY),
        )
        guard = SpendGuardModel(
            inner=inner,
            client=client,
            budget_id=budget_id,
            window_instance_id=window_instance_id,
            unit=unit,
            pricing=pricing,
            claim_estimator=lambda *_a: _claim(amount, budget_id, window_instance_id, unit),
        )
        return Agent(model=guard)

    pre = counting["calls"]
    async with run_context(RunContext(run_id="deny-conf-pydantic_ai-allow")):
        await make_agent(_ALLOW_AMOUNT).run("hello")
    if counting["calls"] != pre + 1:
        raise ConformanceError(f"ALLOW did not reach provider (calls {pre}->{counting['calls']})")

    pre = counting["calls"]
    denied = False
    async with run_context(RunContext(run_id="deny-conf-pydantic_ai-deny")):
        try:
            await make_agent(_BUDGET_BUST).run("hello")
        except DecisionDenied:
            denied = True
    if not denied:
        raise ConformanceError("DENY did not raise DecisionDenied")
    if counting["calls"] != pre:
        raise ConformanceError(f"DENY reached provider (calls {pre}->{counting['calls']})")


# --------------------------------------------------------------------------- #
# langchain — real ChatOpenAI.ainvoke through SpendGuardChatModel.              #
# --------------------------------------------------------------------------- #
async def drive_deny_langchain(client, *, budget_id, window_instance_id, unit, pricing, base_url, counting):
    from langchain_core.messages import HumanMessage
    from langchain_openai import ChatOpenAI

    from spendguard.integrations.langchain import (
        RunContext,
        SpendGuardChatModel,
        run_context,
    )

    def make_model(amount):
        return SpendGuardChatModel(
            inner=ChatOpenAI(model="gpt-4o-mini", api_key=_DUMMY_KEY, base_url=base_url),
            client=client,
            budget_id=budget_id,
            window_instance_id=window_instance_id,
            unit=unit,
            pricing=pricing,
            claim_estimator=lambda *_a: _claim(amount, budget_id, window_instance_id, unit),
        )

    pre = counting["calls"]
    async with run_context(RunContext(run_id="deny-conf-langchain-allow")):
        await make_model(_ALLOW_AMOUNT).ainvoke([HumanMessage(content="hello")])
    if counting["calls"] != pre + 1:
        raise ConformanceError(f"ALLOW did not reach provider (calls {pre}->{counting['calls']})")

    pre = counting["calls"]
    denied = False
    async with run_context(RunContext(run_id="deny-conf-langchain-deny")):
        try:
            await make_model(_BUDGET_BUST).ainvoke([HumanMessage(content="hello")])
        except DecisionDenied:
            denied = True
    if not denied:
        raise ConformanceError("DENY did not raise DecisionDenied")
    if counting["calls"] != pre:
        raise ConformanceError(f"DENY reached provider (calls {pre}->{counting['calls']})")


# --------------------------------------------------------------------------- #
# openai_agents — real Runner.run(Agent) over a real OpenAIChatCompletionsModel.#
# --------------------------------------------------------------------------- #
async def drive_deny_openai_agents(client, *, budget_id, window_instance_id, unit, pricing, base_url, counting):
    from agents import Agent, Runner
    from agents.models.openai_chatcompletions import OpenAIChatCompletionsModel
    from openai import AsyncOpenAI

    from spendguard.integrations.openai_agents import (
        RunContext,
        SpendGuardAgentsModel,
        run_context,
    )

    # No external trace export (would need a real OpenAI key + network).
    try:
        from agents import set_tracing_disabled

        set_tracing_disabled(True)
    except Exception:  # noqa: BLE001
        pass

    oai_client = AsyncOpenAI(base_url=base_url, api_key=_DUMMY_KEY)

    def make_agent(amount):
        inner = OpenAIChatCompletionsModel(model="gpt-4o-mini", openai_client=oai_client)
        guard = SpendGuardAgentsModel(
            inner=inner,
            client=client,
            budget_id=budget_id,
            window_instance_id=window_instance_id,
            unit=unit,
            pricing=pricing,
            claim_estimator=lambda *_a: _claim(amount, budget_id, window_instance_id, unit),
        )
        return Agent(name="deny-conformance", model=guard)

    pre = counting["calls"]
    async with run_context(RunContext(run_id="deny-conf-openai_agents-allow")):
        await Runner.run(make_agent(_ALLOW_AMOUNT), "hello")
    if counting["calls"] != pre + 1:
        raise ConformanceError(f"ALLOW did not reach provider (calls {pre}->{counting['calls']})")

    pre = counting["calls"]
    denied = False
    async with run_context(RunContext(run_id="deny-conf-openai_agents-deny")):
        try:
            await Runner.run(make_agent(_BUDGET_BUST), "hello")
        except DecisionDenied:
            denied = True
    if not denied:
        raise ConformanceError("DENY did not raise DecisionDenied")
    if counting["calls"] != pre:
        raise ConformanceError(f"DENY reached provider (calls {pre}->{counting['calls']})")


# --------------------------------------------------------------------------- #
# agt — real PolicyEvaluator + SpendGuardCompositeEvaluator.evaluate(). AGT is  #
# a tool-policy + budget gate with NO LLM provider call, so there is no counter #
# to move; the contract is ALLOW (allowed=True) vs SG-DENY (allowed=False).     #
# --------------------------------------------------------------------------- #
async def drive_deny_agt(client, *, budget_id, window_instance_id, unit, pricing, base_url, counting):
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

    def make_composite(amount):
        return SpendGuardCompositeEvaluator(
            agt_evaluator=agt_evaluator,
            spendguard_client=client,
            budget_id=budget_id,
            window_instance_id=window_instance_id,
            unit=unit,
            pricing=pricing,
            claim_estimator=lambda *_a: _claim(amount, budget_id, window_instance_id, unit),
        )

    pre = counting["calls"]
    async with run_context(RunContext(run_id="deny-conf-agt")):
        allow = await make_composite(_ALLOW_AMOUNT).evaluate(
            {"tool_name": "web_search", "tool_call_id": "deny-conf-agt-allow"}
        )
        if not allow.allowed:
            raise ConformanceError(f"ALLOW path denied: {allow.reason!r}")
        deny = await make_composite(_BUDGET_BUST).evaluate(
            {"tool_name": "web_search", "tool_call_id": "deny-conf-agt-deny"}
        )
    if deny.allowed or "SPENDGUARD_DENY" not in (deny.reason or ""):
        raise ConformanceError(f"DENY path not SPENDGUARD_DENY: allowed={deny.allowed} reason={deny.reason!r}")
    if counting["calls"] != pre:
        raise ConformanceError("AGT evaluate must not touch any provider")


# --------------------------------------------------------------------------- #
# litellm — real SpendGuardDirectAcompletion over real litellm.acompletion.     #
# --------------------------------------------------------------------------- #
async def drive_deny_litellm(client, *, budget_id, window_instance_id, unit, pricing, base_url, counting):
    import litellm

    from spendguard.integrations.litellm import (
        BudgetBinding,
        SpendGuardDirectAcompletion,
    )

    litellm.api_base = base_url

    binding = BudgetBinding(
        budget_id=budget_id,
        window_instance_id=window_instance_id,
        unit=unit,
        pricing=pricing,
    )

    def _estimator(ctx):
        override = str((ctx.data or {}).get("spendguard_estimate_override", "") or "").strip()
        amount = override if override.isdigit() else _ALLOW_AMOUNT
        return _claim(amount, budget_id, window_instance_id, unit)

    def _reconciler(ctx, response):
        usage = getattr(response, "usage", None)
        tokens = int(getattr(usage, "completion_tokens", 0) or 0)
        return _claim(str(max(tokens, 1)), budget_id, window_instance_id, unit)

    wrapper = SpendGuardDirectAcompletion(
        client=client,
        budget_resolver=lambda _ctx: binding,
        claim_estimator=_estimator,
        claim_reconciler=_reconciler,
    )

    pre = counting["calls"]
    await wrapper(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "hello"}],
        api_key=_DUMMY_KEY,
    )
    if counting["calls"] != pre + 1:
        raise ConformanceError(f"ALLOW did not reach provider (calls {pre}->{counting['calls']})")

    pre = counting["calls"]
    denied = False
    try:
        await wrapper(
            model="gpt-4o-mini",
            messages=[{"role": "user", "content": "hello"}],
            api_key=_DUMMY_KEY,
            spendguard_estimate_override=_BUDGET_BUST,
        )
    except DecisionDenied:
        denied = True
    if not denied:
        raise ConformanceError("DENY did not raise DecisionDenied")
    if counting["calls"] != pre:
        raise ConformanceError(f"DENY reached provider (calls {pre}->{counting['calls']})")


REGISTRY = {
    "pydantic_ai": drive_deny_pydantic_ai,
    "langchain": drive_deny_langchain,
    "openai_agents": drive_deny_openai_agents,
    "agt": drive_deny_agt,
    "litellm": drive_deny_litellm,
}


async def run_conformance(client, *, budget_id, window_instance_id, unit, pricing, base_url, counting):
    """Drive every in-image adapter through a real ALLOW + DENY against the
    counting provider. Returns (passed, results)."""
    results = []
    for name, drive in REGISTRY.items():
        try:
            await drive(
                client,
                budget_id=budget_id,
                window_instance_id=window_instance_id,
                unit=unit,
                pricing=pricing,
                base_url=base_url,
                counting=counting,
            )
            results.append(
                (name, "PASS", "real ALLOW hit provider; real DENY raised DecisionDenied, provider counter unchanged")
            )
        except ConformanceError as e:
            results.append((name, "FAIL", f"conformance: {e}".splitlines()[0][:160]))
        except Exception as e:  # noqa: BLE001
            results.append((name, "FAIL-SHIM", f"{type(e).__name__}: {e}".splitlines()[0][:160]))
    passed = sum(1 for _, st, _ in results if st == "PASS")
    return passed, results
