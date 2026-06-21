---
title: "用 SpendGuard 给 Google ADK 做预算管控"
description: >-
  用 Agentic SpendGuard 给 Google Agent Development Kit
  (google-adk) 的 agent 做调用前的 token 预算管控。一个 callback 同时覆盖
  before_model_callback 和 after_model_callback 两个槽位,支持
  Gemini 直连、Vertex 后端的 Gemini,以及 LiteLlm 包装的 OpenAI /
  Anthropic。
---


> 你的 Google ADK `LlmAgent` 在一条棘手的推理链上撞进了 tool-call
> 死循环。每一轮 `before_model_callback` 都会往 Gemini 再发一个
> 请求。没有 gate,你得等到下个月的账单 dashboard 才知道烧了多少钱。
> SpendGuard 把同一个 `SpendGuardAdkCallback` 同时挂到
> `before_model_callback` 和 `after_model_callback` 上,这样每一轮 model
> 调用在上游请求发出*之前*就先对 budget 做一次 reserve —— 而且同一个
> callback 对 Vertex Gemini 或 LiteLlm 包装的 OpenAI / Anthropic 一样管用。

## 为什么需要它

- **一个 callback,两个槽位。** `SpendGuardAdkCallback` 是单个实例,你把它
  同时注册到 `before_model_callback` 和 `after_model_callback`。分派靠
  payload 类型走 —— `LlmRequest` 进 PRE,`LlmResponse` 进 POST。
- **按 shape 区分多厂商,不靠字符串匹配。** 对
  `LlmAgent(model="gemini-2.0-flash")`、Vertex 后端的 Gemini,以及
  `LlmAgent(model=LiteLlm("openai/gpt-4o-mini"))` 都能用,因为 usage
  提取读的是 `usage_metadata` 字段的 shape,不是 model 字符串匹配。
- **调用前拒绝,不是事后记账。** 超预算的调用会返回一个合成的
  `LlmResponse(error_code="SPENDGUARD_DENY")`,让 ADK 直接短路掉这一轮
  —— 压根不会去碰 Gemini API。
- **审计 + 审批管线与其他所有框架共享。** 这个 callback 写入的是和
  LangChain、Pydantic-AI、OpenAI Agents 集成同一套 SpendGuard ledger,所以
  一个跨多框架的 agent 集群能拿到统一的一份决策日志。

## 配置(60 秒)

```bash
pip install 'spendguard-sdk[adk]'
```

用 demo 栈拉起一个 sidecar:

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard && make demo-up
```

## 接线

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

## 你能拿到什么

- **每一轮 `LlmAgent` model 调用都做调用前 budget reservation**,包括
  tool-loop 的每次迭代。
- **多厂商覆盖。** Gemini 直连、Vertex Gemini、LiteLlm 包装 —— 全部按
  `usage_metadata` 的 shape 提取 usage。
- **并发安全。** ADK 在每次 `Runner.run_async` 调用时都构造一个全新的
  `CallbackContext`,所以并发的 run 天然通过 `callback_context.state`
  彼此隔离。
- **DENY 不抛异常。** callback 走的是文档里写明的
  `LlmResponse(error_code="SPENDGUARD_DENY", ...)` 短路通道,因此用户自己
  的 `after_model_callback` 链(如果有)照样能看到这次 deny。

## 常见用法

### 自定义 run_id 做跨框架关联

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

默认是 `ctx.invocation_id`(ADK 每次 `Runner.run_async` 分配一个 UUID)。
当你想让 run_id 跟某个 LangChain 或 OpenAI Agents 的父 trace 对齐时,就覆盖它。

### 自定义 claim estimator

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

不传时,callback 会根据 `req.model` 分派默认 estimator(Gemini 家族 /
经 LiteLlm 前缀剥离后的 OpenAI / 未知 model 走 chars/4 兜底并打一次性 warning)。

### DENY 行为

当 `request_decision` 返回 DENY 时,callback 会:

1. 设置 `ctx.state["spendguard.denied"] = True`。
2. 返回一个合成的 `LlmResponse`,其 `error_code="SPENDGUARD_DENY"`,
   `error_message` 里是逗号拼接的 reason code(默认是 `BUDGET_EXHAUSTED`)。
3. ADK 终止这一轮 —— 压根碰不到内层的 Gemini transport。
4. POST 是 no-op(不 commit、不 release —— 这次 deny 没带任何 reservation)。

你可以把自己的 `after_model_callback` 串在 SpendGuard 的*后面*,用来记录
这次 deny,同时不丢掉短路语义。

### Tool callback

Tool callback(`before_tool_callback` / `after_tool_callback`)不在 v0.1.x
的范围内。spend gating 落在 model 边界上;tool 调用本身不直接驱动 spend。
tool 级别的 budget 管控留作后续增强项继续跟踪。

## 相关

- [Quickstart](../quickstart.md) —— 5 分钟拉起整套栈
- [Contract DSL 参考](../contracts/yaml.md) —— 编写 allow/stop 规则
- 其他集成:[Pydantic-AI](pydantic-ai.md) · [LangChain & LangGraph](langchain.md) · [OpenAI Agents SDK](openai-agents.md) · [Microsoft AGT](agt.md)
