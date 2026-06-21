---
title: "用 SpendGuard 控制 Pydantic-AI 的预算"
description: >-
  用 Agentic SpendGuard 给 Pydantic-AI 的 agent 做调用前的 token 预算把关。
  每一次 Model.request() 在 LLM 调用发出之前都会先去对预算 reserve,附带一条签名的
  审计轨迹,以及超预算调用的 human-approval 流程。
---


> 你的 Pydantic-AI agent 调用 `agent.run("...")`,run 循环就会一次又一次地 dispatch
> `Model.request()` —— 每个 step 一次、每次 retry 一次、multi-step 的 tool 循环也是每步
> 一次。没有关卡的话,每一次 iteration 都是一次没人拦的 provider 调用。SpendGuard 把
> model 包一层,让每一次 `request()` 都在上游 LLM 调用发出*之前*先去对预算 reserve。

## 为什么你会想要这个

- **调用前就把关,不是事后看 dashboard。** reservation 发生在 OpenAI/Anthropic 调用
  之前。超预算的调用会 raise `DecisionStopped`,上游请求根本不会发出去。
- **幂等,retry 也安全。** Pydantic-AI 在 transient error 时会重新进入 `request()`。
  SpendGuard 会从 messages + settings + run_id 推出一个稳定的 `idempotency_key`,所以
  retry 会对回原本那笔 reservation,而不是再分一笔新的。
- **tool 循环一样在预算内。** multi-step、会用 tool 的 agent,每一次 model 调用都被
  把关,包括 tool output 衍生出来的 step。
- **审计轨迹。** 每一个决策(allow / stop / require_approval / degrade)都被签名、
  串成链,可供事后分析。
- **human-in-the-loop approval。** 当一条 contract 触发 `REQUIRE_APPROVAL` 时,用
  `await e.resume(client)` 做 pause-and-resume。

## 安装(60 秒)

```bash
pip install spendguard-sdk
```

Pydantic-AI 的 auto-install 目前 fail-closed,因为 CVE-2026-25580 影响 1.56.0
之前的 `pydantic-ai` / `pydantic-ai-slim`,而 PyPI 目前还没放出修好的 1.56.0+ 版本。
等上游发了之后,在同一个环境里装一个经过核查、没有漏洞的 `pydantic-ai` 版本。

你还需要一个跑起来、能通过 Unix Domain Socket 访问到的 SpendGuard sidecar。最快的方式就是
demo stack:

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard && make demo-up
```

demo 会把 sidecar 的 UDS 绑在 `deploy/demo/runtime/uds/adapter.sock`。

## 接起来

```python
import asyncio

from pydantic_ai import Agent
from pydantic_ai.models.openai import OpenAIModel

from spendguard import SpendGuardClient, new_uuid7
from spendguard.integrations.pydantic_ai import (
    RunContext, SpendGuardModel, run_context,
)
from spendguard._proto.spendguard.common.v1 import common_pb2


async def main() -> None:
    client = SpendGuardClient(
        socket_path="/var/run/spendguard/adapter.sock",
        tenant_id="00000000-0000-4000-8000-000000000001",
    )
    await client.connect()
    await client.handshake()

    guarded = SpendGuardModel(
        inner=OpenAIModel("gpt-4o-mini"),
        client=client,
        budget_id="my-budget",
        window_instance_id="my-window",
        unit=common_pb2.UnitRef(
            unit_id="usd_micros",
            token_kind="usd_micros",
            model_family="gpt-4",
        ),
        pricing=common_pb2.PricingFreeze(pricing_version="2025-q4"),
        claim_estimator=lambda messages, settings: [
            common_pb2.BudgetClaim(
                budget_id="my-budget",
                window_instance_id="my-window",
                amount_micros=1_000_000,  # 1 USD reservation per call
            )
        ],
    )

    agent = Agent(model=guarded)
    async with run_context(RunContext(run_id=str(new_uuid7()))):
        result = await agent.run("Hello")
        print(result.output)


asyncio.run(main())
```

## 你会拿到什么

- **调用前的预算 reservation。** 当 reservation 会超过预算时,被包起来的 model 会
  raise `DecisionStopped`,而不是去打 LLM。
- **签名的审计链。** 每一个决策都带着一个密码学签名记进 ledger;通过 `audit_outbox`
  的事务模式做到 replay-safe。
- **approval 续跑。** 当一条 contract 触发 `REQUIRE_APPROVAL` 时,exception 带着
  `e.resume(client)` —— 等操作者在 dashboard 上批准之后,调用它就能续跑。

## 常见模式

### 每个 tenant 各自的预算

每个 tenant 传不同的 `budget_id` / `window_instance_id`。control plane API
(`POST /v1/budgets`)可以在不重启 agent 的情况下开出新预算。

### 处理 approval

```python
from spendguard import ApprovalRequired

try:
    result = await agent.run(prompt)
except ApprovalRequired as e:
    await wait_for_operator_approval(e.decision_id)
    result = await e.resume(client)
```

### 不烧 token 就能测试

把 `OpenAIModel` 换成 `pydantic_ai.models.test.TestModel`。SpendGuard 的 wrapper 照样
会记下 reservation 和决策,所以你不用 provider key 就能对预算逻辑做单元测试。

## 相关

- [快速开始](../quickstart.md) —— 5 分钟把整套跑起来
- [Contract DSL reference](../contracts/yaml.md) —— 写出 allow/stop 规则
- 其他集成:[LangChain & LangGraph](langchain.md) · [OpenAI Agents SDK](openai-agents.md) · [Microsoft AGT](agt.md)
