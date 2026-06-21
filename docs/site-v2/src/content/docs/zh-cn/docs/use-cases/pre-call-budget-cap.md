---
title: "LLM API 请求的调用前预算上限"
description: >-
  在花费发生之前、而不是之后,对 LLM API 请求设下硬性的 token 预算上限。给 OpenAI、
  Anthropic 以及任何 provider 的 Stripe 式 auth/capture reservation —— 超预算的调用
  会在边界上就被拒绝。
---


> 你想要一个上限,它说“这个 agent 每小时在 gpt-4o 上最多花 $X,任何一次会把它推过
> 这条线的调用,都必须*在*请求发到 provider *之前*被拒绝”。Token 用量 dashboard 和
> 日告警给你的是事后的数字,不是那道关卡。这篇给你的就是那道关卡的做法。

## 为什么标准答案没用

大多数 LLM 成本工具做的是**对账**,不是**控制**:

| 做法 | 它做什么 | 你什么时候才知道 |
|---|---|---|
| provider 账单 / billing API | 告诉你花了多少 | 账期结束时 |
| 用量 dashboard | 把 token 数汇总 | 花完之后好几小时 |
| provider key 上的 rate limit | 限每秒/每分钟的请求数 | 不是看金额 —— 是看次数 |
| soft alert(“你到 80% 了”) | ping 一个 webhook | 预算已经花掉大半之后 |

这些没有一个会*阻止*那次调用。它们只是把账单报给你 —— 运气好的话能赶在下一张账单出来之前。当一个 agent
陷在重试循环或 tool-use 循环里时,“把钱花掉”和“看到 dashboard”之间的那段空档,
正是真正的伤害发生的时候。

## 真正有用的做法

每一次 LLM 调用前面都坐着一个预算 reservation。这个 reservation 的行为就像 Stripe
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

有三个特性让它能成立:

1. **调用前的拒绝靠机制保证,不靠自觉。** 超预算那条路径是一个被抛出来的 exception,不是一个
   软告警。应用代码不可能不小心把它忽略掉。
2. **reservation 是真正入账的,不是拍脑袋估个数。** ledger 把 reservation(auth 阶段)
   和 commit(capture 阶段)分开记,所以预估 reserve 了 1,500 个 token、实际只用了
   800 个,就会把 700 个释放回预算。
3. **重试时幂等。** 一次输入完全相同的重试会收敛到原本那笔 reservation,而不是再分
   一笔新的。否则一个重试 47 次的循环会烧掉 47 倍的 reservation。

## 给我看 code

reservation 就是一次调用。Agentic SpendGuard SDK 帮你处理掉 auth/commit/release 的
整个生命周期:

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
            projected_claims=[claim],          # 预估的 USD 或 token
            idempotency_key=derive_key(...),   # 跨重试都稳定
        )
        # reservation 做好了。现在去打 LLM 调用。
    except DecisionStopped as e:
        # 超预算了。这次 LLM 调用绝对不能发生。
        raise
```

framework adapter(Pydantic-AI / LangChain / OpenAI Agents / AGT)会把这整段包进
一个 `Model.request()` 的 override 里,所以应用代码那边不用改。

## 延伸阅读

- [Pydantic-AI 集成](../integrations/pydantic-ai.md) —— drop-in 的 `Model` wrapper,
  帮你处理 auth/capture 生命周期
- [Reservation 模式深入](reservation-pattern.md) —— LLM 花费 auth/capture 背后的
  架构推理
- [拦住失控的 agent](agent-runaway-protection.md) —— 这个模式专门要防的就是那个失效模式
- [Contract DSL reference](../contracts/yaml.md) —— 写出每次调用该 allow / stop /
  require-approval 的规则
