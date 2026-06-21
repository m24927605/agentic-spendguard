---
title: "LLM API 請求的呼叫前預算上限"
description: >-
  在花費發生之前、而不是之後,對 LLM API 請求設下硬性的 token 預算上限。給 OpenAI、
  Anthropic 以及任何 provider 的 Stripe 式 auth/capture reservation —— 超預算的呼叫
  會在邊界上就被拒絕。
---


> 你想要一個上限,它說「這個 agent 每小時在 gpt-4o 上最多花 $X,任何一通會把它推過
> 這條線的呼叫,都必須*在*請求送到 provider *之前*被拒絕」。Token 用量 dashboard 跟
> 日警報給你的是事後的數字,不是那道關卡。這篇給你的就是那道關卡的做法。

## 為什麼標準答案沒用

大多數 LLM 成本工具做的是**對帳**,不是**控制**:

| 做法 | 它做什麼 | 你什麼時候才知道 |
|---|---|---|
| provider 帳單 / billing API | 告訴你花了多少 | 帳期結束時 |
| 用量 dashboard | 把 token 數加總 | 花完之後好幾小時 |
| provider key 上的 rate limit | 限每秒/每分鐘的請求數 | 不是看金額 —— 是看次數 |
| soft alert(「你到 80% 了」) | ping 一個 webhook | 預算已經花掉大半之後 |

這些沒有一個會*阻止*那通呼叫。它們只是把帳單攤給你看,頂多讓你趕在下一張帳單來之前知道而已。當一個 agent
陷在重試迴圈或 tool-use 迴圈裡時,「把錢花掉」跟「看到 dashboard」之間的那段空檔,
正是真正的傷害發生的時候。

## 真正有用的做法

每一通 LLM 呼叫前面都坐著一個預算 reservation。這個 reservation 的行為就像 Stripe
的 auth/capture:

```
agent → SDK wrapper
          │
          ▼
       sidecar.request_decision(budget_id, projected_claim)
          │
          ├── budget would be exceeded ───► STOP   (raise, no LLM call)
          │
          ├── budget can cover it     ───► RESERVE (auth) ──┐
          │                                                  │
          │                                                  ▼
          │                                          your LLM call goes out
          │                                                  │
          ├── provider response ──────► sidecar.commit (capture actual)
          │                              or sidecar.release (cancel auth)
          │
          └── crash / timeout         ─► reservation auto-releases on TTL
```

有三個特性讓它能成立:

1. **呼叫前的拒絕是機制性的。** 超預算那條路徑是一個被丟出來的 exception,不是一個
   軟性警告。應用程式不可能不小心把它忽略掉。
2. **reservation 是真的入帳記過的一筆,不是估個數而已。** ledger 把 reservation(auth 階段)
   跟 commit(capture 階段)分開記,所以預估 reserve 了 1,500 個 token、實際只用了
   800 個,就會把 700 個釋放回預算。
3. **重試時冪等。** 一次輸入完全相同的重試會收斂到原本那筆 reservation,而不是再配
   一筆新的。否則一個重試 47 次的迴圈會燒掉 47 倍的 reservation。

## 給我看 code

reservation 就是一個呼叫。Agentic SpendGuard SDK 幫你處理掉 auth/commit/release 的
整個生命週期:

```python
from spendguard import SpendGuardClient, DecisionStopped

async with SpendGuardClient(socket_path="/var/run/spendguard/adapter.sock",
                            tenant_id=tenant_id) as sg:
    await sg.handshake()
    try:
        outcome = await sg.request_decision(
            trigger="LLM_CALL_PRE",
            run_id=run_id, decision_id=decision_id,
            route="llm.call",
            projected_claims=[claim],          # 預估的 USD 或 token
            idempotency_key=derive_key(...),   # 跨重試都穩定
        )
        # reservation 做好了。現在去打 LLM 呼叫。
    except DecisionStopped as e:
        # 超預算了。這通 LLM 呼叫絕對不能發生。
        raise
```

framework adapter(Pydantic-AI / LangChain / OpenAI Agents / AGT)會把這整段包進
一個 `Model.request()` 的 override 裡,所以應用程式那邊不用改。

## 延伸閱讀

- [Pydantic-AI 整合](../integrations/pydantic-ai.md) —— drop-in 的 `Model` wrapper,
  幫你處理 auth/capture 生命週期
- [Reservation 模式深入](reservation-pattern.md) —— LLM 花費 auth/capture 背後的
  架構推理
- [擋下失控的 agent](agent-runaway-protection.md) —— 這個模式專門要防的就是那個失效模式
- [Contract DSL reference](../contracts/yaml.md) —— 寫出每通呼叫該 allow / stop /
  require-approval 的規則
