---
title: "在失控的 AI agent 把账单砸到你之前先拦住它"
description: >-
  当一个 AI agent 把同一次失败的 LLM 调用重试几百次,账单会在六小时后才落地。
  这篇教你怎么把每一次重试都拿去对预算把关,让失控循环在机制上根本不可能发生 ——
  而不是事后才监控到。
---


> 凌晨 3 点。你的 LangChain agent 因为某个下游工具一直返回坏掉的结果,把同一次
> `gpt-4o` 调用重试了 47 次。每一次重试都发去 OpenAI,每一次重试都被 provider
> 计费。等到 on-call 值班在早上 6 点注意到告警时,这个 agent 已经烧掉 $400、却
> 一个有用的输出都没产出。Token 用量的 dashboard 有抓到 —— 晚了六小时。

## 为什么标准答案没用

直觉是加监控:跟踪每个 agent 的花费,日预算到 80% 就发告警,到 100% 就叫
on-call。这是标准套路。但在失控这种情况下,它有三个地方会崩:

1. **监控是事后的。** agent 在告警发出去之前就已经把钱花掉了。告警只是告诉你
   坏事已经发生了。
2. **每次调用的成本无法预测。** 一次被重试的调用,随着 context window 变大,可能
   吃掉 1,000 个 token,也可能吃掉 100,000 个。你没办法从日预算反推出一个每次调用
   的上限。
3. **重试循环不理会 rate limit。** provider 的 rate limit 限的是请求数,不是金额。
   一个每 token 贵 10 倍的 model(gpt-4o vs gpt-4o-mini),在同一个 rate limit 下
   就烧掉 10 倍的钱。

真正该设的关卡不是“用量看着不对就告警”,而是“预算兜不住就拒掉这次
调用”。而且这件事必须放在请求路径上、在上游调用之前、每一次都做。

## 真正有用的做法

每一次 LLM 调用都会经过一个持有 auth/capture ledger 的 sidecar。在请求路径上:

1. SDK wrapper 算出一个预估的 claim(根据输入长度 + model 定价,估出 USD 或 token
   成本)。
2. sidecar 拿这个预估 claim 去对预算的 reservation 余额。
3. **如果预算扛不住:** wrapper 抛出 `DecisionStopped`。这次 LLM 调用根本不会
   发出去。应用代码那边看到的就是一个 Python exception,照常往上传。
4. **如果预算扛得住:** sidecar 把金额 reserve 起来然后返回。wrapper 再去打
   上游调用。

有两个特性让它对“失控循环”这种情况特别有韧性:

- **幂等的 reservation。** 一次输入完全相同的重试(同样的 messages + 同样的 model
  配置 + 同样的 run/step ID)会收敛到原本那笔 reservation 上。一个重试 47 次的循环
  只分配一笔 reservation,不是 47 笔。
- **crash 时自动 release。** 如果 wrapper 或 pod 在 reserve 之后、完成上游调用之前
  挂掉,那笔 reservation 会在一个可配置的 TTL(默认 600 秒)之后过期。预算不会被
  永久锁住。

整体效果:一个失控循环打出第一次重试、拿到 reservation,后续的重试都撞上关卡
(幂等收敛),然后卡在它本来就在失败的那件事上 —— 而不是卡在把钱花爆。

## 给我看 code

LangChain 的 wrapper 让这道关卡对 agent 来说是透明的:

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
    budget_id="my-budget",            # 你要强制执行的上限
    window_instance_id="hourly-2026",
    # ... pricing + claim_estimator 配置
)

agent = create_react_agent(guarded, tools=[my_tool])

async with run_context(RunContext(run_id=str(new_uuid7()))):
    await agent.ainvoke({"messages": [HumanMessage(content="...")]})
```

当预算用完时,agent 下一次 `invoke()` 会抛出 `DecisionStopped`,而不是去打
OpenAI。重试循环会自己退下来,不再继续烧钱。

## 延伸阅读

- [LangChain & LangGraph 集成](../integrations/langchain.md) —— 上面用到的 wrapper
- [调用前预算上限](pre-call-budget-cap.md) —— reservation 模式的细节
- [快速开始](../quickstart.md) —— 5 分钟把整套跑起来,含一个会走到这条路径的 DENY demo
- [Contract DSL reference](../contracts/yaml.md) —— 写出决定哪些调用会触发关卡的规则
