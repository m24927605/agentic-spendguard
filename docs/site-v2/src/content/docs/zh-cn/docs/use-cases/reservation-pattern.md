---
title: "LLM 预算的 reservation 模式"
description: >-
  LLM 花费的 reservation 模式。一个 Stripe 式的 auth/capture ledger,把 runtime 的
  预算把关变成一个原语(primitive)。看调用前的关卡、预估的 reservation,以及调用后的
  capture/release 阶段,是怎么组成一条成本正确的审计轨迹。
---


> auth/capture 这个模式在支付领域已经四十年了,在 feature flag 领域也有几年了。它
> 还没成为 LLM 花费的一个原语,但它应该要是 —— 同样的形状解的是同一类问题。这篇从
> 第一性原理讲这个模式,并展示 Agentic SpendGuard 是怎么实现它的。

## 为什么标准答案没用

“控制 LLM 花费”最直觉的设计是一个 counter:

```
budget_remaining = 1000.00
on each LLM call:
    budget_remaining -= actual_cost   # arrives after the call
    if budget_remaining < 0: alert
```

这会崩,因为**实际成本是在调用之后才到的**。provider 返回一笔 usage 记录,告诉你它
计了多少 token 的费。等你把 counter 减下去时,钱早就花掉了。这个 counter 是个记账
工具,不是一道关卡。

要把关,你得在调用*之前*就知道成本。但 LLM 的成本不是固定的 —— output token 取决于
model 生成了什么,而那又取决于……你还没打的那次调用。

这正是 auth/capture 在支付领域解的问题。当你入住时,Visa 网络并不知道一笔酒店账单的
最终金额 —— 酒店可能再加酒水、room service、房间损坏费。所以酒店先**授权(auth-hold)**一个
预估金额,把资金 reserve 起来但不真的扣款。退房时,酒店再**capture** 实际金额,把没
用到的部分释放回去。

## 真正有用的做法

把 reservation 模式对应到 LLM 预算:

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

由此浮现的特性:

- **调用前的拒绝靠机制保证,不靠自觉。** 超预算那条路径在 LLM 调用之前就 raise。没有软告警
  的分支。
- **auth 阶段的估计可以保守一点。** 一个返回的 token 比预估少的 model,没用到的部分
  会在 capture 时被释放。预算 reservation 永远不会被锁在超过实际用量的地方。
- **幂等是结构性的。** 重跑同一个 decision(同样的 decision_id、同样的 idempotency_key)
  会收敛到已有的那笔 reservation。重试不会重复扣款。
- **crash-safe。** 一个在 auth 之后、capture 之前挂掉的 pod,丢掉的是 in-memory 的
  状态,不是那笔持久化的记录。ledger entry 还在;TTL sweeper 最终会把它释放掉。

## 给我看 code

这个模式在 SDK 里浮现出来,就是每次 LLM 调用对应一次 decision 调用:

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

framework adapter 把这整段打包进一个 `Model.request()` 的 override,所以应用代码那边
是“把 model 包一层”的一行代码,而不是每次调用都要走五步流程。

## 这不是什么

- **它不是一个 billing 系统。** SpendGuard 不开发票、也不跟 provider 对清。它把关
  调用;provider 还是会就你打出去的调用跟你计费。
- **它不是一个用量分析 dashboard。** 它把每一笔 reservation 和 capture 都记进一条
  审计链,但把那个变成 BI 图表是另一回事。
- **它不是免费的。** 每个 decision 是一次 UDS gRPC 往返(POC 里 p99 sub-5ms)。对
  每秒打几十次 LLM 调用的 agent 来说,这可以忽略。对更高频的系统,先压测再说。

## 延伸阅读

- [6 层架构](../concepts/architecture.md) —— reservation 模式在整个 SpendGuard
  runtime 里的位置
- [Decision 生命周期](../concepts/decision-lifecycle.md) —— auth → capture →
  release 状态机的细节
- [Ledger storage spec](../reference/ledger-schema.md) —— 实现这条审计链的 Postgres
  schema
- [调用前预算上限](pre-call-budget-cap.md) —— 这个模式在实践中的 use-case 框架
