"""Minimal end-to-end example: pydantic-ai Agent with SpendGuard gating.

Run with a sidecar listening on the configured UDS:

    SPENDGUARD_SIDECAR_UDS=/run/spendguard.sock \
    SPENDGUARD_TENANT_ID=11111111-1111-1111-1111-111111111111 \
    SPENDGUARD_BUDGET_ID=22222222-2222-2222-2222-222222222222 \
    SPENDGUARD_WINDOW_INSTANCE_ID=33333333-3333-3333-3333-333333333333 \
    OPENAI_API_KEY=sk-... \
    python examples/basic_agent.py
"""

from __future__ import annotations

import asyncio
import os
import uuid
from collections.abc import Sequence

from pydantic_ai import Agent
from pydantic_ai.models.openai import OpenAIModel

from spendguard_pydantic_ai import (
    RunContext,
    SpendGuardClient,
    SpendGuardModel,
    new_uuid7,
    run_context,
)
from spendguard_pydantic_ai._proto.spendguard.common.v1 import common_pb2


def _env(name: str) -> str:
    val = os.environ.get(name)
    if not val:
        raise RuntimeError(f"env var {name} is required")
    return val


def estimate_claims(
    messages: Sequence[object],
    model_settings: object | None,
) -> list[common_pb2.BudgetClaim]:
    """Naive token estimator: 1 claim per call, 500 tokens projected.

    Production code should use a real tokenizer (tiktoken /
    anthropic-tokenizer) to count input tokens, then add the
    `model_settings.max_tokens` ceiling for output.
    """
    return [
        common_pb2.BudgetClaim(
            budget_id=_env("SPENDGUARD_BUDGET_ID"),
            unit=common_pb2.UnitRef(
                unit_id=_env("SPENDGUARD_UNIT_ID"),
                token_kind="output_token",
                model_family="gpt-4",
            ),
            amount_atomic="500",
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=_env("SPENDGUARD_WINDOW_INSTANCE_ID"),
        ),
    ]


async def main() -> None:
    socket_path = _env("SPENDGUARD_SIDECAR_UDS")
    tenant_id = _env("SPENDGUARD_TENANT_ID")
    budget_id = _env("SPENDGUARD_BUDGET_ID")
    window_id = _env("SPENDGUARD_WINDOW_INSTANCE_ID")

    inner = OpenAIModel("gpt-4o-mini")

    async with SpendGuardClient(
        socket_path=socket_path,
        tenant_id=tenant_id,
    ) as client:
        await client.handshake()

        guarded = SpendGuardModel(
            inner=inner,
            client=client,
            budget_id=budget_id,
            window_instance_id=window_id,
            unit=common_pb2.UnitRef(
                unit_id=_env("SPENDGUARD_UNIT_ID"),
                token_kind="output_token",
                model_family="gpt-4",
            ),
            pricing=common_pb2.PricingFreeze(
                pricing_version=_env("SPENDGUARD_PRICING_VERSION"),
                price_snapshot_hash=bytes.fromhex(
                    _env("SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX")
                ),
                fx_rate_version=_env("SPENDGUARD_FX_RATE_VERSION"),
                unit_conversion_version=_env("SPENDGUARD_UNIT_CONVERSION_VERSION"),
            ),
            claim_estimator=estimate_claims,
        )
        agent = Agent(model=guarded)

        run_id = str(new_uuid7())
        async with run_context(RunContext(run_id=run_id)):
            result = await agent.run("Say hello in three words.")
            print(f"agent output: {result.output}")
            print(f"run_id: {run_id}")


if __name__ == "__main__":
    asyncio.run(main())
