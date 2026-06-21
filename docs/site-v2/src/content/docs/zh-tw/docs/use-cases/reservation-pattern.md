---
title: "LLM 預算的 reservation 模式"
description: >-
  LLM 花費的 reservation 模式。一個 Stripe 式的 auth/capture ledger,把 runtime 的
  預算把關變成一個 primitive。看呼叫前的關卡、預估的 reservation,以及呼叫後的
  capture/release 階段,是怎麼組成一條成本正確的稽核軌跡。
---


> auth/capture 這個模式在支付領域已經四十年了,在 feature flag 領域也有幾年了。它
> 還沒成為 LLM 花費的一個 primitive,但它應該要是 —— 同樣的形狀解的是同一類問題。這篇從
> 第一原理講這個模式,並展示 Agentic SpendGuard 是怎麼實作它的。

## 為什麼標準答案沒用

「控制 LLM 花費」最直覺的設計是一個 counter:

```
budget_remaining = 1000.00
on each LLM call:
    budget_remaining -= actual_cost   # arrives after the call
    if budget_remaining < 0: alert
```

這會破,因為**實際成本是在呼叫之後才到的**。provider 回傳一筆 usage 紀錄,告訴你它
收了多少 token 的錢。等你把 counter 減下去時,錢早就花掉了。這個 counter 是個記帳
工具,不是一道關卡。

要把關,你得在呼叫*之前*就知道成本。但 LLM 的成本不是固定的 —— output token 取決於
model 生成了什麼,而那又取決於……你還沒打的那通呼叫。

這正是 auth/capture 在支付領域解的問題。當你入住時,Visa 網路並不知道一筆飯店帳的
最終金額 —— 飯店可能再加酒水、room service、房間毀損費。所以飯店先**授權(auth-hold)**一個
預估金額,把資金 reserve 起來但不真的扣款。退房時,飯店再**capture** 實際金額,把沒
用到的部分釋放回去。

## 真正有用的做法

把 reservation 模式對應到 LLM 預算:

```
Phase 1 — Estimate
    Given:  the messages, the model, the pricing table.
    Output: a projected claim (e.g., "this call will cost ~$0.04").

Phase 2 — Auth (Reserve)
    Sidecar checks projected_claim against budget.
    Budget can cover? → record a reservation entry, return RESERVED.
    Budget can't cover? → return STOP, the LLM call must not happen.

Phase 3 — Upstream LLM call
    Application makes the actual provider call.
    Provider returns actual_cost in the usage record.

Phase 4 — Capture (Commit)
    Sidecar receives actual_cost.
    Ledger: reservation → commit, freeing unused portion.

Phase 5 — Release on failure
    If Phase 3 throws / times out / crashes:
        Application calls sidecar.release(decision_id).
        Reservation rolls back, budget is restored.
    Otherwise, a TTL background sweeper auto-releases stale
    reservations after a configurable timeout.
```

由此浮現的特性:

- **呼叫前的拒絕是機制性的。** 超預算那條路徑在 LLM 呼叫之前就 raise。沒有軟性警告
  的分支。
- **auth 階段的估計可以保守一點。** 一個回傳的 token 比預估少的 model,沒用到的部分
  會在 capture 時被釋放。預算 reservation 永遠不會被鎖在超過實際用量的地方。
- **冪等是結構性的。** 重跑同一個 decision(同樣的 decision_id、同樣的 idempotency_key)
  會收斂到既有的那筆 reservation。重試不會重複扣款。
- **crash-safe。** 一個在 auth 之後、capture 之前掛掉的 pod,丟掉的是 in-memory 的
  狀態,不是那筆持久化的紀錄。ledger entry 還在;TTL sweeper 最終會把它釋放掉。

## 給我看 code

這個模式在 SDK 裡浮現出來,就是每通 LLM 呼叫對應一次 decision 呼叫:

```python
# Phase 1+2: estimate + auth
outcome = await sg.request_decision(
    trigger="LLM_CALL_PRE",
    run_id=run_id, decision_id=decision_id,
    route="llm.call",
    projected_claims=[estimated_claim],
    idempotency_key=derive_key(...),
)

# Phase 3: upstream LLM call
try:
    response = await openai.chat.completions.create(...)
except Exception:
    # Phase 5: release on failure
    await sg.release(decision_id)
    raise

# Phase 4: capture
await sg.commit(
    decision_id=decision_id,
    actual_claims=[claim_from(response.usage)],
)
```

framework adapter 把這整段打包進一個 `Model.request()` 的 override,所以應用程式那邊
是「把 model 包一層」的一行,而不是每通呼叫五個步驟的 protocol。

## 這不是什麼

- **它不是一個 billing 系統。** SpendGuard 不開發票、也不跟 provider 對清。它把關
  呼叫;provider 還是會就你打出去的呼叫跟你收錢。
- **它不是一個用量分析 dashboard。** 它把每一筆 reservation 跟 capture 都記進一條
  稽核鏈,但把那個變成 BI 圖表是另一回事。
- **它不是免費的。** 每個 decision 是一次 UDS gRPC 來回(POC 裡 p99 sub-5ms)。對
  每秒打幾十通 LLM 呼叫的 agent 來說,這可以忽略。對更高頻的系統,先量再說。

## 延伸閱讀

- [6 層架構](../concepts/architecture.md) —— reservation 模式在整個 SpendGuard
  runtime 裡的位置
- [Decision 生命週期](../concepts/decision-lifecycle.md) —— auth → capture →
  release 狀態機的細節
- [Ledger storage spec](../reference/ledger-schema.md) —— 實作這條稽核鏈的 Postgres
  schema
- [呼叫前預算上限](pre-call-budget-cap.md) —— 這個模式在實務上的 use-case 框架
