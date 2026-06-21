---
title: "用 SpendGuard 控制 LangChain & LangGraph 的預算"
description: >-
  用 Agentic SpendGuard 給 LangChain 和 LangGraph agent 做呼叫前的 token 預算把關。
  一個 BaseChatModel wrapper 兩個框架通吃 —— 支援 create_react_agent、tool loop、
  with_structured_output、bind_tools。
---


> 你的 LangChain agent 的 tool loop 剛剛因為 OpenAI 回了個不穩定的結果而走進重試
> 路徑。每一次 `ChatOpenAI.invoke()` 都再送一個請求給 provider。沒有關卡的話,你要
> 等到下個月的帳單才知道花了多少。SpendGuard 把 `BaseChatModel` 包一層,讓每一次
> invocation 都在上游呼叫送出*之前*先去對預算 reserve —— 而且因為 LangGraph 是
> 建在 `BaseChatModel` 上的,同一個 wrapper 對 LangGraph 也通用。

## 你為什麼會想要這個

- **一個 wrapper,兩個框架。** `SpendGuardChatModel` 是一個 drop-in 的 `BaseChatModel`
  subclass —— 直接丟給 `create_react_agent`、任何 `RunnableSequence`,或自訂的
  LangGraph node,其他什麼都不用改。
- **tool call 跟 structured output 都保留。** `bind_tools()` 跟
  `with_structured_output()` 會 forward 給內層的 model。Pydantic-typed 的 output 跟
  function-calling 照常運作。
- **呼叫前就拒絕,不是事後對帳。** 超預算的呼叫會在 `invoke()` / `ainvoke()` 裡面
  raise,所以 chain 在任何一個 token 被花掉之前就停了。
- **稽核 + approval pipeline 跟其他框架共用。** 這個 wrapper 寫進去的是跟 Pydantic-AI、
  OpenAI Agents 整合同一個 SpendGuard ledger,所以一個多框架的 agent 機隊會有一份
  統一的決策 log。

## 安裝(60 秒)

```bash
pip install 'spendguard-sdk[langchain,langgraph]'
```

透過 demo stack 把一個 sidecar 帶起來:

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard && make demo-up
```

## 接起來

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

## 你會拿到什麼

- 每一次 `ChatOpenAI.invoke()` / `ainvoke()` / streaming 呼叫都有**呼叫前的預算
  reservation**。
- **同一個 wrapper 涵蓋 LangGraph。** `create_react_agent`、自訂的 `StateGraph`
  node、tool loop 全都流過同一道關卡。
- **structured output 跟 tool call 都保留。** wrapper 的 `bind_tools()` 跟
  `with_structured_output()` 都是直接 passthrough。

## 常見模式

### 每個 tool 各自的預算粒度

每個 tool 配一個各自的 `budget_id`。`claim_estimator` 會去看進來的 messages,挑出
對的 budget claim —— 一個吃大量 context 的 summarization tool,拿到的 reservation
會跟一個 calculator tool 不一樣。

### LangGraph 的 subgraph 跟 sub-agent

每個自己 new 一個 model instance 的 subgraph,都需要自己的 `SpendGuardChatModel`
wrapper。所有 subgraph 共用一個 `SpendGuardClient` 就好 —— 這個 client 是
thread-safe 的,而且會把 UDS 連線 pool 起來。

### Streaming 回應

streaming 的 token delta 不會改變 reservation 的流程;reservation 是在 stream 打開
之前就做好的。async streaming 用 wrapper 的 `astream()`。

## 相關

- [快速開始](../quickstart.md) —— 5 分鐘把整套跑起來
- [Contract DSL reference](../contracts/yaml.md) —— 寫出 allow/stop 規則
- 其他整合:[Pydantic-AI](pydantic-ai.md) · [OpenAI Agents SDK](openai-agents.md) · [Microsoft AGT](agt.md)
