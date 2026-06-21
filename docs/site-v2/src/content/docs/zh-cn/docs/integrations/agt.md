---
title: "Microsoft AGT (Agent Governance Toolkit) 集成"
description: >-
  把 Microsoft AGT (Agent Governance Toolkit) 和 Agentic SpendGuard 组成一个复合策略评估器。
  先跑 AGT 的确定性访问检查;只有 AGT 放行的动作才走 SpendGuard 的预算 reservation——
  AGT 拒绝的动作不会白白占用 reservation。
---


> Microsoft 的 Agent Governance Toolkit 管确定性策略
> (这个用户能不能调这个工具、这个 tenant 能不能访问这份数据)。
> SpendGuard 管花钱相关的策略(这笔预算现在还撑不撑得起这次
> 调用)。把它们组成复合:AGT 先做廉价的否决;剩下的交给
> SpendGuard 做 reservation。AGT 拒绝掉的动作绝不会
> 消耗一次 SpendGuard reservation。

## 为什么需要它

- **分层治理。** 确定性访问控制(AGT)加上花钱控制
  (SpendGuard),统一收口到一个 `evaluate()`
  入口。
- **不浪费 reservation。** AGT 的拒绝在调用 SpendGuard
  sidecar 之前就短路了,所以被拒的动作在 ledger 里不产生
  任何开销。
- **两条审计链,可对账。** AGT 写自己的审计日志;
  SpendGuard 写进 `canonical_events`。把 AGT 事件灌进
  SpendGuard 链的 relay 在 roadmap 上(集成本身不
  涵盖这一块)。

---

## 已经在用 AGT?三条路接入 SpendGuard

挑一条跟你现有 AGT 代码结构对得上的。
三条路都假设下面的[先决条件](#先决条件一次性配置)已经就位。

### 路线 A —— 复合评估器 *(推荐;改动最少)*

你已经有一个 AGT `PolicyEvaluator` 在卡工具动作。包一层就行;
SpendGuard 只在 AGT 放行的动作上跑。

```python
# Before — pure AGT
from agent_os.policies import PolicyEvaluator, PolicyDocument, ...

agt = PolicyEvaluator(policies=[...your existing rules...])
result = await agt.evaluate({"tool_name": "execute_code", ...})
```

```python
# After — composite (AGT first, SpendGuard on AGT-allow only)
from spendguard import SpendGuardClient
from spendguard.integrations.agt import SpendGuardCompositeEvaluator
from spendguard._proto.spendguard.common.v1 import common_pb2

async with SpendGuardClient(
    socket_path="/var/run/spendguard/adapter.sock",
    tenant_id="<your-tenant-uuid>",
) as sg:
    await sg.handshake()

    composite = SpendGuardCompositeEvaluator(
        agt_evaluator=agt,                       # ← existing AGT object, unchanged
        spendguard_client=sg,
        budget_id="<budget-uuid>",
        window_instance_id="<window-uuid>",
        unit=common_pb2.UnitRef(
            unit_id="<unit-uuid>",
            token_kind="output_token",
            model_family="gpt-4",
        ),
        pricing=common_pb2.PricingFreeze(pricing_version="...", ...),
        claim_estimator=lambda payload: [
            common_pb2.BudgetClaim(
                budget_id="<budget-uuid>",
                amount_atomic="500",             # ← your estimate per tool action
                unit=common_pb2.UnitRef(unit_id="<unit-uuid>"),
            )
        ],
    )
    result = await composite.evaluate({"tool_name": "execute_code", ...})
    # result.allowed: bool
    # result.reason: "AGT_DENY: ..." | "SPENDGUARD_DENY: ..." | "ALLOW (AGT + SpendGuard both PASS)"
    # result.matched_rule_ids: list[str]
```

**你的 AGT 规则一行不动。** SpendGuard 只在 AGT 放行之后才跑;AGT 拒绝会短路掉 sidecar 调用 → 不浪费 reservation。

### 路线 B —— `gate_budget()` hook *(适合策略回调驱动的流程)*

如果 AGT 的某条策略里本来就在跑异步回调,加一行就够:

```python
from spendguard.integrations.agt import gate_budget

async def my_existing_policy_callback(payload):
    # ... your existing AGT logic ...

    await gate_budget(                           # ← new line
        payload,
        client=sg,
        budget_id="<budget-uuid>",
        window_instance_id="<window-uuid>",
        unit=unit,
        pricing=pricing,
        claim_estimator=estimator,
    )
    # `gate_budget` raises DecisionDenied if SpendGuard denies;
    # AGT chain surfaces that as a hard policy fail.

    return PolicyDecision.ALLOW
```

### 路线 C —— 并行调用 *(灵活性最高)*

如果你的架构里有多个评估器,而你想让 SpendGuard 和 AGT 并排存在(而不是嵌进 AGT 里),那就在你想做预算卡点的任何地方直接调 `sg.request_decision(...)`。它跟 AGT 的先后顺序得你自己来安排。

```python
# Whenever you decide to budget-gate, regardless of AGT
outcome = await sg.request_decision(
    trigger="LLM_CALL_PRE",
    run_id=run_id, step_id=step_id, llm_call_id=call_id,
    decision_id=decision_id, route="llm.call",
    projected_claims=[claim],
    idempotency_key=derive_idempotency_key(...),
)
```

---

## 先决条件(一次性配置)

| 步骤 | 内容 |
|---|---|
| 1. **部署 sidecar** | Helm:`helm install spendguard charts/spendguard`(DaemonSet —— 每个节点一个 pod)。Docker Compose:本地开发用 `make demo-up`。 |
| 2. **Postgres ledger 可达** | 通过 `postgres.ledgerUrl` + `postgres.canonicalUrl`(Helm)传入连接串,或走 demo 的 compose 配置。 |
| 3. **Tenant + budget 已 seed** | 通过 control-plane REST API 插入(`POST /v1/tenants`、`POST /v1/budgets`),或在安装时直接通过 SP 写入。 |
| 4. **Contract bundle 已发布** | 写一份带你规则的 `contract.yaml`(比如 `hard-cap-deny when claim > 1B`),用 `bundles-init` ConfigMap 或你 operator 的 bundle 流水线打包。 |
| 5. **装好 Python SDK** | `pip install --pre 'spendguard-sdk[agt]'` —— 会把 `agent-governance-toolkit>=3.4` + `agent-os-kernel>=3.0` 作为传递依赖一并拉下来。这个包在 PyPI 上是 alpha,所以 `--pre` 标志是必需的。 |

---

## 运维坑点

- **Reservation TTL 默认 60s。** 如果 AGT 放行的工具动作跑得比这更久(一条长 shell 命令、一次多轮 LLM 调用),SpendGuard 会自动释放这次 reservation。对于长工具调用,在 contract bundle 的 budget 块里调大 `reservation_ttl_seconds`,或者传 `X-SpendGuard-Reservation-TTL`(proxy 路径)。
- **AGT 拒绝的动作不会出现在 `canonical_events` 里。** AGT 在 SpendGuard 跑之前就短路了。AGT 自己的审计日志记下这次拒绝;SpendGuard 链只记 AGT 放行 → SpendGuard reservation/commit/release。等 AGT → SpendGuard relay 落地后,两条链可以按 `decision_id` 对账(两边都会发这个字段)。
- **复合的 `result.reason` 跟 AGT 的裁决走。** 当两层都会拒绝时,你看到的是 `AGT_DENY: ...`,因为 AGT 短路了。想在拒绝场景里看到 SpendGuard 的原因,得 AGT 先放行。
- **`claim_estimator` 每次 `evaluate()` 调用都会跑。** 每次调用都是一次独立的 SpendGuard `request_decision` —— 所以 AGT 流量大的话,会 1:1 对应到 sidecar 的 UDS 往返(同一 pod 上每次约 1–3ms)。
- **多 tenant。** 每个 tenant 建一个 `SpendGuardCompositeEvaluator`。这样 AGT 策略和 SpendGuard 的 `budget_id` 就都反映各自 tenant 的配置,不用在运行时分支判断。

---

## 最快验证

随包附带的 demo 会拿真实的 sidecar + ledger + Postgres 把三条路全跑一遍:

```bash
make demo-down -v
DEMO_MODE=agent_real_agt make demo-up
```

预期输出:

```
[demo] handshake ok session_id=...
[demo] (1) AGT-deny: allowed=False reason="AGT_DENY: Matched rule 'deny-dangerous'"
[demo] (2) AGT+SG allow: allowed=True reason='ALLOW (AGT + SpendGuard both PASS)'
[demo] (3) AGT-allow+SG-deny: allowed=False reason='SPENDGUARD_DENY: BUDGET_EXHAUSTED'
[demo] AGT composite all 3 paths PASS
```

完整 demo 源码:[`deploy/demo/demo/run_demo.py::run_agt_composite_mode`](https://github.com/m24927605/agentic-spendguard/blob/main/deploy/demo/demo/run_demo.py) —— 照着改成你自己的 AGT 规则集即可。

---

## 全新项目示例(没有现成 AGT)

如果你是从零开始,想看两层是怎么接到一起的:

```python
import asyncio

from agent_os.policies import (
    PolicyEvaluator, PolicyDocument, PolicyRule, PolicyCondition,
    PolicyAction, PolicyOperator, PolicyDefaults,
)

from spendguard import SpendGuardClient, new_uuid7
from spendguard.integrations.agt import SpendGuardCompositeEvaluator
from spendguard._proto.spendguard.common.v1 import common_pb2


async def main() -> None:
    # 1. AGT: deterministic access policy
    agt = PolicyEvaluator(policies=[
        PolicyDocument(
            name="block-untrusted-tools", version="1.0",
            defaults=PolicyDefaults(action=PolicyAction.ALLOW),
            rules=[
                PolicyRule(
                    name="deny-dangerous",
                    condition=PolicyCondition(
                        field="tool_name",
                        operator=PolicyOperator.IN,
                        value=["shell", "delete_file"],
                    ),
                    action=PolicyAction.DENY,
                    priority=100,
                ),
            ],
        )
    ])

    # 2. SpendGuard client
    client = SpendGuardClient(
        socket_path="/var/run/spendguard/adapter.sock",
        tenant_id="00000000-0000-4000-8000-000000000001",
    )
    await client.connect()
    await client.handshake()

    # 3. Composite
    composite = SpendGuardCompositeEvaluator(
        agt_evaluator=agt,
        spendguard_client=client,
        budget_id="44444444-4444-4444-8444-444444444444",
        window_instance_id="55555555-5555-4555-8555-555555555555",
        unit=common_pb2.UnitRef(
            unit_id="66666666-6666-4666-8666-666666666666",
            token_kind="output_token",
            model_family="gpt-4",
        ),
        pricing=common_pb2.PricingFreeze(pricing_version="demo-pricing-v1"),
        claim_estimator=lambda payload: [
            common_pb2.BudgetClaim(
                budget_id="44444444-4444-4444-8444-444444444444",
                window_instance_id="55555555-5555-4555-8555-555555555555",
                amount_atomic="500",
                unit=common_pb2.UnitRef(unit_id="66666666-6666-4666-8666-666666666666"),
            )
        ],
    )

    # 4. Evaluate
    result = await composite.evaluate({
        "tool_name": "web_search",
        "tool_args": {"q": "AI agent budget control"},
        "tenant_id": "00000000-0000-4000-8000-000000000001",
        "run_id": str(new_uuid7()),
    })
    print(result.allowed, result.reason)


asyncio.run(main())
```

---

## 相关

- [Quickstart](../quickstart.md) —— 5 分钟拉起整套栈
- [Contract DSL 参考](../contracts/yaml.md) —— 编写 allow/stop 规则
- 其他集成:[Pydantic-AI](pydantic-ai.md) · [LangChain & LangGraph](langchain.md) · [OpenAI Agents SDK](openai-agents.md)
