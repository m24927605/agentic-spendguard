# ruff: noqa: ANN001, ANN201, ANN202, ANN401, S101
"""COV_D22 — Default estimator factory tests."""

from __future__ import annotations

from types import SimpleNamespace
from unittest.mock import patch

from spendguard._proto.spendguard.common.v1 import common_pb2
from spendguard.integrations._default_estimator import (
    agno_default_claim_estimator,
)


def _unit():
    return common_pb2.UnitRef(unit_id="u1", model_family="gpt-4")


def test_DE1_str_input_builds_one_debit_claim() -> None:
    """Case #1: str input → single DEBIT claim with input + output."""
    est = agno_default_claim_estimator(
        budget_id="b1",
        window_instance_id="w1",
        unit=_unit(),
        model="gpt-4o-mini",
    )
    agent = SimpleNamespace(model=SimpleNamespace(id="gpt-4o-mini"))
    claims = est(agent, "the quick brown fox")
    assert len(claims) == 1
    assert int(claims[0].amount_atomic) > 0
    assert claims[0].direction == common_pb2.BudgetClaim.DEBIT
    assert claims[0].budget_id == "b1"
    assert claims[0].window_instance_id == "w1"


def test_DE2_list_input_matches_str_magnitude() -> None:
    """Case #2: list input → comparable to str input."""
    est = agno_default_claim_estimator(
        budget_id="b1",
        window_instance_id="w1",
        unit=_unit(),
        model="gpt-4o-mini",
    )
    agent = SimpleNamespace(model=SimpleNamespace(id="gpt-4o-mini"))
    claims = est(
        agent, [{"role": "user", "content": "the quick brown fox"}]
    )
    assert len(claims) == 1
    assert int(claims[0].amount_atomic) > 0


def test_DE3_arbitrary_object_is_str_coerced() -> None:
    """Case #3: arbitrary object → str-coerced, no exception."""
    est = agno_default_claim_estimator(
        budget_id="b1",
        window_instance_id="w1",
        unit=_unit(),
        model="gpt-4o-mini",
    )
    agent = SimpleNamespace(model=SimpleNamespace(id="gpt-4o-mini"))

    class Custom:
        def __str__(self) -> str:
            return "custom payload here"

    claims = est(agent, Custom())
    assert len(claims) == 1
    assert int(claims[0].amount_atomic) > 0


def test_DE4_resolves_agent_model_id_at_call_time() -> None:
    """Case #4: agent.model.id used per call, overriding constructor model."""
    with patch(
        "spendguard.integrations._default_estimator.estimator_for_model",
        wraps=__import__(
            "spendguard.estimators", fromlist=["estimator_for_model"]
        ).estimator_for_model,
    ) as spy:
        est = agno_default_claim_estimator(
            budget_id="b1",
            window_instance_id="w1",
            unit=_unit(),
            model="",  # blank constructor model
        )
        agent = SimpleNamespace(model=SimpleNamespace(id="gpt-4o-mini"))
        est(agent, "hi")
        # spy invoked with the agent.model.id
        assert any(call.args[0] == "gpt-4o-mini" for call in spy.call_args_list)


def test_DE5_falls_back_to_constructor_model_when_agent_lacks_model() -> None:
    """Case #5: agent with no .model → constructor model used."""
    with patch(
        "spendguard.integrations._default_estimator.estimator_for_model",
        wraps=__import__(
            "spendguard.estimators", fromlist=["estimator_for_model"]
        ).estimator_for_model,
    ) as spy:
        est = agno_default_claim_estimator(
            budget_id="b1",
            window_instance_id="w1",
            unit=_unit(),
            model="claude-3-5-sonnet",
        )
        raw_agent = SimpleNamespace()
        est(raw_agent, "hi")
        assert any(
            call.args[0] == "claude-3-5-sonnet"
            for call in spy.call_args_list
        )
