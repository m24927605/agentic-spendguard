# OpenAI Agents SDK integration

Scaffold module — POC validation deferred until OpenAI Agents SDK
API surface stabilizes.

```python
from agents import Agent, Runner
from spendguard import SpendGuardClient
from spendguard.integrations.openai_agents import SpendGuardAgentsModel

guarded = SpendGuardAgentsModel(
    inner_model_name="gpt-4o-mini",
    client=client,
    budget_id=...,
    # ...
)

agent = Agent(name="my-agent", instructions="...", model=guarded)
result = await Runner.run(agent, "Hello")
```

Install: `pip install 'spendguard-sdk[openai-agents]'`

See `sdk/python/src/spendguard/integrations/openai_agents.py` source
for the full integration shape; the module ships ready-to-test once
`pip install agent-governance-toolkit` is in your env.
