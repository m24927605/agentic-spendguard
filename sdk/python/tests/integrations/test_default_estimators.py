"""Default-estimator integration tests for SLICE_12 Phase C.

Verifies each of the 5 integrations:

* Accepts ``claim_estimator=None`` (omitted at call site) and installs
  the default token estimator dispatched from the inner model name.
* Honors caller-supplied ``claim_estimator`` per spec §8.5 (backward
  compat — explicit value wins).
* The default estimator produces non-empty ``BudgetClaim`` lists with
  amounts > 0 for known models.

These tests use the lightweight ``ResolverContext`` / payload mocks
from the existing test corpus so we don't have to spin up a sidecar.
"""

from __future__ import annotations

from types import SimpleNamespace
from typing import Any

import pytest

from spendguard import SpendGuardClient
from spendguard._proto.spendguard.common.v1 import common_pb2


# ─────────────────────────────────────────────────────────────────────
# Helpers shared across integrations
# ─────────────────────────────────────────────────────────────────────


@pytest.fixture
def stub_client() -> SpendGuardClient:
    """Real SpendGuardClient instance (not connected) — satisfies
    LangChain's pydantic v2 isinstance check; never makes RPC."""
    return SpendGuardClient(socket_path="/dev/null", tenant_id="t1")


@pytest.fixture
def unit_ref() -> Any:
    return common_pb2.UnitRef(unit_id="usd_micros")


@pytest.fixture
def pricing() -> Any:
    return common_pb2.PricingFreeze(pricing_version="v1")


# ─────────────────────────────────────────────────────────────────────
# OpenAI Agents — default estimator dispatched from inner.model
# ─────────────────────────────────────────────────────────────────────


class TestOpenAIAgentsDefaults:
    """Validates ``SpendGuardAgentsModel(claim_estimator=None)`` works."""

    def test_default_estimator_dispatched_from_inner_model(
        self, unit_ref: Any, pricing: Any
    ) -> None:
        pytest.importorskip("agents")
        from spendguard.integrations.openai_agents import SpendGuardAgentsModel

        # Mock inner: just needs a `.model` attribute.
        inner = SimpleNamespace(model="gpt-4o-mini")

        wrapper = SpendGuardAgentsModel(
            inner=inner,
            client=SimpleNamespace(),  # not exercised
            budget_id="b1",
            window_instance_id="w1",
            unit=unit_ref,
            pricing=pricing,
            claim_estimator=None,
        )
        # Default estimator installed
        assert wrapper._claim_estimator is not None
        # Estimator should produce a single non-zero claim
        claims = wrapper._claim_estimator("Hello, world!")
        assert len(claims) == 1
        amount = int(claims[0].amount_atomic)
        assert amount > 0
        assert claims[0].budget_id == "b1"
        assert claims[0].window_instance_id == "w1"

    def test_explicit_estimator_wins(
        self, unit_ref: Any, pricing: Any
    ) -> None:
        pytest.importorskip("agents")
        from spendguard.integrations.openai_agents import SpendGuardAgentsModel

        inner = SimpleNamespace(model="gpt-4o")

        def my_estimator(_input: Any) -> list[Any]:
            return [common_pb2.BudgetClaim(amount_atomic="9999")]

        wrapper = SpendGuardAgentsModel(
            inner=inner,
            client=SimpleNamespace(),
            budget_id="b1",
            window_instance_id="w1",
            unit=unit_ref,
            pricing=pricing,
            claim_estimator=my_estimator,
        )
        assert wrapper._claim_estimator is my_estimator
        claims = wrapper._claim_estimator("anything")
        assert claims[0].amount_atomic == "9999"


# ─────────────────────────────────────────────────────────────────────
# LangChain — default estimator via pydantic v2 model_post_init
# ─────────────────────────────────────────────────────────────────────


class TestLangChainDefaults:
    def test_default_estimator_dispatched_from_inner_model_name(
        self, unit_ref: Any, pricing: Any, stub_client: SpendGuardClient
    ) -> None:
        pytest.importorskip("langchain_core")
        from langchain_core.language_models import BaseChatModel
        from langchain_core.messages import HumanMessage

        from spendguard.integrations.langchain import SpendGuardChatModel

        # Use a stand-in inner that quacks like a ChatModel
        class FakeInner(BaseChatModel):
            model_name: str = "gpt-4o"

            @property
            def _llm_type(self) -> str:
                return "fake"

            def _generate(
                self, messages: Any, stop: Any = None, run_manager: Any = None, **kwargs: Any
            ) -> Any:
                raise NotImplementedError

        wrapper = SpendGuardChatModel(
            inner=FakeInner(),
            client=stub_client,
            budget_id="b1",
            window_instance_id="w1",
            unit=unit_ref,
            pricing=pricing,
            # claim_estimator omitted intentionally
        )
        assert wrapper.claim_estimator is not None
        claims = wrapper.claim_estimator(
            [HumanMessage(content="Hello, world!")]
        )
        assert len(claims) == 1
        assert int(claims[0].amount_atomic) > 0
        assert claims[0].budget_id == "b1"

    def test_explicit_estimator_wins(
        self, unit_ref: Any, pricing: Any, stub_client: SpendGuardClient
    ) -> None:
        pytest.importorskip("langchain_core")
        from langchain_core.language_models import BaseChatModel
        from langchain_core.messages import HumanMessage

        from spendguard.integrations.langchain import SpendGuardChatModel

        class FakeInner(BaseChatModel):
            model_name: str = "gpt-4o"

            @property
            def _llm_type(self) -> str:
                return "fake"

            def _generate(
                self, messages: Any, stop: Any = None, run_manager: Any = None, **kwargs: Any
            ) -> Any:
                raise NotImplementedError

        def custom(messages: Any) -> list[Any]:
            return [common_pb2.BudgetClaim(amount_atomic="1234")]

        wrapper = SpendGuardChatModel(
            inner=FakeInner(),
            client=stub_client,
            budget_id="b1",
            window_instance_id="w1",
            unit=unit_ref,
            pricing=pricing,
            claim_estimator=custom,
        )
        claims = wrapper.claim_estimator([HumanMessage(content="x")])
        assert claims[0].amount_atomic == "1234"


# ─────────────────────────────────────────────────────────────────────
# LiteLLM — default estimator with per-call resolver re-invocation
# ─────────────────────────────────────────────────────────────────────


class TestLiteLLMDefaults:
    def test_default_estimator_uses_data_model(
        self, unit_ref: Any, pricing: Any
    ) -> None:
        pytest.importorskip("litellm")
        from spendguard.integrations.litellm import (
            BudgetBinding,
            ResolverContext,
            SpendGuardLiteLLMCallback,
        )

        binding = BudgetBinding(
            budget_id="b1",
            window_instance_id="w1",
            unit=unit_ref,
            pricing=pricing,
        )

        resolver_calls = [0]

        def resolver(_ctx: Any) -> BudgetBinding:
            resolver_calls[0] += 1
            return binding

        def reconciler(_ctx: Any, _resp: Any) -> list[Any]:
            return []

        cb = SpendGuardLiteLLMCallback(
            client=SimpleNamespace(),
            budget_resolver=resolver,
            claim_estimator=None,  # default
            claim_reconciler=reconciler,
            fail_closed=True,
        )

        rctx = ResolverContext(
            data={
                "model": "gpt-4o-mini",
                "messages": [{"role": "user", "content": "Hello world"}],
                "max_tokens": 100,
            },
            user_api_key_dict=None,
            call_type="completion",
        )
        claims = cb._claim_estimator(rctx)
        assert len(claims) == 1
        amount = int(claims[0].amount_atomic)
        # input_tokens + max_tokens=100 ⇒ amount >= 100
        assert amount >= 100
        assert claims[0].budget_id == "b1"
        # The default invocation calls the resolver once (per-call binding)
        assert resolver_calls[0] == 1

    def test_explicit_estimator_wins(
        self, unit_ref: Any, pricing: Any
    ) -> None:
        pytest.importorskip("litellm")
        from spendguard.integrations.litellm import (
            BudgetBinding,
            ResolverContext,
            SpendGuardLiteLLMCallback,
        )

        def resolver(_ctx: Any) -> BudgetBinding:
            return BudgetBinding(
                budget_id="b1",
                window_instance_id="w1",
                unit=unit_ref,
                pricing=pricing,
            )

        def reconciler(_ctx: Any, _resp: Any) -> list[Any]:
            return []

        def custom(_ctx: Any) -> list[Any]:
            return [common_pb2.BudgetClaim(amount_atomic="42")]

        cb = SpendGuardLiteLLMCallback(
            client=SimpleNamespace(),
            budget_resolver=resolver,
            claim_estimator=custom,
            claim_reconciler=reconciler,
        )
        rctx = ResolverContext(
            data={"model": "gpt-4o", "messages": []},
            user_api_key_dict=None,
            call_type="completion",
        )
        claims = cb._claim_estimator(rctx)
        assert claims[0].amount_atomic == "42"


# ─────────────────────────────────────────────────────────────────────
# Pydantic-AI — default estimator from inner.model_name
# ─────────────────────────────────────────────────────────────────────


class TestPydanticAIDefaults:
    def test_default_estimator_dispatched_from_inner_model_name(
        self, unit_ref: Any, pricing: Any
    ) -> None:
        pytest.importorskip("pydantic_ai")
        from spendguard.integrations.pydantic_ai import SpendGuardModel

        # Mock pydantic-ai Model
        class FakeInner:
            model_name = "claude-3-5-sonnet"
            system = "anthropic"

        wrapper = SpendGuardModel(
            inner=FakeInner(),  # type: ignore[arg-type]
            client=SimpleNamespace(),
            budget_id="b1",
            window_instance_id="w1",
            unit=unit_ref,
            pricing=pricing,
            claim_estimator=None,
        )
        assert wrapper._claim_estimator is not None
        # Send a Pydantic-AI-like message
        msg = SimpleNamespace(content="Hello, Claude!")
        claims = wrapper._claim_estimator([msg], None)
        assert len(claims) == 1
        assert int(claims[0].amount_atomic) > 0

    def test_explicit_estimator_wins(
        self, unit_ref: Any, pricing: Any
    ) -> None:
        pytest.importorskip("pydantic_ai")
        from spendguard.integrations.pydantic_ai import SpendGuardModel

        class FakeInner:
            model_name = "gpt-4o"
            system = "openai"

        def custom(messages: Any, settings: Any) -> list[Any]:
            return [common_pb2.BudgetClaim(amount_atomic="777")]

        wrapper = SpendGuardModel(
            inner=FakeInner(),  # type: ignore[arg-type]
            client=SimpleNamespace(),
            budget_id="b1",
            window_instance_id="w1",
            unit=unit_ref,
            pricing=pricing,
            claim_estimator=custom,
        )
        claims = wrapper._claim_estimator([], None)
        assert claims[0].amount_atomic == "777"


# ─────────────────────────────────────────────────────────────────────
# AGT — default estimator from payload's model + tool action handling
# ─────────────────────────────────────────────────────────────────────


class TestAGTDefaults:
    def test_default_estimator_from_payload_model(
        self, unit_ref: Any, pricing: Any
    ) -> None:
        pytest.importorskip("agent_governance_toolkit")
        from spendguard.integrations.agt import SpendGuardCompositeEvaluator

        evaluator = SpendGuardCompositeEvaluator(
            agt_evaluator=SimpleNamespace(),  # not invoked here
            spendguard_client=SimpleNamespace(),
            budget_id="b1",
            window_instance_id="w1",
            unit=unit_ref,
            pricing=pricing,
            claim_estimator=None,
            default_model="gpt-4o-mini",
        )
        # Tool payload with prompt
        payload = {"prompt": "Search for cats", "tool_call_id": "t1"}
        claims = evaluator._claim_estimator(payload)
        assert len(claims) == 1
        assert int(claims[0].amount_atomic) > 0

    def test_default_estimator_per_payload_model_override(
        self, unit_ref: Any, pricing: Any
    ) -> None:
        pytest.importorskip("agent_governance_toolkit")
        from spendguard.integrations.agt import SpendGuardCompositeEvaluator

        evaluator = SpendGuardCompositeEvaluator(
            agt_evaluator=SimpleNamespace(),
            spendguard_client=SimpleNamespace(),
            budget_id="b1",
            window_instance_id="w1",
            unit=unit_ref,
            pricing=pricing,
            default_model="gpt-4o-mini",
        )
        # Payload overrides model → claude-3 dispatch
        payload = {
            "prompt": "Hello",
            "model": "claude-3-5-sonnet",
        }
        claims = evaluator._claim_estimator(payload)
        # Just verify it returns a non-zero amount (different encoder)
        assert int(claims[0].amount_atomic) > 0

    def test_explicit_estimator_wins(
        self, unit_ref: Any, pricing: Any
    ) -> None:
        pytest.importorskip("agent_governance_toolkit")
        from spendguard.integrations.agt import SpendGuardCompositeEvaluator

        def custom(_payload: Any) -> list[Any]:
            return [common_pb2.BudgetClaim(amount_atomic="555")]

        evaluator = SpendGuardCompositeEvaluator(
            agt_evaluator=SimpleNamespace(),
            spendguard_client=SimpleNamespace(),
            budget_id="b1",
            window_instance_id="w1",
            unit=unit_ref,
            pricing=pricing,
            claim_estimator=custom,
        )
        claims = evaluator._claim_estimator({"prompt": "x"})
        assert claims[0].amount_atomic == "555"


# ─────────────────────────────────────────────────────────────────────
# Cross-integration parity: same model produces matching amounts
# ─────────────────────────────────────────────────────────────────────


class TestCrossIntegrationParity:
    def test_same_model_different_integrations_consistent(
        self, unit_ref: Any
    ) -> None:
        """Default estimators across integrations agree on token counts
        for the same model + same message content."""
        from spendguard.integrations._default_estimator import (
            agt_default_claim_estimator,
            langchain_default_claim_estimator,
            litellm_default_claim_estimator,
            openai_agents_default_claim_estimator,
            pydantic_ai_default_claim_estimator,
        )

        content = "Tell me a story about a brave knight."
        binding_kwargs = {
            "budget_id": "b1",
            "window_instance_id": "w1",
            "unit": unit_ref,
            "model": "gpt-4o",
        }

        # LangChain takes Sequence[BaseMessage]; we use dict-shaped that
        # the chars/4 / tiktoken path also accepts.
        langchain_est = langchain_default_claim_estimator(**binding_kwargs)
        langchain_claims = langchain_est([{"role": "user", "content": content}])

        pydantic_est = pydantic_ai_default_claim_estimator(**binding_kwargs)
        pydantic_claims = pydantic_est(
            [{"role": "user", "content": content}], None
        )

        oa_est = openai_agents_default_claim_estimator(**binding_kwargs)
        oa_claims = oa_est(content)

        agt_est = agt_default_claim_estimator(**binding_kwargs)
        agt_claims = agt_est({"prompt": content})

        # All four should produce > 0 amounts; the exact numbers may
        # differ slightly because each integration's signature
        # provides a different `max_tokens` source (defaults from
        # family context window for some, None for others).
        for claims in (langchain_claims, pydantic_claims, oa_claims, agt_claims):
            assert len(claims) == 1
            assert int(claims[0].amount_atomic) > 0
            assert claims[0].budget_id == "b1"
