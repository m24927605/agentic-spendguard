---
description: >-
  Pre-call token budget enforcement for LangChain and LangGraph agents using
  Agentic SpendGuard. One BaseChatModel wrapper covers both — works with
  create_react_agent, tool loops, with_structured_output, and bind_tools.
---

# LangChain & LangGraph budget control with SpendGuard

> Your LangChain agent's tool loop just hit the retry path on a flaky
> OpenAI response. Each `ChatOpenAI.invoke()` ships another request to
> the provider. Without a gate, you find out the cost on next month's
> invoice. SpendGuard wraps the `BaseChatModel` so every invocation
> reserves against a budget *before* the upstream call goes out — and
> the same wrapper works for LangGraph since LangGraph builds on
> `BaseChatModel`.

## Why you'd want this

- **One wrapper, both frameworks.** `SpendGuardChatModel` is a drop-in
  `BaseChatModel` subclass — pass it to `create_react_agent`, any
  `RunnableSequence`, or a custom LangGraph node without other changes.
- **Tool calls and structured output preserved.** `bind_tools()` and
  `with_structured_output()` forward to the inner model. Pydantic-typed
  outputs and function-calling continue to work.
- **Pre-call refusal, not post-hoc accounting.** Over-budget calls
  raise inside `invoke()` / `ainvoke()` so the chain halts before any
  token is spent.
- **Audit + approval pipeline shared with every other framework.** The
  wrapper writes to the same SpendGuard ledger as the Pydantic-AI and
  OpenAI-Agents integrations, so a multi-framework agent fleet gets a
  single decision log.

## Setup (60 seconds)

```bash
pip install 'spendguard-sdk[langchain,langgraph]'
```

Bring up a sidecar via the demo stack:

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard && make demo-up
```

## Wire it up

```python
import asyncio

from langchain_core.messages import HumanMessage
from langchain_openai import ChatOpenAI
from langgraph.prebuilt import create_react_agent

from spendguard import SpendGuardClient, new_uuid7
from spendguard.integrations.langchain import (
    RunContext, SpendGuardChatModel, run_context,
)
from spendguard._proto.spendguard.common.v1 import common_pb2


async def main() -> None:
    client = SpendGuardClient(
        socket_path="/var/run/spendguard/adapter.sock",
        tenant_id="00000000-0000-4000-8000-000000000001",
    )
    await client.connect()
    await client.handshake()

    guarded = SpendGuardChatModel(
        inner=ChatOpenAI(model="gpt-4o-mini"),
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

    # LangGraph: works because SpendGuardChatModel forwards bind_tools()
    agent = create_react_agent(guarded, tools=[my_tool])

    async with run_context(RunContext(run_id=str(new_uuid7()))):
        result = await agent.ainvoke({
            "messages": [HumanMessage(content="Hello")]
        })
        print(result["messages"][-1].content)


asyncio.run(main())
```

## What you get

- **Pre-call budget reservation** on every `ChatOpenAI.invoke()` /
  `ainvoke()` / streaming call.
- **Same wrapper covers LangGraph.** `create_react_agent`, custom
  `StateGraph` nodes, and tool-loops all flow through one gate.
- **Structured output and tool calls preserved.** Wrapper's
  `bind_tools()` and `with_structured_output()` are passthroughs.

## Common patterns

### Per-tool budget granularity

Configure separate `budget_id`s per tool. The `claim_estimator`
inspects the inbound messages and picks the right budget claim —
a big-context summarization tool gets a different reservation than
a calculator tool.

### LangGraph subgraphs and sub-agents

Each subgraph that constructs its own model instance needs its own
`SpendGuardChatModel` wrapper. Share one `SpendGuardClient` across
all of them — the client is thread-safe and pools the UDS
connection.

### Streaming responses

Streamed token deltas don't change the reservation flow; the
reservation is made before the stream opens. Use the wrapper's
`astream()` for async streaming.

## Related

- [Quickstart](../quickstart.md) — full stack up in 5 minutes
- [Contract DSL reference](../contracts/yaml.md) — author allow/stop rules
- Other integrations: [Pydantic-AI](pydantic-ai.md) · [OpenAI Agents SDK](openai-agents.md) · [Microsoft AGT](agt.md)
