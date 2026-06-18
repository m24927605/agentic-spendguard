<div align="center">

# 🛡️ Agentic SpendGuard

**The spend firewall for LLM agents.**

Reserve budget *before* the provider is called — refuse the call the moment the
budget is gone, with a signed audit trail of why. Not another dashboard that
shows you the bill after it lands.

[![License: Apache 2.0](https://img.shields.io/badge/license-Apache%202.0-blue)](LICENSE)
[![PyPI: spendguard-sdk](https://img.shields.io/pypi/v/spendguard-sdk?label=pypi)](https://pypi.org/project/spendguard-sdk/)
[![Rust](https://img.shields.io/badge/core-Rust-orange)](services/)
[![Postgres 16 ledger](https://img.shields.io/badge/ledger-Postgres%2016-336791)](services/ledger/migrations/)
[![wire: mTLS gRPC](https://img.shields.io/badge/wire-mTLS%20gRPC-purple)](proto/)
[![PRs welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](CONTRIBUTING.md)

[Quick start](#-quick-start) · [How it works](#%EF%B8%8F-how-it-works) · [Benchmark](#-benchmark) · [Integrations](docs/integrations.md) · [Architecture](ARCHITECTURE.md)

</div>

---

## Why

A support agent hits a rate-limited tool at 2:47am. The retry loop re-plans,
re-prompts, re-tries — each retry a fresh `gpt-4o` call with the full context.
Forty minutes later one stuck conversation has burned ~$380. You find out when
the provider dashboard updates the next morning.

"Track usage and send alerts" is **reconciliation** — you see the bill after it
lands. SpendGuard is **control**: every request reserves budget against a
per-tenant ledger *before* the provider is called. Budget gone → the call is
refused and the provider is never hit.

If you've used Stripe: this is **auth/capture, applied to LLM tokens.** Reserve
the estimated cost pre-call; capture the real `usage` post-call. Idempotent,
atomic, fail-closed.

## 🚀 Quick start

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard
export OPENAI_API_KEY=sk-...
make demo-up DEMO_MODE=proxy
```

This brings up Postgres + ledger + sidecar + egress proxy and runs a real
`gpt-4o-mini` call through it. **Your application code changes by one line:**

```python
from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:9000/v1",   # ← only change
    api_key=os.environ["OPENAI_API_KEY"],
)
client.chat.completions.create(model="gpt-4o-mini", messages=[...])
```

| Decision | HTTP | Result |
|---|---|---|
| **CONTINUE** (budget available) | 200 | Provider response byte-identical; ledger writes a `commit_estimated` audit row |
| **STOP** (over hard-cap) | 429 + `Retry-After` | Structured `spendguard_blocked` body — **the request never reaches the provider** |

## 📊 Benchmark

Identical fixture — 100 attempted calls, $1.00 budget, $0.18/call — through
three drop-in budget tools, measured against a ground-truth pricing table
(`make benchmark`):

| Runner | Wire calls | $ spent | Overshoot |
|---|---:|---:|---:|
| **Agentic SpendGuard** | 5 | $0.90 | **−10%** ✅ |
| `agentbudget` | 6 | $1.08 | +8% (enforces *post*-call) |
| `agent-guard` | 100 | $18.00 | **+1700%** ❌ (no-ops on a self-hosted base URL) |

SpendGuard does **pre-call reservation** against the ledger and refuses call #6
before it leaves the runner. Reproducible in
[`benchmarks/runaway-loop/`](benchmarks/runaway-loop/).

## 🛡️ How it works

Three layers. The proxy is the only thing your client talks to.

```
agent ──HTTP──▶ egress-proxy ──UDS gRPC──▶ sidecar ──mTLS gRPC──▶ ledger
                     │                                              │
                     └── byte-identical forward on CONTINUE         ▼
                                              audit_outbox (signed, append-only)
                                                                    │
                                              outbox-forwarder ─▶ canonical_ingest ─▶ your SIEM
```

1. **Egress proxy** (Rust + axum) — speaks the OpenAI Chat Completions /
   Responses wire protocol; forwards byte-identically on `CONTINUE`, returns
   HTTP 429 on `STOP` without ever calling the provider.
2. **Sidecar** (Rust + tonic over UDS) — per-pod Contract DSL evaluator;
   decides `Continue` / `Stop` / `RequireApproval` / `Degrade`; signs every
   decision (Ed25519 or AWS KMS ECDSA P-256).
3. **Ledger + audit chain** (Postgres) — append-only double-entry ledger; the
   **hard cap is enforced in the ledger itself** (`BUDGET_EXHAUSTED`).
   `audit_outbox` rows are immutable (DB triggers) and signed — tamper-evident.

→ Full details in **[ARCHITECTURE.md](ARCHITECTURE.md)**.

## 🎚️ Capability levels

Pick the trust model that fits how much the agent's code can be trusted not to
bypass the gate.

| Level | Mechanism | Residual bypass |
|---|---|---|
| **L0** advisory_sdk | SDK logs decisions; never blocks | code that skips the SDK |
| **L1** semantic_adapter | SDK refuses the upstream call on `STOP` | importing the provider client directly |
| **L2** egress_proxy_hard_block | network proxy rejects un-gated egress (+ NetworkPolicy) | none |
| **L3** provider_key_gateway | provider keys live server-side; agent never sees them | none |

## 🔌 Integrations

Two ways in: a **drop-in proxy** (one env var, no code change) or a
**framework adapter** (wrap the model object). Highlights:

- **Proxies / gateways:** LiteLLM (proxy, guardrail, SDK shim), Kong AI
  Gateway, Envoy ext_proc, any OpenAI-compatible base URL (AnythingLLM,
  LobeChat, OpenClaw, Coze, …).
- **Python frameworks:** OpenAI Agents, LangChain / LangGraph, Pydantic-AI,
  Google ADK, AWS Strands, DSPy, Agno, AutoGen / AG2, LlamaIndex, CrewAI,
  SmolAgents, Letta.
- **TS / JS:** Vercel AI SDK, Mastra, LangChain.js, OpenAI Agents (TS), Inngest
  AgentKit, n8n, Flowise, Botpress.
- **.NET + Python:** Microsoft Agent Framework, Microsoft Agent Governance
  Toolkit ([merged upstream](https://github.com/microsoft/agent-governance-toolkit/pull/2398)).

**→ Full matrix, install snippets, and demo gates: [docs/integrations.md](docs/integrations.md).**

## 📦 SDK

For approval workflows, model degradation, or multi-budget claims, use the SDK
directly:

```bash
pip install spendguard-sdk          # Python
npm install @spendguard/sdk         # TypeScript
```

```python
async with SpendGuardClient(socket_path="/var/run/spendguard/adapter.sock",
                            tenant_id=TENANT) as sg:
    await sg.handshake()
    outcome = await sg.request_decision(trigger="LLM_CALL_PRE", ...)
    # CONTINUE → make the call; DecisionStopped / ApprovalRequired raise.
```

## 🚀 Deploy

- **Local / demo:** [`deploy/demo/compose.yaml`](deploy/demo/compose.yaml) —
  full stack with PKI bootstrap and internal mTLS (`make demo-up`).
- **Kubernetes:** [`charts/spendguard/`](charts/spendguard/) — DaemonSet
  sidecar + core Deployments; `chart.profile=production` enforces required-input
  gates at render time.

## 📚 Documentation

- [**Architecture**](ARCHITECTURE.md) — components, data model, invariants.
- [**Integrations**](docs/integrations.md) — full adapter matrix + demo modes.
- [**Specs**](docs/specs/) — authoritative, versioned source of truth
  ([ledger](docs/ledger-storage-spec-v1alpha1.md),
  [contract DSL](docs/contract-dsl-spec-v1alpha1.md),
  [trace schema](docs/trace-schema-spec-v1alpha1.md),
  [sidecar](docs/sidecar-architecture-spec-v1alpha1.md)).
- [**Contributing**](CONTRIBUTING.md) · [**Security**](SECURITY.md) · [**Code of Conduct**](CODE_OF_CONDUCT.md)

## Status

Single-maintainer, Apache-2.0, **Beta**. Strong demo coverage (30+ integrations,
each with a green `DEMO_MODE` gate) and a signed, tamper-evident audit chain;
limited production usage so far. The wire spec and audit invariants are
append-only — **open an issue before touching `proto/` or `migrations/`.** PRs
welcome.

## License

[Apache 2.0](LICENSE).

SpendGuard vendors tokenizer assets for predictor validation. The Llama
tokenizer path uses Meta Llama 3.1-derived files and is *Built with Llama*; see
[`crates/spendguard-tokenizer/LICENSE_NOTICES.md`](crates/spendguard-tokenizer/LICENSE_NOTICES.md)
for attribution and Meta Llama 3.1 Acceptable Use Policy obligations before
redistributing or enabling that path.
