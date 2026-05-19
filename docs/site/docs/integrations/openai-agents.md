---
description: >-
  Pre-call token budget enforcement for OpenAI Agents SDK agents using
  Agentic SpendGuard. Every model call inside Runner.run is reserved against a budget
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
from agents.models.openai_chatcompletions import OpenAIChatCompletionsModel
from openai import AsyncOpenAI

from spendguard import SpendGuardClient, new_uuid7
from spendguard.integrations.openai_agents import (
    RunContext,
    SpendGuardAgentsModel,
    run_context,
)
from spendguard._proto.spendguard.common.v1 import common_pb2


async def main() -> None:
    client = SpendGuardClient(
        socket_path="/var/run/spendguard/adapter.sock",
        tenant_id="00000000-0000-4000-8000-000000000001",
    )
    await client.connect()
    await client.handshake()

    unit = common_pb2.UnitRef(
        unit_id="66666666-6666-4666-8666-666666666666",
        token_kind="output_token",
        model_family="gpt-4",
    )
    pricing = common_pb2.PricingFreeze(pricing_version="demo-pricing-v1")

    inner_model = OpenAIChatCompletionsModel(
        model="gpt-4o-mini",
        openai_client=AsyncOpenAI(),
    )
    guarded = SpendGuardAgentsModel(
        inner=inner_model,
        client=client,
        budget_id="44444444-4444-4444-8444-444444444444",
        window_instance_id="55555555-5555-4555-8555-555555555555",
        unit=unit,
        pricing=pricing,
        claim_estimator=lambda _input: [
            common_pb2.BudgetClaim(
                budget_id="44444444-4444-4444-8444-444444444444",
                unit=unit,
                amount_atomic="500",
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id="55555555-5555-4555-8555-555555555555",
            )
        ],
    )

    agent = Agent(
        name="my-agent",
        instructions="Be terse.",
        model=guarded,
    )
    run_id = str(new_uuid7())
    async with run_context(RunContext(run_id=run_id)):
        result = await Runner.run(agent, "Hello")
    print(result.final_output)


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

## Try it without the SDK

Want to see the wrapper invariant without installing anything? The
[runnable example](https://github.com/m24927605/agentic-spendguard/tree/main/examples/openai-agents-composite)
has a `--mock` mode that uses zero non-stdlib dependencies and proves the
core invariant — **SpendGuard DENY ⇒ the inner Model is never invoked** —
in under five seconds:

```bash
python examples/openai-agents-composite/openai_agents_composite_demo.py --mock
```

## Related

- [Runnable example: `examples/openai-agents-composite/`](https://github.com/m24927605/agentic-spendguard/tree/main/examples/openai-agents-composite) — mock + real modes
- [Quickstart](../quickstart.md) — full stack up in 5 minutes
- [Contract DSL reference](../contracts/yaml.md) — author allow/stop rules
- Other integrations: [Pydantic-AI](pydantic-ai.md) · [LangChain & LangGraph](langchain.md) · [Microsoft AGT](agt.md)
