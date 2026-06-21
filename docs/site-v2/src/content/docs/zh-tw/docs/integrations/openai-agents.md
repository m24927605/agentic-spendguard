---
title: "用 SpendGuard 控制 OpenAI Agents SDK 的預算"
description: >-
  用 Agentic SpendGuard 給 OpenAI Agents SDK 的 agent 做呼叫前的 token 預算把關。
  Runner.run 裡面每一次 model 呼叫,在 LLM 請求送出之前都會先去對預算 reserve,
  而且附帶一條簽章的稽核軌跡。
---


> OpenAI Agents SDK 會把你的 agent 跑在一個迴圈裡,每一個 step 送出一個 model
> 請求。沒有關卡的話,一個卡住的 agent 可以一直重打同一個 `Runner.run`,直到
> 某個環節 OOM,或者 API 開始對你 rate-limit。SpendGuard 把 agent 的 model 包一層,
> 讓每個 step 都先去對預算 reserve —— 預算用完的時候,就直接拒掉那次呼叫。

## 你為什麼會想要這個

- **step 層級的預算。** `Runner.run()` 裡面每一次 model 呼叫都是一個 reservation
  點。失控的迴圈撞上的是關卡,不是你的 provider 帳單。
- **貼合 SDK 的介面。** `SpendGuardAgentsModel` 的介面跟內層 model 對齊,
  所以 agent 之間的 handoff 跟 tool-use 迴圈照常運作,其他地方都不用動。
- **跟你整套 stack 共用同一個 ledger。** 如果你同時也在跑 Pydantic-AI 或
  LangChain 的 agent,所有決策都會落在同一條稽核鏈上。

## 安裝(60 秒)

```bash
pip install 'spendguard-sdk[openai-agents]'
```

透過 demo stack 把一個 sidecar 帶起來:

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard && make demo-up
```

## 接起來

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

## 你會拿到什麼

- **每個 step 各自 reserve。** 每一次 `Runner.run` 的 step 都會流過被包起來的
  model,並且對你的預算 reserve。
- **稽核鏈。** 跟其他框架整合用的是同一種簽章 ledger 格式。
- **composability 保留。** handoff 跟 tool-use 的語意原封不動地流過 wrapper。

## 常見模式

### 多個 sub-agent

每一個 `Agent(model=...)` 都需要自己的 `SpendGuardAgentsModel` wrapper。所有
sub-agent 共用一個 `SpendGuardClient` 就好 —— 這個 client 是 thread-safe 的,而且
會把 UDS 連線 pool 起來。

### 跟 SpendGuard 並行的 tracing

OpenAI Agents SDK 會發出自己的 trace 事件。SpendGuard 的 `decision_id` 會被標進
model wrapper 的 log 裡,所以跑完之後你可以把 SDK 的 trace 跟 SpendGuard 的稽核
紀錄對應起來。

## 相關

- [快速開始](../quickstart.md) —— 5 分鐘把整套跑起來
- [Contract DSL reference](../contracts/yaml.md) —— 寫出 allow/stop 規則
- 其他整合:[Pydantic-AI](pydantic-ai.md) · [LangChain & LangGraph](langchain.md) · [Microsoft AGT](agt.md)
