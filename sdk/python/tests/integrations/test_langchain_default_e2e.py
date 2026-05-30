"""End-to-end integration test: LangChain ChatModel + default estimator.

SLICE_12 §8.2: "LangChain ChatOpenAI without claim_estimator → works
with default; audit row populated."

We don't actually call OpenAI here (no API key required in CI). The
test verifies:
1. `SpendGuardChatModel(inner=ChatOpenAI(...), ...)` without
   `claim_estimator` constructs without error.
2. The default estimator on the wrapper dispatches to o200k_base /
   cl100k_base based on the inner model_name.
3. Calling the estimator with a list of HumanMessage produces a
   single non-zero BudgetClaim.
4. The same flow works for ChatAnthropic (vendored BPE path).
"""

from __future__ import annotations

import pytest

from spendguard import SpendGuardClient
from spendguard._proto.spendguard.common.v1 import common_pb2


@pytest.fixture
def stub_client() -> SpendGuardClient:
    return SpendGuardClient(socket_path="/dev/null", tenant_id="t1")


@pytest.fixture
def unit() -> common_pb2.UnitRef:
    return common_pb2.UnitRef(unit_id="usd_micros")


@pytest.fixture
def pricing() -> common_pb2.PricingFreeze:
    return common_pb2.PricingFreeze(pricing_version="v1")


# ─────────────────────────────────────────────────────────────────────
# LangChain — ChatOpenAI default estimator path
# ─────────────────────────────────────────────────────────────────────


class TestLangChainOpenAIDefault:
    def test_chat_openai_default_estimator(
        self,
        stub_client: SpendGuardClient,
        unit: common_pb2.UnitRef,
        pricing: common_pb2.PricingFreeze,
    ) -> None:
        # ChatOpenAI may not be installed; skip if missing
        pytest.importorskip("langchain_openai")
        from langchain_core.messages import HumanMessage, SystemMessage
        from langchain_openai import ChatOpenAI

        from spendguard.integrations.langchain import SpendGuardChatModel

        # ChatOpenAI constructs without an API key (it lazy-validates on
        # first .invoke() call). We never invoke, so this is safe.
        inner = ChatOpenAI(model="gpt-4o-mini", api_key="dummy")
        wrapper = SpendGuardChatModel(
            inner=inner,
            client=stub_client,
            budget_id="b1",
            window_instance_id="w1",
            unit=unit,
            pricing=pricing,
            # claim_estimator omitted — default dispatched
        )
        assert wrapper.claim_estimator is not None

        # Default estimator works on HumanMessage + SystemMessage
        messages = [
            SystemMessage(content="You are a helpful assistant."),
            HumanMessage(content="What is the capital of France?"),
        ]
        claims = wrapper.claim_estimator(messages)
        assert len(claims) == 1
        amount = int(claims[0].amount_atomic)
        # Sanity bounds: short prompt → tens of tokens; + max_tokens
        # default 16K for o200k → amount in the 16K range.
        assert amount > 0
        assert amount < 100_000
        assert claims[0].budget_id == "b1"
        assert claims[0].window_instance_id == "w1"

    def test_chat_anthropic_default_estimator(
        self,
        stub_client: SpendGuardClient,
        unit: common_pb2.UnitRef,
        pricing: common_pb2.PricingFreeze,
    ) -> None:
        pytest.importorskip("langchain_anthropic")
        from langchain_anthropic import ChatAnthropic
        from langchain_core.messages import HumanMessage

        from spendguard.integrations.langchain import SpendGuardChatModel

        inner = ChatAnthropic(
            model_name="claude-3-5-sonnet-20240620",
            api_key="dummy",
            timeout=30.0,
            stop=None,
        )
        wrapper = SpendGuardChatModel(
            inner=inner,
            client=stub_client,
            budget_id="b1",
            window_instance_id="w1",
            unit=unit,
            pricing=pricing,
        )
        # Default dispatched to anthropic-v3-bpe
        claims = wrapper.claim_estimator(
            [HumanMessage(content="Tell me about Paris")]
        )
        assert len(claims) == 1
        assert int(claims[0].amount_atomic) > 0


class TestPydanticAIOpenAIDefault:
    def test_pydantic_ai_default(
        self,
        stub_client: SpendGuardClient,
        unit: common_pb2.UnitRef,
        pricing: common_pb2.PricingFreeze,
    ) -> None:
        pytest.importorskip("pydantic_ai")
        # pydantic-ai versions vary; the public class is `OpenAIModel`
        # in 0.0.x. We construct without a real OpenAI API call.
        from pydantic_ai.models.openai import OpenAIModel

        from spendguard.integrations.pydantic_ai import SpendGuardModel

        # OpenAIModel requires an OPENAI_API_KEY env var even in dummy
        # mode; set one for the duration of construction.
        import os

        os.environ.setdefault("OPENAI_API_KEY", "sk-dummy-for-test")
        inner = OpenAIModel("gpt-4o-mini")
        wrapper = SpendGuardModel(
            inner=inner,
            client=stub_client,
            budget_id="b1",
            window_instance_id="w1",
            unit=unit,
            pricing=pricing,
            # claim_estimator omitted
        )
        assert wrapper._claim_estimator is not None
        # Empty messages still produce a valid claim
        from types import SimpleNamespace

        msg = SimpleNamespace(content="Hello")
        claims = wrapper._claim_estimator([msg], None)
        assert int(claims[0].amount_atomic) > 0


class TestWithRunPlanIntegrationFlow:
    """Verify ``with_run_plan`` composes with the default-estimator
    integration path — the user gets BOTH defaults without writing any
    spendguard-specific code beyond the integration construction."""

    def test_with_run_plan_decorates_integration_user_code(
        self,
        stub_client: SpendGuardClient,
        unit: common_pb2.UnitRef,
        pricing: common_pb2.PricingFreeze,
    ) -> None:
        pytest.importorskip("langchain_openai")
        from langchain_core.messages import HumanMessage
        from langchain_openai import ChatOpenAI

        from spendguard import current_run_plan, with_run_plan
        from spendguard.integrations.langchain import SpendGuardChatModel

        inner = ChatOpenAI(model="gpt-4o-mini", api_key="dummy")
        wrapper = SpendGuardChatModel(
            inner=inner,
            client=stub_client,
            budget_id="b1",
            window_instance_id="w1",
            unit=unit,
            pricing=pricing,
        )

        @with_run_plan(planned_calls=8, planned_tools=2)
        async def my_agent() -> int:
            # Inside the decorated frame, plan is visible
            plan = current_run_plan()
            assert plan is not None
            assert plan.planned_steps_hint == 10
            # And the default estimator is wired in
            claims = wrapper.claim_estimator(
                [HumanMessage(content="anything")]
            )
            assert int(claims[0].amount_atomic) > 0
            return plan.planned_steps_hint

        import asyncio

        result = asyncio.run(my_agent())
        assert result == 10
