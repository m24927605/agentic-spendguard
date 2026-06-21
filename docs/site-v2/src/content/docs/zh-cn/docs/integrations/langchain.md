---
title: "用 SpendGuard 控制 LangChain & LangGraph 的预算"
description: >-
  用 Agentic SpendGuard 给 LangChain 和 LangGraph agent 做调用前的 token 预算把关。
  一个 BaseChatModel wrapper 两个框架通吃 —— 支持 create_react_agent、tool loop、
  with_structured_output、bind_tools。
---


> 你的 LangChain agent 的 tool loop 刚因为 OpenAI 返回了个不稳定的结果,又走进了重试
> 路径。每一次 `ChatOpenAI.invoke()` 都会再往 provider 发一个请求。没有关卡,你只能
> 等到下个月的账单才知道花了多少。SpendGuard 把 `BaseChatModel` 包一层,让每一次
> invocation 都在上游调用发出*之前*先去对预算 reserve —— 而且因为 LangGraph 底层
> 就是 `BaseChatModel`,同一个 wrapper 拿到 LangGraph 里照样能用。

## 为什么你会想要这个

- **一个 wrapper,两个框架。** `SpendGuardChatModel` 是一个 drop-in 的 `BaseChatModel`
  subclass —— 直接丢给 `create_react_agent`、任何 `RunnableSequence`,或自定义的
  LangGraph node,其他什么都不用改。
- **tool call 和 structured output 都保留。** `bind_tools()` 和
  `with_structured_output()` 会 forward 给内层的 model。Pydantic-typed 的 output 和
  function-calling 照常工作。
- **调用前就拒绝,不是事后对账。** 超预算的调用会在 `invoke()` / `ainvoke()` 里面
  raise,所以 chain 在任何一个 token 被花掉之前就停了。
- **审计 + approval pipeline 跟其他框架共用。** 这个 wrapper 写进去的是跟 Pydantic-AI、
  OpenAI Agents 集成同一个 SpendGuard ledger,所以一个多框架的 agent 机队会有一份
  统一的决策 log。

## 安装(60 秒)

```bash
pip install 'spendguard-sdk[langchain,langgraph]'
```

通过 demo stack 把一个 sidecar 拉起来:

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard && make demo-up
```

## 接起来

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

## 你会拿到什么

- 每一次 `ChatOpenAI.invoke()` / `ainvoke()` / streaming 调用都有**调用前的预算
  reservation**。
- **同一个 wrapper 涵盖 LangGraph。** `create_react_agent`、自定义的 `StateGraph`
  node、tool loop 全都流过同一道关卡。
- **structured output 和 tool call 都保留。** wrapper 的 `bind_tools()` 和
  `with_structured_output()` 都是直接 passthrough。

## 常见模式

### 每个 tool 各自的预算粒度

每个 tool 配一个各自的 `budget_id`。`claim_estimator` 会去看进来的 messages,挑出
对的 budget claim —— 一个吃大量 context 的 summarization tool,拿到的 reservation
会跟一个 calculator tool 不一样。

### LangGraph 的 subgraph 和 sub-agent

每个自己 new 一个 model instance 的 subgraph,都需要自己的 `SpendGuardChatModel`
wrapper。所有 subgraph 共用一个 `SpendGuardClient` 就好 —— 这个 client 是
thread-safe 的,而且会把 UDS 连接 pool 起来。

### Streaming 响应

streaming 的 token delta 不影响 reservation;reservation 在 stream 打开
之前就已经做好了。async streaming 用 wrapper 的 `astream()`。

## 相关

- [快速开始](../quickstart.md) —— 5 分钟把整套跑起来
- [Contract DSL reference](../contracts/yaml.md) —— 写出 allow/stop 规则
- 其他集成:[Pydantic-AI](pydantic-ai.md) · [OpenAI Agents SDK](openai-agents.md) · [Microsoft AGT](agt.md)
