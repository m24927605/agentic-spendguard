---
title: "用 SpendGuard 控制 OpenAI Agents SDK 的预算"
description: >-
  用 Agentic SpendGuard 给 OpenAI Agents SDK 的 agent 做调用前的 token 预算把关。
  Runner.run 里面每一次 model 调用,在 LLM 请求发出之前都会先去对预算 reserve,
  而且附带一条签名的审计轨迹。
---


> OpenAI Agents SDK 会把你的 agent 跑在一个循环里,每一个 step 发出一个 model
> 请求。没有关卡的话,一个卡住的 agent 可以一直重打同一个 `Runner.run`,直到
> 某个环节 OOM,或者 API 开始对你 rate-limit。SpendGuard 把 agent 的 model 包一层,
> 让每个 step 都先去对预算 reserve —— 预算用完的时候,就直接拒掉那次调用。

## 为什么你会想要这个

- **step 层级的预算。** `Runner.run()` 里面每一次 model 调用都是一个 reservation
  点。失控的循环撞上的是关卡,不是你的 provider 账单。
- **贴合 SDK 的接口。** `SpendGuardAgentsModel` 的接口跟内层 model 对齐,
  所以 agent 之间的 handoff 和 tool-use 循环照常工作,其他地方都不用动。
- **跟你整套 stack 共用同一个 ledger。** 如果你同时也在跑 Pydantic-AI 或
  LangChain 的 agent,所有决策都会落在同一条审计链上。

## 安装(60 秒)

```bash
pip install 'spendguard-sdk[openai-agents]'
```

通过 demo stack 把一个 sidecar 拉起来:

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard && make demo-up
```

## 接起来

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

## 你会拿到什么

- **每个 step 各自 reserve。** 每一次 `Runner.run` 的 step 都会流过被包起来的
  model,并且对你的预算 reserve。
- **审计链。** 跟其他框架集成用的是同一种签名 ledger 格式。
- **composability 保留。** handoff 和 tool-use 的语义原封不动地流过 wrapper。

## 常见模式

### 多个 sub-agent

每一个 `Agent(model=...)` 都需要自己的 `SpendGuardAgentsModel` wrapper。所有
sub-agent 共用一个 `SpendGuardClient` 就好 —— 这个 client 是 thread-safe 的,而且
会把 UDS 连接 pool 起来。

### 跟 SpendGuard 并行的 tracing

OpenAI Agents SDK 会发出自己的 trace 事件。SpendGuard 的 `decision_id` 会被标进
model wrapper 的 log 里,所以跑完之后你可以把 SDK 的 trace 跟 SpendGuard 的审计
行对应起来。

## 相关

- [快速开始](../quickstart.md) —— 5 分钟把整套跑起来
- [Contract DSL reference](../contracts/yaml.md) —— 写出 allow/stop 规则
- 其他集成:[Pydantic-AI](pydantic-ai.md) · [LangChain & LangGraph](langchain.md) · [Microsoft AGT](agt.md)
