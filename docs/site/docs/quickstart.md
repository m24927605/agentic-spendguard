---
description: >-
  Run the full Agentic SpendGuard stack locally in five minutes — Docker
  Compose brings up the sidecar, ledger, canonical-ingest, dashboard, and
  a Pydantic-AI agent demo, with a working DENY decision out of the box.
---

# Quickstart

From `git clone` to a green DENY demo in ≈5 minutes.

## Prerequisites

- Docker Desktop (or any Docker engine + Compose v2)
- ~4 GiB free RAM (Rust services build in parallel)
- 5 minutes for first build (cached after that)

## 1. Clone and bring up the stack

```bash
git clone https://github.com/m24927605/agentic-flow-cost-evaluation
cd agentic-flow-cost-evaluation/deploy/demo

docker compose down -v --remove-orphans   # clean state
DEMO_MODE=deny make demo-up
```

Expected output (last lines):

```text
[demo] DENY raised decision_id=... reason_codes=['BUDGET_EXHAUSTED']
       matched_rule_ids=['{contract}:{budget}:hard-cap-deny']
[demo] DENY assertions PASS
```

What just happened:

1. Postgres started + ran ledger + canonical migrations
2. `pricing-seed-init` loaded `deploy/demo/init/pricing/seed.yaml` into
   `pricing_table` (35 rows, 5 providers)
3. `bundles-init` baked a real `contract.yaml` into the demo bundle
4. `pki-init` minted a CA + per-service mTLS certs
5. `ledger`, `canonical-ingest`, `sidecar` etc came up
6. `demo` container connected via UDS, sent a $2000 budget claim
7. Sidecar's contract evaluator matched `hard-cap-deny` rule (limit
   $1000) → adapter raised `DecisionStopped`
8. Carrier `denied_decision` row + `audit_outbox` row written
9. `outbox-forwarder` pushed the audit row to `canonical_events`

## 2. Run the other demo modes

| Mode | What it shows |
|---|---|
| `decision` | Full reserve→commit→provider_report lifecycle |
| `invoice` | + invoice_reconcile (3-stage commit close) |
| `agent` | Pydantic-AI Agent + MockLLM |
| `release` | reserve → RUN_ABORTED → release (full refund) |
| `ttl_sweep` | reserve(TTL=5s) → sweeper auto-release |
| **`deny`** | **Phase 3 wedge: contract evaluator STOP** |
| `agent_real` | Real OpenAI gpt-4o-mini (needs `OPENAI_API_KEY`) |
| `agent_real_anthropic` | Real Claude haiku-4-5 |
| `agent_real_langchain` | LangChain ChatOpenAI |
| `agent_real_langgraph` | LangGraph ReAct agent |
| `multi_provider_usd` | Cross-provider USD budget netting |

```bash
# Real OpenAI:
export OPENAI_API_KEY=sk-...
DEMO_MODE=agent_real make demo-up

# Cross-provider USD:
export OPENAI_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-...
DEMO_MODE=multi_provider_usd make demo-up
```

## 3. Open the dashboard

```bash
docker compose up -d dashboard
open http://localhost:8090
```

Enter the bearer token (`demo-dashboard-token-replace-in-production`)
when prompted. You'll see budget overview, recent decisions, DENY
stats, and outbox forwarder health.

## 4. Provision a fresh tenant via control plane

```bash
docker compose up -d control-plane

curl -X POST http://localhost:8091/v1/tenants \
  -H 'Authorization: Bearer demo-admin-token-replace-in-production' \
  -H 'Content-Type: application/json' \
  -d '{"name": "acme-corp", "opening_deposit_atomic": "1000"}'
```

The response includes `sidecar_config_env` — a block of env vars you
can drop straight into a sidecar's deployment.

## Next

- [Authoring contracts](contracts/yaml.md) — write your own rules
- [Pydantic-AI integration](integrations/pydantic-ai.md) — wire the SDK
  into your agent
- [Helm deployment](deployment/helm.md) — move to k8s
