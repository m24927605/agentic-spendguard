---
title: "用 SpendGuard 控制 Pydantic-AI 的預算"
description: >-
  用 Agentic SpendGuard 給 Pydantic-AI 的 agent 做呼叫前的 token 預算把關。
  每一次 Model.request() 在 LLM 呼叫送出之前都會先去對預算 reserve,附帶一條簽章的
  稽核軌跡,以及超預算呼叫的 human-approval 流程。
---


> 你的 Pydantic-AI agent 呼叫 `agent.run("...")`,run 迴圈就會一次又一次地 dispatch
> `Model.request()` —— 每個 step 一次、每次 retry 一次、multi-step 的 tool 迴圈也是每步
> 一次。沒有關卡的話,每一次 iteration 都是一筆沒人擋的 provider 呼叫。SpendGuard 把
> model 包一層,讓每一次 `request()` 都在上游 LLM 呼叫送出*之前*先去對預算 reserve。

## 你為什麼會想要這個

- **呼叫前就把關,不是事後看 dashboard。** reservation 發生在 OpenAI/Anthropic 呼叫
  之前。超預算的呼叫會 raise `DecisionStopped`,上游請求根本不會送出去。
- **冪等,retry 也安全。** Pydantic-AI 在 transient error 時會重新進入 `request()`。
  SpendGuard 會從 messages + settings + run_id 推出一個穩定的 `idempotency_key`,所以
  retry 會對回原本那筆 reservation,而不是再開一筆新的。
- **tool 迴圈一樣在預算內。** multi-step、會用 tool 的 agent,每一次 model 呼叫都被
  把關,包括 tool output 衍生出來的 step。
- **稽核軌跡。** 每一個決策(allow / stop / require_approval / degrade)都會簽章、
  串成鏈,可供事後分析。
- **human-in-the-loop approval。** 當一條 contract 觸發 `REQUIRE_APPROVAL` 時,用
  `await e.resume(client)` 做 pause-and-resume。

## 安裝(60 秒)

```bash
pip install spendguard-sdk
```

Pydantic-AI 的 auto-install 目前暫時 fail-closed,因為 CVE-2026-25580 影響 1.56.0
之前的 `pydantic-ai` / `pydantic-ai-slim`,而 PyPI 目前還沒放出修好的 1.56.0+ 版本。
等上游發了之後,在同一個環境裡裝一個經過查核、沒有漏洞的 `pydantic-ai` 版本。

你還需要一個跑著的 SpendGuard sidecar,透過 Unix Domain Socket 連得到。最快的路徑就是
demo stack:

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard && make demo-up
```

demo 會把 sidecar 的 UDS 綁在 `deploy/demo/runtime/uds/adapter.sock`。

## 接起來

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

## 你會拿到什麼

- **呼叫前的預算 reservation。** 當 reservation 會超過預算時,被包起來的 model 會
  raise `DecisionStopped`,而不是去打 LLM。
- **簽章的稽核鏈。** 每一個決策都帶著一個密碼學簽章記進 ledger;透過 `audit_outbox`
  的交易模式做到 replay-safe。
- **approval 續跑。** 當一條 contract 觸發 `REQUIRE_APPROVAL` 時,exception 帶著
  `e.resume(client)` —— 等操作者在 dashboard 上核准之後,呼叫它就能續跑。

## 常見模式

### 每個 tenant 各自的預算

每個 tenant 傳不同的 `budget_id` / `window_instance_id`。control plane API
(`POST /v1/budgets`)可以在不重啟 agent 的情況下開出新預算。

### 處理 approval

```python
from spendguard import ApprovalRequired

try:
    result = await agent.run(prompt)
except ApprovalRequired as e:
    await wait_for_operator_approval(e.decision_id)
    result = await e.resume(client)
```

### 不燒 token 就能測試

把 `OpenAIModel` 換成 `pydantic_ai.models.test.TestModel`。SpendGuard 的 wrapper 照樣
會記下 reservation 跟決策,所以你不用 provider key 就能對預算邏輯做單元測試。

## 相關

- [快速開始](../quickstart.md) —— 5 分鐘把整套跑起來
- [Contract DSL reference](../contracts/yaml.md) —— 寫出 allow/stop 規則
- 其他整合:[LangChain & LangGraph](langchain.md) · [OpenAI Agents SDK](openai-agents.md) · [Microsoft AGT](agt.md)
