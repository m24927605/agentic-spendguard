---
description: >-
  Pre-call token budget enforcement for OpenAI Agents SDK agents using
  SpendGuard. Every model call inside Runner.run is reserved against a budget
  before the LLM request is sent, with signed audit trail.
---

# OpenAI Agents SDK budget control with SpendGuard

> The OpenAI Agents SDK runs your agent in a loop and ships a model
> request per step. Without a gate, a stuck agent can re-invoke the
> same `Runner.run` until something OOMs or the API rate-limits you.
> SpendGuard wraps the agent's model so every step reserves against a
> budget — and refuses the call when the budget is exhausted.

## Why you'd want this

- **Step-level budgeting.** Every model call inside `Runner.run()` is
  a reservation point. A runaway loop hits the gate, not your provider
  invoice.
- **SDK-native shape.** `SpendGuardAgentsModel` mirrors the inner
  model's surface area, so agent-to-agent handoff and tool-use loops
  continue to work without changes elsewhere.
- **Same ledger as the rest of your stack.** If you also run
  Pydantic-AI or LangChain agents, all decisions land in the same
  audit chain.

## Setup (60 seconds)

```bash
pip install 'spendguard-sdk[openai-agents]'
```

Sidecar via demo stack:

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard && make demo-up
```

## Wire it up

```python
import asyncio

from agents import Agent, Runner

from spendguard import SpendGuardClient, new_uuid7
from spendguard.integrations.openai_agents import SpendGuardAgentsModel
from spendguard._proto.spendguard.common.v1 import common_pb2


async def main() -> None:
    client = SpendGuardClient(
        socket_path="/var/run/spendguard/adapter.sock",
        tenant_id="00000000-0000-4000-8000-000000000001",
    )
    await client.connect()
    await client.handshake()

    guarded = SpendGuardAgentsModel(
        inner_model_name="gpt-4o-mini",
        client=client,
        budget_id="my-budget",
        window_instance_id="my-window",
        unit=common_pb2.UnitRef(
            unit_id="usd_micros",
            token_kind="usd_micros",
            model_family="gpt-4",
        ),
        pricing=common_pb2.PricingFreeze(pricing_version="2025-q4"),
        claim_estimator=lambda messages: [
            common_pb2.BudgetClaim(
                budget_id="my-budget",
                window_instance_id="my-window",
                amount_micros=1_000_000,
            )
        ],
    )

    agent = Agent(
        name="my-agent",
        instructions="Be terse.",
        model=guarded,
    )
    result = await Runner.run(agent, "Hello")
    print(result.output)


asyncio.run(main())
```

## What you get

- **Per-step reservation.** Every `Runner.run` step calls through the
  wrapped model and reserves against your budget.
- **Audit chain.** Same signed ledger format as the other framework
  integrations.
- **Composability preserved.** Handoff and tool-use semantics flow
  through the wrapper unchanged.

## Common patterns

### Multiple sub-agents

Each `Agent(model=...)` needs its own `SpendGuardAgentsModel`
wrapper. Share one `SpendGuardClient` across sub-agents — the client
is thread-safe and pools the UDS connection.

### Tracing alongside SpendGuard

The OpenAI Agents SDK emits its own trace events. SpendGuard's
`decision_id` is tagged into the model wrapper's log so you can
correlate the SDK trace with the SpendGuard audit row after the run.

## Related

- [Quickstart](../quickstart.md) — full stack up in 5 minutes
- [Contract DSL reference](../contracts/yaml.md) — author allow/stop rules
- Other integrations: [Pydantic-AI](pydantic-ai.md) · [LangChain & LangGraph](langchain.md) · [Microsoft AGT](agt.md)
