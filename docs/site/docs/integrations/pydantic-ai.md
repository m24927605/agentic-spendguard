# Pydantic-AI integration

```python
from pydantic_ai import Agent
from pydantic_ai.models.openai import OpenAIModel

from spendguard import SpendGuardClient, new_uuid7
from spendguard.integrations.pydantic_ai import (
    RunContext, SpendGuardModel, run_context,
)
from spendguard._proto.spendguard.common.v1 import common_pb2

client = SpendGuardClient(socket_path=..., tenant_id=...)
await client.connect()
await client.handshake()

guarded = SpendGuardModel(
    inner=OpenAIModel("gpt-4o-mini"),
    client=client,
    budget_id=...,
    window_instance_id=...,
    unit=common_pb2.UnitRef(...),
    pricing=common_pb2.PricingFreeze(...),
    claim_estimator=lambda messages, settings: [common_pb2.BudgetClaim(...)],
)

agent = Agent(model=guarded)
async with run_context(RunContext(run_id=str(new_uuid7()))):
    result = await agent.run("Hello")
```

Install: `pip install 'spendguard-sdk[pydantic-ai]'`
