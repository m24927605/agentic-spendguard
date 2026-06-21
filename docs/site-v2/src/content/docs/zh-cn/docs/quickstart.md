---
title: "快速开始"
description: >-
  在本地五分钟内把整套 Agentic SpendGuard 跑起来 —— Docker Compose 会拉起
  sidecar、ledger、canonical-ingest、dashboard,以及一个 Pydantic-AI agent
  demo,开箱就有一个能跑的 DENY 决策。
---


从 `git clone` 到一个绿灯的 DENY demo,大约 5 分钟。

## 前置条件

- Docker Desktop(或任何 Docker engine + Compose v2)
- ~4 GiB 可用 RAM(Rust 服务会并行 build)
- 第一次 build 约 5 分钟(之后有 cache 就快了)

## 1. Clone 下来、把整套拉起来

```bash
git clone https://github.com/m24927605/agentic-spendguard
cd agentic-spendguard/deploy/demo

docker compose down -v --remove-orphans   # 干净状态
DEMO_MODE=deny make demo-up
```

预期输出(最后几行):

```text
[demo] DENY raised decision_id=... reason_codes=['BUDGET_EXHAUSTED']
       matched_rule_ids=['{contract}:{budget}:hard-cap-deny']
[demo] DENY assertions PASS
```

刚刚发生了什么:

1. Postgres 起来,跑了 ledger + canonical migration
2. `pricing-seed-init` 把 `deploy/demo/init/pricing/seed.yaml` 加载进
   `pricing_table`(35 行、5 个 provider)
3. `bundles-init` 把一份真的 `contract.yaml` 打进 demo bundle
4. `pki-init` 签出一个 CA + 每个服务各自的 mTLS 证书
5. `ledger`、`canonical-ingest`、`sidecar` 等等都起来了
6. `demo` container 通过 UDS 连上,发出一笔 $2000 的预算 claim
7. Sidecar 的 contract evaluator 匹配到 `hard-cap-deny` 规则(上限 $1000)
   → adapter 抛出 `DecisionStopped`
8. 写下 carrier 的 `denied_decision` 行 + `audit_outbox` 行
9. `outbox-forwarder` 把那条审计记录推进 `canonical_events`

## 2. 跑跑其他 demo 模式

| 模式 | 它展示什么 |
|---|---|
| `decision` | 完整的 reserve→commit→provider_report 生命周期 |
| `invoice` | + invoice_reconcile(三阶段 commit 收尾) |
| `agent` | Pydantic-AI Agent + MockLLM |
| `release` | reserve → RUN_ABORTED → release(全额退回) |
| `ttl_sweep` | reserve(TTL=5s)→ sweeper 自动 release |
| **`deny`** | **Phase 3 wedge:contract evaluator STOP** |
| `agent_real` | 真的调用 OpenAI gpt-4o-mini(需要 `OPENAI_API_KEY`) |
| `agent_real_anthropic` | 真的调用 Claude haiku-4-5 |
| `agent_real_langchain` | LangChain ChatOpenAI |
| `agent_real_langgraph` | LangGraph ReAct agent |
| `multi_provider_usd` | 跨 provider 的 USD 预算净额结算 |

```bash
# 真实 OpenAI:
export OPENAI_API_KEY=sk-...
DEMO_MODE=agent_real make demo-up

# 跨 provider USD:
export OPENAI_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-...
DEMO_MODE=multi_provider_usd make demo-up
```

## 3. 打开 dashboard

```bash
docker compose up -d dashboard
open http://localhost:8090
```

弹出提示时输入 bearer token(`demo-dashboard-token-replace-in-production`)。
你会看到预算总览、最近的决策、DENY 统计,以及 outbox forwarder 的健康状态。

## 4. 通过 control plane 开一个新 tenant

```bash
docker compose up -d control-plane

curl -X POST http://localhost:8091/v1/tenants \
  -H 'Authorization: Bearer demo-admin-token-replace-in-production' \
  -H 'Content-Type: application/json' \
  -d '{"name": "acme-corp", "opening_deposit_atomic": "1000"}'
```

响应里会带上 `sidecar_config_env` —— 一整块环境变量,你可以直接塞进某个 sidecar 的
deployment 里。

## 下一步

- [Authoring contracts](contracts/yaml.md) —— 写你自己的规则
- [Pydantic-AI 集成](integrations/pydantic-ai.md) —— 把 SDK 接入你的 agent
- [Helm 部署](deployment/helm.md) —— 搬到 k8s
