---
title: "在失控的 AI agent 把帳單砸到你之前先擋下它"
description: >-
  當一個 AI agent 把同一通失敗的 LLM 呼叫重試了幾百次,帳單會在六小時後才入帳。
  這篇教你怎麼把每一次重試都拿去對預算把關,讓失控迴圈在機制上根本不可能發生 ——
  而不是事後才監控到。
---


> 凌晨 3 點。你的 LangChain agent 因為某個下游工具一直回傳壞掉的結果,把同一通
> `gpt-4o` 呼叫重試了 47 次。每一次重試都送去 OpenAI,每一次重試都被 provider
> 收費。等到 on-call 輪值在早上 6 點注意到警報時,這個 agent 已經燒掉 $400、卻
> 一個有用的輸出都沒產出。Token 用量的 dashboard 有抓到 —— 晚了六小時。

## 為什麼標準答案沒用

直覺是加監控:追蹤每個 agent 的花費,日預算到 80% 就發警報,到 100% 就 call
on-call。這是標準劇本。但在失控這種情況下,它有三個地方會破:

1. **監控是事後的。** agent 在警報 call 出去之前就已經把錢花掉了。警報只是告訴你
   壞事已經發生了。
2. **每通呼叫的成本無法預測。** 一通被重試的呼叫,隨著 context window 變大,可能
   吃掉 1,000 個 token,也可能吃掉 100,000 個。你沒辦法從日預算反推出一個每通呼叫
   的上限。
3. **重試迴圈不甩 rate limit。** provider 的 rate limit 限的是請求數,不是金額。
   一個每 token 貴 10 倍的 model(gpt-4o vs gpt-4o-mini),在同一個 rate limit 下
   就燒掉 10 倍的錢。

對的關卡不是「用量看起來怪怪的就發警報」。對的關卡是「預算 cover 不了就拒絕這通
呼叫」。而且這件事必須放在請求路徑上、在上游呼叫之前、每一次都做。

## 真正有用的做法

每一通 LLM 呼叫都會經過一個持有 auth/capture ledger 的 sidecar。在請求路徑上:

1. SDK wrapper 算出一個預估的 claim(根據輸入長度 + model 定價,估出 USD 或 token
   成本)。
2. sidecar 拿這個預估 claim 去對預算的 reservation 餘額。
3. **如果預算 cover 不了:** wrapper 丟出 `DecisionStopped`。這通 LLM 呼叫根本不會
   送出去。應用程式那邊看到的就是一個 Python exception,照常往上傳。
4. **如果預算 cover 得了:** sidecar 把金額 reserve 起來然後回傳。wrapper 再去打
   上游呼叫。

有兩個特性讓它對「失控迴圈」這種情況特別有韌性:

- **冪等的 reservation。** 一次輸入完全相同的重試(同樣的 messages + 同樣的 model
  設定 + 同樣的 run/step ID)會收斂到原本那筆 reservation 上。一個重試 47 次的迴圈
  只分配一筆 reservation,不是 47 筆。
- **crash 時自動 release。** 如果 wrapper 或 pod 在 reserve 之後、完成上游呼叫之前
  掛掉,那筆 reservation 會在一個可設定的 TTL(預設 600 秒)之後過期。預算不會被
  永久鎖住。

整體效果:一個失控迴圈打出第一次重試、拿到 reservation,後續的重試都撞上關卡
(冪等收斂),然後卡在它本來就在失敗的那件事上 —— 而不是卡在把錢花爆。

## 給我看 code

LangChain 的 wrapper 讓這道關卡對 agent 來說是透明的:

```python
from langchain_openai import ChatOpenAI
from langgraph.prebuilt import create_react_agent

from spendguard import SpendGuardClient
from spendguard.integrations.langchain import (
    RunContext, SpendGuardChatModel, run_context,
)

client = SpendGuardClient(socket_path="...", tenant_id="...")
await client.connect()
await client.handshake()

guarded = SpendGuardChatModel(
    inner=ChatOpenAI(model="gpt-4o"),
    client=client,
    budget_id="my-budget",            # 你要強制執行的上限
    window_instance_id="hourly-2026",
    # ... pricing + claim_estimator 設定
)

agent = create_react_agent(guarded, tools=[my_tool])

async with run_context(RunContext(run_id=str(new_uuid7()))):
    await agent.ainvoke({"messages": [HumanMessage(content="...")]})
```

當預算用完時,agent 下一次 `invoke()` 會丟出 `DecisionStopped`,而不是去打
OpenAI。重試迴圈會收掉,不再燒更多錢。

## 延伸閱讀

- [LangChain & LangGraph 整合](../integrations/langchain.md) —— 上面用到的 wrapper
- [呼叫前預算上限](pre-call-budget-cap.md) —— reservation 模式的細節
- [快速開始](../quickstart.md) —— 5 分鐘把整套跑起來,含一個會走到這條路徑的 DENY demo
- [Contract DSL reference](../contracts/yaml.md) —— 寫出決定哪些呼叫會觸發關卡的規則
