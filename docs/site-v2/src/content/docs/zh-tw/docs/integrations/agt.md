---
title: "Microsoft AGT (Agent Governance Toolkit) 整合"
description: >-
  把 Microsoft AGT (Agent Governance Toolkit) 和 Agentic SpendGuard 串成
  一個複合式 policy evaluator。AGT 那層 deterministic 的存取檢查先跑;只有 AGT
  放行的 action 才會走 SpendGuard 的 budget reservation —— AGT 擋掉的就不會白白
  佔一筆 reservation。
---


> Microsoft 的 Agent Governance Toolkit 負責 deterministic 的 policy
> (這個 user 能不能呼叫這個 tool、這個 tenant 能不能存取這份 data)。
> SpendGuard 負責跟花費有關的 policy(這筆 budget 現在付不付得起這次
> 呼叫)。把兩者串成一個複合式評估:AGT 先用便宜的方式否決;剩下放行的
> 才交給 SpendGuard 去 reserve。被 AGT 擋掉的 action 永遠不會吃掉一筆
> SpendGuard reservation。

## 你為什麼會想要這個

- **分層治理。** Deterministic 的存取控制(AGT)加上花費控制
  (SpendGuard),全部收在同一個 `evaluate()` 進入點後面。
- **不浪費 reservation。** AGT 一擋掉就直接 short-circuit,根本不會打到
  SpendGuard sidecar,所以被擋的 action 在 ledger 裡完全不花成本。
- **兩條 audit trail,可以對帳。** AGT 寫自己那份 audit log;SpendGuard
  寫進 `canonical_events`。把 AGT 的 event 收進 SpendGuard chain 的 relay
  還在 roadmap 上(整合本身不含這塊)。

---

## 已經在用 AGT 了?三條路把 SpendGuard 加進來

挑一條跟你現有 AGT 程式碼結構最對得上的就好。三條都假設下面的
[先決條件](#先決條件一次性設定)已經就緒。

### Path A —— 複合式 Evaluator *(建議;改動最少)*

你已經有一個 AGT `PolicyEvaluator` 在 gate tool action。包一層就好;
SpendGuard 只會在 AGT 放行的 action 上跑。

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

**你的 AGT rule 完全不用動。** SpendGuard 只在 AGT 放行之後才跑;AGT 一擋掉就 short-circuit 掉那次 sidecar 呼叫 → 不會白佔一筆 reservation。

### Path B —— `gate_budget()` hook *(給 policy callback 驅動的流程)*

如果你的 AGT 已經在某條 policy 裡跑一個 async callback,那就加一行:

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

### Path C —— 平行呼叫 *(彈性最大)*

如果你的架構有好幾個 evaluator,而你想讓 SpendGuard 跟 AGT 並排(而不是塞在 AGT 裡面),那就在任何你想做 budget gate 的地方直接呼叫 `sg.request_decision(...)`。它跟 AGT 之間怎麼排序,要你自己負責。

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

## 先決條件(一次性設定)

| 步驟 | 內容 |
|---|---|
| 1. **部署 sidecar** | Helm:`helm install spendguard charts/spendguard`(DaemonSet —— 每個 node 一個 pod)。Docker Compose:本機開發用 `make demo-up`。 |
| 2. **連得到 Postgres ledger** | 透過 `postgres.ledgerUrl` + `postgres.canonicalUrl`(Helm)把連線字串傳進來,或走 demo 的 compose 設定。 |
| 3. **建好 tenant + budget** | 透過 control-plane REST API(`POST /v1/tenants`、`POST /v1/budgets`)塞進去,或在安裝時直接從 SP 寫入。 |
| 4. **發佈 contract bundle** | 寫一份 `contract.yaml` 放你的 rule(例如 `hard-cap-deny when claim > 1B`),用 `bundles-init` ConfigMap 或你 operator 的 bundle pipeline 打包。 |
| 5. **裝好 Python SDK** | `pip install --pre 'spendguard-sdk[agt]'` —— 會順帶把 `agent-governance-toolkit>=3.4` + `agent-os-kernel>=3.0` 當 transitive dep 拉進來。這個套件在 PyPI 上還是 alpha,所以 `--pre` 這個 flag 一定要加。 |

---

## 操作上要注意的眉角

- **Reservation TTL 預設是 60s。** 如果 AGT 放行的那個 tool action 跑得比這還久(一條長的 shell command、一次多輪的 LLM 呼叫),SpendGuard 會自動把 reservation 釋放掉。tool 呼叫會拖很久的話,把 contract bundle 裡 budget 區塊的 `reservation_ttl_seconds` 調大,或是傳 `X-SpendGuard-Reservation-TTL`(走 proxy 路徑那條)。
- **被 AGT 擋掉的 action 不會出現在 `canonical_events` 裡。** AGT 在 SpendGuard 跑之前就 short-circuit 掉了。AGT 自己那份 audit log 會記下這個 deny;SpendGuard 的 chain 只會記到「AGT 放行 → SpendGuard reservation/commit/release」這段。等之後 AGT → SpendGuard 的 relay 出來,兩條 chain 可以靠 `decision_id` 對帳(兩邊都會帶這個值)。
- **複合式的 `result.reason` 跟著 AGT 的判決走。** 兩層都會 deny 的時候,你看到的會是 `AGT_DENY: ...`,因為 AGT 先 short-circuit 掉了。想在 deny 的情境看到 SpendGuard 的理由,前提是 AGT 得先放行。
- **`claim_estimator` 是每次 `evaluate()` 呼叫都跑一次。** 每次呼叫都是它自己一筆 SpendGuard `request_decision` —— 所以 AGT 流量一大,就 1:1 對應到 sidecar 的 UDS round trip(同一個 pod 上大概一次 ~1–3ms)。
- **多 tenant。** 一個 tenant 建一個 `SpendGuardCompositeEvaluator`。這樣 AGT policy 跟 SpendGuard 的 `budget_id` 就都各自反映該 tenant 的設定,runtime 完全不用分支判斷。

---

## 最快的驗證方式

內附的 demo 會拿真的 sidecar + ledger + Postgres 把三條路都跑一遍:

```bash
make demo-down -v
DEMO_MODE=agent_real_agt make demo-up
```

預期輸出:

```
[demo] handshake ok session_id=...
[demo] (1) AGT-deny: allowed=False reason="AGT_DENY: Matched rule 'deny-dangerous'"
[demo] (2) AGT+SG allow: allowed=True reason='ALLOW (AGT + SpendGuard both PASS)'
[demo] (3) AGT-allow+SG-deny: allowed=False reason='SPENDGUARD_DENY: BUDGET_EXHAUSTED'
[demo] AGT composite all 3 paths PASS
```

完整 demo 原始碼:[`deploy/demo/demo/run_demo.py::run_agt_composite_mode`](https://github.com/m24927605/agentic-spendguard/blob/main/deploy/demo/demo/run_demo.py) —— 複製過去改成你自己的 AGT rule set 就好。

---

## 全新專案範例(還沒有 AGT)

如果你是從零開始,想看看兩層怎麼接起來:

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

## 相關文件

- [Quickstart](../quickstart.md) —— 5 分鐘把整個 stack 拉起來
- [Contract DSL reference](../contracts/yaml.md) —— 撰寫 allow/stop rule
- 其他整合:[Pydantic-AI](pydantic-ai.md) · [LangChain & LangGraph](langchain.md) · [OpenAI Agents SDK](openai-agents.md)
