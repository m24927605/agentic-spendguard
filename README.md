<div align="center">

# 🛡️ Agentic SpendGuard

**The spend firewall for LLM agents.**

Stops runaway agents *before* the provider is called — not after the
invoice arrives the next morning. Budget reserved per-call, signed
audit trail, p50 ≤10ms decision overhead ([Contract §14 SLO](docs/specs/contract-dsl-spec-v1alpha1.md)).
Works with **LiteLLM proxy**, **OpenAI Agents SDK**, **LangGraph**,
**LangChain**, **Pydantic-AI**, and **Microsoft Agent Governance
Toolkit** ([community integration merged upstream](https://github.com/microsoft/agent-governance-toolkit/pull/2398)).

[![License: Apache 2.0](https://img.shields.io/badge/license-Apache%202.0-blue)](LICENSE)
[![PyPI: spendguard-sdk](https://img.shields.io/pypi/v/spendguard-sdk?label=pypi)](https://pypi.org/project/spendguard-sdk/)
[![Built with Rust 1.91](https://img.shields.io/badge/rust-1.91-orange)](deploy/demo/runtime/Dockerfile.ledger)
[![Postgres 15+ ledger](https://img.shields.io/badge/postgres-15%2B-336791)](services/ledger/migrations/)
[![mTLS gRPC](https://img.shields.io/badge/wire-mTLS%20gRPC-purple)](proto/)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](https://github.com/m24927605/agentic-spendguard/issues)

</div>

```bash
pip install 'spendguard-sdk[litellm]'
```

→ [90-second demo](#-quick-start-30-seconds) · [Microsoft AGT integration](https://github.com/microsoft/agent-governance-toolkit/blob/main/docs/integrations/spendguard-integration.md) · [Architecture](#%EF%B8%8F-how-it-works)

---

## 💡 Why this exists

Picture the failure mode SpendGuard is built to stop:

A customer-support agent hits a rate-limited tool at 2:47am. The retry
policy kicks in. The agent loop re-plans, re-prompts, re-tries — each
retry a fresh `gpt-4o` call with the full conversation in context.
Forty minutes later, one stuck conversation has consumed ~$380 in
tokens. Multiply across the other tenants doing the same during the
incident.

The post-mortem starts with *"we didn't know until the OpenAI dashboard
updated the next morning."*

**SpendGuard moves detection from tomorrow to the 11th call.** Every
request reserves tokens against a per-tenant budget before the provider
is called. Budget exhausted → the call is refused with a signed audit
row of why (HTTP 429 from the egress proxy; HTTP 403 from the LiteLLM
callback — see [adapter integrations](#-adapter-integrations) for which path your
client takes). The provider is never hit.

The standard answer — *"track usage, send alerts"* — is reconciliation,
not control. You see the bill **after** it lands. SpendGuard inverts
this: if the agent isn't allowed to spend that much on that model under
that tenant right now, the LLM call never happens.

---

## 🚀 Quick start (30 seconds)

```bash
git clone git@github.com:m24927605/agentic-spendguard.git
cd agentic-spendguard
export OPENAI_API_KEY=sk-...
make demo-up DEMO_MODE=proxy
```

That spins up Postgres + ledger + sidecar + the egress proxy, then runs a real `gpt-4o-mini` call through it. **Your application code stays unchanged:**

```python
from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:9000/v1",   # ← only change
    api_key=os.environ["OPENAI_API_KEY"],
)
client.chat.completions.create(model="gpt-4o-mini", messages=[...])
```

| Decision | HTTP | Body |
|---|---|---|
| **CONTINUE** (budget available) | 200 | OpenAI's response byte-identical; ledger writes a `commit_estimated` audit row |
| **STOP** (over hard-cap) | 429 + `Retry-After: 86400` | `{"error":{"code":"spendguard_blocked","details":{"reason_codes":["BUDGET_EXHAUSTED"],...}}}` — **the HTTP request never reaches OpenAI** |

If you've integrated Stripe before: this is **auth/capture, applied to LLM tokens**. PRE the call, the proxy reserves the cost against a configured budget; POST the call, it captures the real `usage.total_tokens`. Idempotent, atomic, fail-closed.

---

## 📊 Head-to-head benchmark

Identical fixture — 100 attempted calls, $1.00 budget, $0.18 per call — through three drop-in budget tools, reporting ground-truth `$` spent against a centralized pricing table.

```text
$ make benchmark
```

| Runner | Budget | Wire calls | $ spent | Overshoot |
|---|---|---:|---:|---:|
| **Agentic SpendGuard** | $1.00 | 5 | $0.90 | **−10.0%** ✅ |
| `agentbudget` | $1.00 | 6 | $1.08 | +8.0% |
| `agent-guard` | $1.00 | 100 | $18.00 | **+1700%** ❌ |

- **`agentbudget`** overshoots by one call because enforcement is **post-call** (the 6th call completes on the wire, *then* it raises `BudgetExhausted`).
- **`agent-guard`** doesn't enforce at all because its HTTP-level interception is hardcoded to `openai.com` / `anthropic.com` and silently no-ops the moment you point an OpenAI client at a self-hosted base URL.
- **Agentic SpendGuard** does **pre-call reservation** against a ledger and refuses call #6 before it leaves the runner.

Reproducible benchmark in [`benchmarks/runaway-loop/`](benchmarks/runaway-loop/). Full results in [RESULTS.md](benchmarks/runaway-loop/RESULTS.md).

### Predictor-upgrade benchmark (SLICE_15)

The predictor upgrade adds a concurrent-burst benchmark comparing decision-time latency + overshoot against LiteLLM proxy at 1 / 10 / 100 concurrent calls. SpendGuard with the SLICE_06 output_predictor + SLICE_09 run_cost_projector tracks p99 < 50ms (Contract DSL §14 SLO) and overshoot below LiteLLM at every burst level.

```bash
# Bring up demo + run the burst harness:
bash tests/e2e/predictor_upgrade.sh
cd benchmarks/predictor-upgrade && cargo build --release
./target/release/predictor-upgrade-bench --bursts 1,10,100 --output ./out
```

| Burst | SpendGuard p99 | LiteLLM p99 | SpendGuard overshoot | LiteLLM overshoot |
|---:|---:|---:|---:|---:|
| 1   | _populated by run_ | _populated by run_ | _populated by run_ | _populated by run_ |
| 10  | _populated by run_ | _populated by run_ | _populated by run_ | _populated by run_ |
| 100 | _populated by run_ | _populated by run_ | _populated by run_ | _populated by run_ |

Latest CI numbers + reproduction details: [`benchmarks/predictor-upgrade/RESULTS.md`](benchmarks/predictor-upgrade/RESULTS.md). Calibration accuracy on a synthetic 1000-call workload: [`benchmarks/predictor-upgrade/calibration_synthetic.py`](benchmarks/predictor-upgrade/calibration_synthetic.py) (slice §8.3 asserts SpendGuard P95 |predicted − actual| / actual ≤ 5%). Portkey: documented N/A — closed-source proxy not benchmark-able from the open repo. Spec set locked on the SLICE_15 merge.

---

## 🧰 What works today

The 1-env-var claim is verified **end-to-end against real OpenAI** for:

| Client | Status | What you change |
|---|:---:|---|
| 🐍 `openai-python` (`from openai import OpenAI`) | ✅ | `base_url=...` |
| 🦜 LangChain `ChatOpenAI` | ✅ | `base_url=...` |
| 🕸️ LangGraph (via `ChatOpenAI`) | ✅ | `base_url=...` |
| 🤖 openai-agents shorthand `Agent(model="...")` | ✅ | `OPENAI_BASE_URL=...` |
| 🌊 Streaming (`stream:true`) on both endpoints | ✅ | (transparent) |

For approval workflows, model-tier degradation, and multi-budget claims that the proxy doesn't yet cover, there's an [SDK wrapper-mode path](#-sdk-advanced-wrapper-mode) below.

Specs:
- Auto-instrument proxy: [`docs/specs/auto-instrument-egress-proxy-spec.md`](docs/specs/auto-instrument-egress-proxy-spec.md) (v7 LOCKED)
- v0.2 streaming SSE: [`docs/specs/egress-proxy-v0.2-streaming-sse.md`](docs/specs/egress-proxy-v0.2-streaming-sse.md)
- v0.3 `/v1/responses` (openai-agents default): [`docs/specs/egress-proxy-v0.3-responses-api.md`](docs/specs/egress-proxy-v0.3-responses-api.md)

---

## 🛡️ How it works

Three layers. The proxy is the thing your client talks to. The other two are infrastructure.

### 1. Egress proxy (Rust + axum)
- Forwards `POST /v1/chat/completions` and `POST /v1/responses` to OpenAI byte-identically on the success path.
- On budget breach: returns **HTTP 429** with a structured `spendguard_blocked` body the client can branch on. **The upstream OpenAI request never fires.**
- Streaming variant: tees the SSE stream to the client byte-identical while side-parsing usage for the commit lane.

### 2. Sidecar (Rust + tonic over UDS)
- Per-pod. Holds a contract DSL evaluator + the gRPC client to the ledger.
- Decides `Continue` / `Stop` / `RequireApproval` / `Degrade` for every LLM call.
- Signs every decision with Ed25519 or AWS KMS ECDSA P-256.

### 3. Audit chain (Postgres + signed CloudEvents)
- Every reservation, commit, release, and denied decision is an immutable row in `audit_outbox`.
- DB-enforced triggers refuse `UPDATE` / `DELETE`. The chain is **tamper-evident**.
- An outbox forwarder closes the loop into `canonical_events`, downstream ETL / SIEM consumers can subscribe.

```
agent  ──HTTP──▶  egress-proxy  ──UDS gRPC──▶  sidecar  ──TLS gRPC──▶  ledger
                       │                                                  │
                       └── byte-identical forward to OpenAI on Continue   │
                                                                          ▼
                                                       audit_outbox (signed, immutable)
                                                                          │
                                                                          ▼
                                                       outbox-forwarder ─▶ canonical_events
                                                                          │
                                                                          ▼
                                                              your SIEM / data lake
```

---

## 🎚️ Capability levels (L0–L3)

Pick the trust model that fits how much your agent's code can be trusted not to bypass the gate.

| Level | What it does | Where the agent can cheat |
|---|---|---|
| **L0** advisory_sdk | SDK logs decisions to sidecar; never blocks | Agent code that bypasses the SDK |
| **L1** semantic_adapter | SDK refuses the upstream call on STOP | Agent that imports the LLM client directly |
| **L2** egress_proxy_hard_block | Network egress proxy rejects un-gated traffic | (none — agent must use the proxy) |
| **L3** provider_key_gateway | Provider API keys live in a gateway; agent never sees them | (none — provider rotates keys) |

POC default is **L3** (recommended for any pilot that runs against a real provider key); lower levels exist for backward-compat with older adapters.

---

## 📦 SDK (advanced wrapper-mode)

For agents that need `REQUIRE_APPROVAL` / `DEGRADE` decisions, multi-budget claims, or custom claim estimators, install the Python SDK:

```bash
pip install --pre spendguard-sdk

# or with a framework integration:
pip install --pre 'spendguard-sdk[pydantic-ai]'
pip install --pre 'spendguard-sdk[langchain]'
pip install --pre 'spendguard-sdk[langgraph]'
pip install --pre 'spendguard-sdk[openai-agents]'
pip install --pre 'spendguard-sdk[agt]'
```

```python
from spendguard import SpendGuardClient, ApprovalRequired, DecisionStopped

async with SpendGuardClient(
    socket_path="/var/run/spendguard/adapter.sock",
    tenant_id=TENANT,
) as sg:
    await sg.handshake()
    try:
        outcome = await sg.request_decision(
            trigger="LLM_CALL_PRE",
            run_id=run_id, step_id=step_id, llm_call_id=call_id,
            decision_id=decision_id, route="llm.call",
            projected_claims=[claim],
            idempotency_key=derive_idempotency_key(...),
        )
        # OK to make the LLM call. outcome.reservation_ids holds the auth.
    except DecisionStopped as e:
        raise
    except ApprovalRequired as e:
        resume_outcome = await e.resume(sg)  # waits for operator
```

| Framework | Module | What gets gated | Runnable example |
|---|---|---|---|
| **Pydantic-AI** | `spendguard.integrations.pydantic_ai` | Every `Model.request()` | — |
| **LangChain** | `spendguard.integrations.langchain` | Every `BaseChatModel` invocation | — |
| **LangGraph** | same module | Same wrapper (LangGraph builds on `BaseChatModel`) | — |
| **OpenAI Agents SDK** | `spendguard.integrations.openai_agents` | Every model call inside an `Agent` run | [`examples/openai-agents-composite/`](examples/openai-agents-composite/) |
| **Microsoft AGT** | `spendguard.integrations.agt` | AGT's PolicyEngine + SpendGuard as a policy plugin | [`microsoft/agent-governance-toolkit#2398`](https://github.com/microsoft/agent-governance-toolkit/pull/2398) |
| **LiteLLM proxy** | `spendguard.integrations.litellm` | Every `/v1/chat/completions` through the LiteLLM proxy | [`docs/specs/litellm-integration/PROXY_RECIPE.md`](docs/specs/litellm-integration/PROXY_RECIPE.md) |

---

## 🌐 Other demo modes

```bash
make demo-up DEMO_MODE=decision               # CONTINUE flow
make demo-up DEMO_MODE=deny                   # hard-cap → STOP
make demo-up DEMO_MODE=approval               # REQUIRE_APPROVAL → resume()
make demo-up DEMO_MODE=ttl_sweep              # reservation TTL release
make demo-up DEMO_MODE=agent_real             # real OpenAI via Pydantic-AI
make demo-up DEMO_MODE=agent_real_anthropic   # real Anthropic
make demo-up DEMO_MODE=agent_real_langgraph   # LangGraph
make demo-up DEMO_MODE=agent_real_openai_agents          # OpenAI Agents SDK (wrapper)
make demo-up DEMO_MODE=agent_real_openai_agents_proxy    # openai-agents via proxy ⭐
make demo-up DEMO_MODE=litellm_real           # LiteLLM proxy: ALLOW+DENY+STREAM+MULTI-TEAM ⭐
make demo-up DEMO_MODE=litellm_deny           # LiteLLM proxy: 3 fail-closed sub-steps
make demo-up DEMO_MODE=approval_hot_reload    # frozen-pricing regression
make demo-up DEMO_MODE=multi_provider_usd     # multi-provider USD normalization
```

`make demo-up` (no flag) spins up the full wrapper-mode stack including the dashboard at `http://localhost:8090`.

---

## ❓ FAQ

<details>
<summary><b>How does this compare to Helicone / Portkey / LiteLLM?</b></summary>

Those proxy your traffic too, but their decision model is **observability**: log the call, then alert / route / retry. SpendGuard's decision model is **auth/capture**: reserve PRE the call, fail-closed on overrun, commit POST. The audit chain isn't a log — it's a tamper-evident ledger backed by Postgres immutability triggers + KMS-signed CloudEvents.

If you only need a per-key dollar cap on a gateway, Portkey or LiteLLM is simpler. SpendGuard is for anyone who needs the LLM call **refused** the moment the budget is gone — whether that's a 1-person SaaS protecting a free tier, or a platform team that also has to hand evidence to compliance after the bill lands.
</details>

<details>
<summary><b>What about latency?</b></summary>

The proxy adds one UDS gRPC roundtrip to the sidecar PRE the call (~1–3ms on the same pod) + one async EmitTraceEvents POST the call (doesn't block the response). The audit-chain write is async via outbox.
</details>

<details>
<summary><b>Does the agent's code need to change?</b></summary>

For the proxy path (Chat Completions + Responses API): **no**. One environment variable. The verified clients listed above all work without any code changes.

For the SDK wrapper-mode (approval workflows / model degradation): yes — but it's typically one line of "wrap the model object" inside your framework. See the integrations table above.
</details>

<details>
<summary><b>What about agents that import the OpenAI client directly and skip the proxy?</b></summary>

That's the L1 → L2 → L3 trust model. L1 (SDK wrapper) blocks via the framework. L2 (`egress_proxy_hard_block`) blocks at the HTTP layer + a Kubernetes NetworkPolicy that forbids egress except via the proxy. L3 (`provider_key_gateway`, future) keeps the provider API key entirely server-side so the agent process can't make calls at all without the gateway.
</details>

<details>
<summary><b>How does the audit chain prevent tampering?</b></summary>

Three layers: (1) `audit_outbox` table has a Postgres trigger refusing any `UPDATE` or `DELETE`; (2) every row carries an Ed25519 or KMS-ECDSA-P256 signature over a canonical hash; (3) `canonical_ingest` verifies signatures at ingest time and quarantines failed verifications. Any tampering fails at the DB layer, the signature layer, or the ingest layer.
</details>

<details>
<summary><b>What's the Phase 1 ledger constraint?</b></summary>

`single_writer_per_budget` only. A given budget can be written by exactly one workload instance at a time, enforced via fencing leases. Multi-region writers come in Phase 2.
</details>

<details>
<summary><b>Why Rust?</b></summary>

Zero-GC in the hot path (the sidecar is in the request-path for every LLM call). `tonic` + `axum` compose cleanly. The team had ~6 months of existing Rust ledger code when the proxy work started.
</details>

---

## 🔌 Service map

| Service | What it does | Port |
|---|---|---:|
| [`ledger`](services/ledger/) | Postgres-backed double-entry ledger + audit transactional outbox | 50051 |
| [`sidecar`](services/sidecar/) | Per-pod UDS gRPC server; contract evaluator; mTLS clients | (UDS) |
| [`canonical_ingest`](services/canonical_ingest/) | Per-decision_id canonical ordering + 3 storage classes | 50052 |
| [`egress_proxy`](services/egress_proxy/) | HTTP proxy for `/v1/chat/completions` + `/v1/responses` (1-env-var) | 9000 |
| [`control_plane`](services/control_plane/) | REST API for tenants / budgets / approvals | 8091 |
| [`dashboard`](services/dashboard/) | Read-only operator UI (budgets / decisions / audit export) | 8090 |
| [`outbox_forwarder`](services/outbox_forwarder/) | Closes the audit-chain loop (ledger → canonical_ingest) | — |
| [`ttl_sweeper`](services/ttl_sweeper/) | Releases expired reservations | — |
| [`webhook_receiver`](services/webhook_receiver/) | Provider HTTPS webhooks → Ledger gRPC ops (HMAC-verified) | 8443 |
| [`usage_poller`](services/usage_poller/) | OpenAI / Anthropic admin-usage API → `provider_usage_records` | — |
| [`signing`](services/signing/) | Producer signing trait (Local Ed25519 + KMS verifier) | — |

Every external surface is mTLS. Every service exposes `/metrics` (Prometheus, per-handler ok/err counters). Every audit row is signed.

---

## 🚀 Deploy

**Docker Compose (demo / local dev):** [`deploy/demo/compose.yaml`](deploy/demo/compose.yaml) — full stack with PKI bootstrap, manifest signing, mTLS internal, all on one network.

**Kubernetes (Helm):** [`charts/spendguard/`](charts/spendguard/) — DaemonSet sidecar + Deployments for ledger / canonical_ingest / control_plane / dashboard / webhook_receiver. `chart.profile=production` enforces required-input gates (bundle hashes, trust-root SPKI, real Postgres URL) at template render time. Validated end-to-end on `kind` via [`scripts/helm-validate-kind.sh`](scripts/helm-validate-kind.sh) (CI: [`.github/workflows/helm-validate.yml`](.github/workflows/helm-validate.yml)).

**Signing modes:**
- `local` — Ed25519 PKCS8 PEM mounted from K8s Secret (demo / on-prem)
- `kms` — AWS KMS-backed ECDSA P-256 via IRSA (production)
- `disabled` — empty signatures (refuses to construct outside `SPENDGUARD_PROFILE=demo`)

---

## 📚 Specs (source of truth)

Read before changing wire format or invariants:

- [`docs/agent-runtime-spend-guardrails-complete.md`](docs/agent-runtime-spend-guardrails-complete.md) — full design doc
- [`docs/trace-schema-spec-v1alpha1.md`](docs/trace-schema-spec-v1alpha1.md) — CloudEvent / audit chain
- [`docs/ledger-storage-spec-v1alpha1.md`](docs/ledger-storage-spec-v1alpha1.md) — double-entry model, idempotency, replay
- [`docs/contract-dsl-spec-v1alpha1.md`](docs/contract-dsl-spec-v1alpha1.md) — Contract DSL + decision boundary semantics
- [`docs/sidecar-architecture-spec-v1alpha1.md`](docs/sidecar-architecture-spec-v1alpha1.md) — fencing, drain, capability handshake
- [`docs/stage2-poc-topology-spec-v1alpha1.md`](docs/stage2-poc-topology-spec-v1alpha1.md) — Phase 1 SaaS topology + durability invariants

All locked at v1alpha1; schema bumps land via additive proto changes (backwards-compatible).

---

## 🤝 Contributing

**Honest status:** Dev Status 4-Beta. Single-maintainer open-source project (Apache 2.0). Solid demo coverage (8+ demo modes, all green) and a signed audit chain — but zero production users yet. PyPI 0.3.0 + Microsoft AGT integration merged 2026-05-19 are the only third-party validation signals. PRs welcome; the wire spec + audit invariants are append-only — open an issue first if you're about to touch `proto/` or `migrations/`.

---

## 📄 License

[Apache 2.0](LICENSE)

## Third-Party Tokenizer Notices

SpendGuard vendors tokenizer assets for predictor validation. The Llama
tokenizer path uses Meta Llama 3.1-derived tokenizer files and is
`Built with Llama`; review
[`crates/spendguard-tokenizer/LICENSE_NOTICES.md`](crates/spendguard-tokenizer/LICENSE_NOTICES.md)
for attribution, the 700 million monthly active users threshold measured
in the calendar month before the Llama 3.1 release date (2024-07-23),
and Meta Llama 3.1 Acceptable Use Policy obligations before
redistributing or enabling that path in a product.
