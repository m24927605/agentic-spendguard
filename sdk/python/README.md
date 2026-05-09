# spendguard-sdk

Runtime safety layer client for AI agent frameworks. Talks to the
SpendGuard sidecar over Unix-domain-socket gRPC; gates each LLM /
tool-call boundary through a Contract DSL evaluator and an atomic
budget ledger with immutable audit chain.

## Install

```bash
# Core only (raw client, no framework integration)
pip install spendguard-sdk

# With the integration you need
pip install 'spendguard-sdk[pydantic-ai]'
pip install 'spendguard-sdk[langchain]'
pip install 'spendguard-sdk[langgraph]'
pip install 'spendguard-sdk[openai-agents]'
```

## Quickstart (Pydantic-AI)

```python
import asyncio
from pydantic_ai import Agent
from pydantic_ai.models.openai import OpenAIModel

from spendguard import SpendGuardClient, new_uuid7
from spendguard.integrations.pydantic_ai import (
    RunContext,
    SpendGuardModel,
    run_context,
)
from spendguard._proto.spendguard.common.v1 import common_pb2


async def main():
    client = SpendGuardClient(
        socket_path="/var/run/spendguard/adapter.sock",
        tenant_id="00000000-0000-4000-8000-000000000001",
    )
    await client.connect()
    await client.handshake()

    guarded = SpendGuardModel(
        inner=OpenAIModel("gpt-4o-mini"),
        client=client,
        budget_id="44444444-4444-4444-8444-444444444444",
        window_instance_id="55555555-5555-4555-8555-555555555555",
        unit=common_pb2.UnitRef(
            unit_id="66666666-6666-4666-8666-666666666666",
            token_kind="output_token",
            model_family="gpt-4",
        ),
        pricing=common_pb2.PricingFreeze(
            pricing_version="demo-pricing-v1",
            price_snapshot_hash=b"<32 bytes>",
            fx_rate_version="demo-fx-v1",
            unit_conversion_version="demo-units-v1",
        ),
        claim_estimator=lambda messages, settings: [
            common_pb2.BudgetClaim(
                budget_id="44444444-4444-4444-8444-444444444444",
                unit=common_pb2.UnitRef(
                    unit_id="66666666-6666-4666-8666-666666666666",
                    token_kind="output_token",
                    model_family="gpt-4",
                ),
                amount_atomic="500",
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id="55555555-5555-4555-8555-555555555555",
            )
        ],
    )

    agent = Agent(model=guarded)
    async with run_context(RunContext(run_id=str(new_uuid7()))):
        result = await agent.run("Say hello in three words.")
    print(result.output)


asyncio.run(main())
```

If a contract rule denies the call, `agent.run(...)` raises
`spendguard.DecisionStopped` carrying `reason_codes` and
`matched_rule_ids`.

## API surface (core)

| Symbol | Purpose |
|---|---|
| `SpendGuardClient` | UDS gRPC client to the sidecar |
| `DecisionStopped`, `DecisionSkipped`, `ApprovalRequired` | per-decision exceptions |
| `derive_idempotency_key(...)` | deterministic key from (tenant, run, step, llm_call, trigger) |
| `new_uuid7()` | UUID v7 helper |

## Wire-protocol compatibility

This SDK pins to a specific protobuf wire version. Check the
sidecar's published version against `spendguard.__version__`; minor
versions are wire-compatible, major bumps are breaking. (Not yet
enforced at handshake; planned for v0.2.)

## License

Apache-2.0.
