---
title: "用 SpendGuard 管 Google ADK 的預算"
description: >-
  用 Agentic SpendGuard 為 Google Agent Development Kit
  (google-adk) 的 agent 做呼叫前的 token 預算控管。一個 callback 同時
  接管 before_model_callback 跟 after_model_callback 兩個 slot,Gemini
  直連、Vertex 上的 Gemini、還有 LiteLlm 包起來的 OpenAI /
  Anthropic 都吃得到。
---


> 你的 Google ADK `LlmAgent` 剛剛在一條難搞的推理鏈上卡進了 tool-call
> 迴圈,每跑一輪 `before_model_callback` 就再對 Gemini 送一筆
> request 出去。少了一道 gate,你大概要到下個月的帳單儀表板才會
> 發現燒了多少。SpendGuard 的做法是把同一顆
> `SpendGuardAdkCallback` 同時插進 `before_model_callback` 跟
> `after_model_callback`,讓每一輪 model turn 在上游呼叫真正送出
> *之前* 就先對 budget 做 reserve——而且同一顆 callback 對
> Vertex Gemini 或 LiteLlm 包起來的 OpenAI / Anthropic 一樣管用。

## 為什麼你會想用這個

- **一顆 callback,兩個 slot。** `SpendGuardAdkCallback` 是同一個
  instance,你把它同時註冊到 `before_model_callback` 跟
  `after_model_callback`。怎麼分流是看 payload 型別——`LlmRequest`
  走 PRE,`LlmResponse` 走 POST。
- **靠形狀辨識多家供應商,不是靠字串比對。** 它對
  `LlmAgent(model="gemini-2.0-flash")`、Vertex 上的 Gemini、還有
  `LlmAgent(model=LiteLlm("openai/gpt-4o-mini"))` 都能用,因為抽用量
  是讀 `usage_metadata` 這個欄位的形狀,不是去 match model 字串。
- **呼叫前就擋,不是事後對帳。** 超出 budget 的呼叫會回一個合成的
  `LlmResponse(error_code="SPENDGUARD_DENY")`,讓 ADK 直接把這一輪
  short-circuit 掉——Gemini API 根本不會被碰到。
- **稽核 + 核准流程跟其他 framework 共用。** 這顆 callback 寫進的是
  跟 LangChain、Pydantic-AI、OpenAI Agents 整合同一份 SpendGuard
  ledger,所以一支跨多 framework 的 agent 機隊會拿到單一的
  decision log。

## 設定(60 秒)

```bash
pip install 'spendguard-sdk[adk]'
```

用 demo stack 把 sidecar 拉起來:

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard && make demo-up
```

## 接起來

```python
import asyncio

from google.adk.agents import LlmAgent
from google.adk.runners import InMemoryRunner

from spendguard import SpendGuardClient
from spendguard.integrations.adk import SpendGuardAdkCallback
from spendguard._proto.spendguard.common.v1 import common_pb2


async def main() -> None:
    client = SpendGuardClient(
        socket_path="/var/run/spendguard/adapter.sock",
        tenant_id="00000000-0000-4000-8000-000000000001",
    )
    await client.connect()
    await client.handshake()

    cb = SpendGuardAdkCallback(
        client=client,
        budget_id="my-budget",
        window_instance_id="my-window",
        unit=common_pb2.UnitRef(
            unit_id="usd_micros",
            token_kind="output_token",
            model_family="gemini-2.0-flash",
        ),
        pricing=common_pb2.PricingFreeze(pricing_version="2025-q4"),
    )

    agent = LlmAgent(
        name="budget-aware-agent",
        model="gemini-2.0-flash",
        instructions="You are a budget-aware assistant.",
        # Same `cb` instance plugged into BOTH slots:
        before_model_callback=cb,
        after_model_callback=cb,
    )

    runner = InMemoryRunner(agent=agent)
    async for event in runner.run_async(
        session_id=client.session_id,
        user_id="alice",
        new_message="Say hello in three words.",
    ):
        print(event)


asyncio.run(main())
```

## 你會拿到什麼

- **呼叫前的 budget reservation**,每一輪 `LlmAgent` 的 model turn 都會做,
  連 tool-loop 迭代的那幾輪也算在內。
- **多供應商覆蓋。** Gemini 直連、Vertex Gemini、LiteLlm
  包裝——全部都是靠 `usage_metadata` 的形狀抽用量。
- **併發安全。** ADK 每次 `Runner.run_async` 呼叫都會建一個全新的
  `CallbackContext`,所以併發的 run 透過 `callback_context.state`
  天生就是互相隔離的。
- **DENY 不會 raise。** callback 走的是文件裡寫好的
  `LlmResponse(error_code="SPENDGUARD_DENY", ...)` short-circuit
  管道,所以使用者自己後面那條 `after_model_callback` chain(如果有的話)
  還是看得到這次的 deny。

## 常見用法

### 自訂 run_id 做跨 framework 關聯

```python
cb = SpendGuardAdkCallback(
    client=client,
    budget_id="...",
    window_instance_id="...",
    unit=common_pb2.UnitRef(...),
    pricing=common_pb2.PricingFreeze(...),
    run_id_fn=lambda ctx: my_parent_trace_id_for(ctx),
)
```

預設是 `ctx.invocation_id`(ADK 每次 `Runner.run_async` 會配一個
UUID)。如果你想讓 run_id 跟某個 LangChain 或 OpenAI Agents 的
parent trace 對得起來,就把它覆寫掉。

### 自訂 claim estimator

```python
def my_estimator(req):
    # Inspect req.contents for image parts, sum tokens for text parts,
    # surcharge image parts at a per-image rate.
    ...
    return [common_pb2.BudgetClaim(...)]

cb = SpendGuardAdkCallback(
    client=client, budget_id="...", window_instance_id="...",
    unit=..., pricing=..., claim_estimator=my_estimator,
)
```

沒給的話,callback 會依 `req.model` 派出預設的 estimator(Gemini
家族 / 透過 LiteLlm 前綴剝除判 OpenAI / 未知 model 則 fallback 到
chars/4,並丟一次性的 warning)。

### DENY 的行為

當 `request_decision` 回 DENY,callback 會:

1. 把 `ctx.state["spendguard.denied"] = True` 設起來。
2. 回一個合成的 `LlmResponse`,帶上
   `error_code="SPENDGUARD_DENY"`,而 `error_message` 裡放的是
   以逗號接起來的 reason code(預設是 `BUDGET_EXHAUSTED`)。
3. ADK 把這一輪終止掉——裡層的 Gemini transport 完全不會被碰。
4. POST 是 no-op(不 commit、不 release——這次 deny 沒有帶任何
   reservation)。

你可以把自己的 `after_model_callback` 接在 SpendGuard 那顆 *後面*,
記下這次 deny,又不會弄丟 short-circuit 的語意。

### Tool callback

Tool callback(`before_tool_callback` / `after_tool_callback`)在
v0.1.x 不在範圍內。花費控管擺在 model 邊界這一層;tool 呼叫本身
不會直接驅動花費。tool 層級的 budget 控管之後再做,目前列為
enhancement 追蹤中。

## 相關

- [Quickstart](../quickstart.md) — 5 分鐘把整個 stack 拉起來
- [Contract DSL reference](../contracts/yaml.md) — 撰寫 allow/stop 規則
- 其他整合:[Pydantic-AI](pydantic-ai.md) · [LangChain & LangGraph](langchain.md) · [OpenAI Agents SDK](openai-agents.md) · [Microsoft AGT](agt.md)
