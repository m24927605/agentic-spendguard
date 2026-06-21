---
title: "快速開始"
description: >-
  在本機五分鐘內把整套 Agentic SpendGuard 跑起來 —— Docker Compose 會帶起
  sidecar、ledger、canonical-ingest、dashboard,以及一個 Pydantic-AI agent
  demo,開箱就有一個會動的 DENY 決策。
---


從 `git clone` 到一個綠燈的 DENY demo,大約 5 分鐘。

## 事前準備

- Docker Desktop(或任何 Docker engine + Compose v2)
- ~4 GiB 可用 RAM(Rust 服務會平行 build)
- 第一次 build 約 5 分鐘(之後有 cache 就快了)

## 1. Clone 下來、把整套帶起來

```bash
git clone https://github.com/m24927605/agentic-spendguard
cd agentic-spendguard/deploy/demo

docker compose down -v --remove-orphans   # 乾淨狀態
DEMO_MODE=deny make demo-up
```

預期輸出(最後幾行):

```text
[demo] DENY raised decision_id=... reason_codes=['BUDGET_EXHAUSTED']
       matched_rule_ids=['{contract}:{budget}:hard-cap-deny']
[demo] DENY assertions PASS
```

剛剛發生了什麼:

1. Postgres 起來,跑了 ledger + canonical migration
2. `pricing-seed-init` 把 `deploy/demo/init/pricing/seed.yaml` 載進
   `pricing_table`(35 列、5 個 provider)
3. `bundles-init` 把一份真的 `contract.yaml` 烤進 demo bundle
4. `pki-init` 簽出一個 CA + 每個服務各自的 mTLS 憑證
5. `ledger`、`canonical-ingest`、`sidecar` 等等都起來了
6. `demo` container 透過 UDS 連上,送出一筆 $2000 的預算 claim
7. Sidecar 的 contract evaluator 對到 `hard-cap-deny` 規則(上限 $1000)
   → adapter 丟出 `DecisionStopped`
8. 寫下 carrier 的 `denied_decision` 列 + `audit_outbox` 列
9. `outbox-forwarder` 把那筆稽核紀錄推進 `canonical_events`

## 2. 跑跑其他 demo 模式

| 模式 | 它展示什麼 |
|---|---|
| `decision` | 完整的 reserve→commit→provider_report 生命週期 |
| `invoice` | + invoice_reconcile(三階段 commit 收尾) |
| `agent` | Pydantic-AI Agent + MockLLM |
| `release` | reserve → RUN_ABORTED → release(全額退回) |
| `ttl_sweep` | reserve(TTL=5s)→ sweeper 自動 release |
| **`deny`** | **Phase 3 wedge:contract evaluator STOP** |
| `agent_real` | 真的打 OpenAI gpt-4o-mini(需要 `OPENAI_API_KEY`) |
| `agent_real_anthropic` | 真的打 Claude haiku-4-5 |
| `agent_real_langchain` | LangChain ChatOpenAI |
| `agent_real_langgraph` | LangGraph ReAct agent |
| `multi_provider_usd` | 跨 provider 的 USD 預算淨額結算 |

```bash
# 真實 OpenAI:
export OPENAI_API_KEY=sk-...
DEMO_MODE=agent_real make demo-up

# 跨 provider USD:
export OPENAI_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-...
DEMO_MODE=multi_provider_usd make demo-up
```

## 3. 打開 dashboard

```bash
docker compose up -d dashboard
open http://localhost:8090
```

跳出提示時輸入 bearer token(`demo-dashboard-token-replace-in-production`)。
你會看到預算總覽、最近的決策、DENY 統計,以及 outbox forwarder 的健康狀態。

## 4. 透過 control plane 開一個新 tenant

```bash
docker compose up -d control-plane

curl -X POST http://localhost:8091/v1/tenants \
  -H 'Authorization: Bearer demo-admin-token-replace-in-production' \
  -H 'Content-Type: application/json' \
  -d '{"name": "acme-corp", "opening_deposit_atomic": "1000"}'
```

回應裡會有 `sidecar_config_env` —— 一整塊環境變數,你可以直接塞進某個 sidecar 的
deployment 裡。

## 下一步

- [Authoring contracts](contracts/yaml.md) —— 寫你自己的規則
- [Pydantic-AI 整合](integrations/pydantic-ai.md) —— 把 SDK 接進你的 agent
- [Helm 部署](deployment/helm.md) —— 搬到 k8s
