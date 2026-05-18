# dev.to / Medium deep-dive draft — Agentic SpendGuard

> **Status**: draft, not yet published. Per `docs/seo-plan.md` §1 Lever 4 + `memory/project_overview.md`, this goes out **1–2 weeks after the HN post**, not before. The HN post is the trigger; this is the long-tail follow-up.
>
> **Two platforms, same source**:
> - `dev.to` — heavier on code blocks, copy-paste-able snippets, reader's-in-a-tab framing.
> - `Medium` — lighter on code, heavier on narrative + diagrams. Pull the headers + intro from below; trim code to ~50% density.
>
> **Target word count**: 1200–1800 (dev.to sweet spot is 1200; Medium tolerates 1800).
>
> **Cross-post timing**: dev.to first, Medium 24h later (avoids the "self-plagiarism" SEO penalty if both index at the same time).

---

## Title options

1. **`How to hard-cap your LLM agent's bill — with 1 environment variable`**  *(action-oriented, concrete; lead candidate for dev.to)*
2. **`Stripe-style auth/capture for LLM token spend`**  *(metaphor-oriented; lead candidate for Medium)*
3. **`The 3 AM agent loop that burned $400 — and the runtime gate that would have stopped it`**  *(story-shaped; reserve for r/programming cross-post)*

## Subtitle

A walkthrough of building an audit-chained, fail-closed budget gate that sits in the egress path of any OpenAI-compatible agent — without code changes.

---

## Body

### The 3 AM problem

A Pydantic-AI agent I was running hit a transient tool error overnight and retried the same `gpt-4o` call about 30 times before the morning cron-job alert fired. The bill landed in the OpenAI dashboard six hours after the calls actually went out. By the time the alert fired, the money was already spent.

The standard pattern for "controlling LLM spend" is *reconciliation*: scrape usage from the provider dashboard, send alerts when a budget threshold trips, hope someone is awake to kill the process. That's not control — it's an autopsy. The provider already shipped the bill.

I wanted something different: a gate **in the request path** that refuses calls which would breach a budget **before** the request reaches OpenAI.

### What "in the request path" actually means

There are a few places you could put such a gate:

| Position | Trade-off |
|---|---|
| Inside your agent code | Every framework needs its own hook. You'd ship a wrapper for Pydantic-AI, LangChain, openai-agents, raw `openai-python`, etc. |
| HTTP egress proxy on `localhost` | One implementation; works for any OpenAI-compatible client by changing `OPENAI_BASE_URL`. |
| Kubernetes NetworkPolicy at the cluster edge | True L2 enforcement, no agent code can bypass. But also opaque and operator-side. |

[Agentic SpendGuard](https://github.com/m24927605/agentic-spendguard) is the middle option, with a hook for the third. The setup looks like this:

```bash
git clone https://github.com/m24927605/agentic-spendguard
cd agentic-spendguard
make demo-up DEMO_MODE=proxy
export OPENAI_BASE_URL=http://localhost:9000/v1
```

Then any code that constructs an OpenAI client — `openai-python`, LangChain `ChatOpenAI`, openai-agents `Agent(model="…")` — is gated. Nothing else changes.

### What "fail-closed" looks like in code

Send a request that would breach the configured budget cap, and the proxy returns `HTTP 429` **before** the upstream OpenAI request fires:

```python
from openai import OpenAI
client = OpenAI(base_url="http://localhost:9000/v1")

try:
    client.chat.completions.create(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "..."}],
    )
except APIError as e:
    if e.code == "spendguard_blocked":
        reasons = e.body["error"]["details"]["reason_codes"]
        # e.g. ["BUDGET_EXHAUSTED"] — your agent can branch here.
```

Critically, no token spend happened. The proxy held a reservation against the configured budget BEFORE forwarding, and returned 429 the moment the contract evaluator decided "STOP". The upstream OpenAI HTTP request never went out.

### The architecture in three layers

1. **Egress proxy** (Rust + axum). Forwards `POST /v1/chat/completions` and `POST /v1/responses` to OpenAI byte-identically on the success path. On budget breach, returns 429 with structured `reason_codes` the agent can branch on.

2. **Sidecar** (Rust + tonic over a Unix Domain Socket). Holds an auth/capture ledger backed by Postgres. The proxy calls `RequestDecision` for every LLM call; the sidecar returns Continue / Stop / RequireApproval / Degrade.

3. **Audit chain** (Postgres with DB-enforced immutability triggers + Ed25519/KMS-ECDSA-P256 signed events). Every decision is signed and appended. The chain is tamper-evident: a Postgres trigger refuses any UPDATE/DELETE on the audit tables.

```
agent code  ──HTTP──▶  egress-proxy  ──UDS gRPC──▶  sidecar  ──TLS gRPC──▶  ledger (Postgres)
                            │                                                       │
                            └─── byte-identical forward to OpenAI on Continue       │
                                                                                    ▼
                                                              audit_outbox (signed, immutable)
                                                                                    │
                                                                                    ▼
                                                              outbox-forwarder ─▶ canonical_events
                                                                                    │
                                                                                    ▼
                                                              your SIEM / downstream ETL
```

### What "Stripe-style auth/capture" buys you over post-hoc telemetry

Helicone / Portkey / LiteLLM are good observability + routing layers. They sit in the same egress position and they ship great traces. What they don't do, fundamentally, is **fail-close on a budget breach**. Their decision model is "log + alert" — same reconciliation problem the dashboard has, just faster.

SpendGuard's decision model is **auth/capture**:

- **PRE call**: reserve `N` tokens at the current PricingFreeze. If reservation would cross the cap, return STOP immediately. No upstream call.
- **POST call**: commit the actual `usage.total_tokens` from OpenAI's response. The reservation is replaced with the real spend. If commit fails (sidecar crash, network blip), the reservation TTL-sweeps and releases — no double-charge.

This is structurally what Stripe does with card authorizations + captures. The reservation IS the gate; the gate fires BEFORE the operation; the audit chain captures both the auth and the capture, immutably.

### The honest limitations (because every "production-ready" claim is suspect until you read the caveats)

- **Phase 1 ledger**: `single_writer_per_budget` only. Multi-region writers come in Phase 2.
- **Operator-supplied PKI**: the chart doesn't bundle Postgres or cert-manager. Production deployments BYO.
- **Streaming SSE**: pass-through works for `POST /v1/chat/completions` and `POST /v1/responses` (verified e2e against real `gpt-4o-mini`). Anthropic / Bedrock native streaming defer to future spec.
- **Reservation TTL is 60s by default**: long-running streams (>60s) require either bumping the TTL or accepting a release + re-reserve mid-stream. Tracked.

### What to do next

If you want to play with it locally:

```bash
git clone https://github.com/m24927605/agentic-spendguard
make demo-up DEMO_MODE=proxy   # boots postgres + sidecar + proxy
export OPENAI_BASE_URL=http://localhost:9000/v1
# Then run your agent. CONTINUE = forwarded. STOP = 429 fail-closed.
```

If you want to reproduce the head-to-head benchmark vs AgentGuard / AgentBudget:

```bash
make benchmark   # 30 seconds, mock OpenAI, no real spend
```

If you want to read the spec rather than the marketing copy, the proxy's full design is in [`docs/specs/auto-instrument-egress-proxy-spec.md`](https://github.com/m24927605/agentic-spendguard/blob/main/docs/specs/auto-instrument-egress-proxy-spec.md). Spec v7 is LOCKED.

---

### Discussion prompts (for the comment section)

If commenters drift the thread productively, follow with:

- *"Why Rust for the sidecar?"* — zero-GC in the hot path; tonic gRPC + axum compose cleanly; the team had ~6 months of existing Rust ledger code.
- *"How does the audit chain prevent tampering?"* — Postgres immutability triggers on `audit_outbox` + Ed25519/KMS-ECDSA-P256 signed events + outbox-forwarder verifies signatures at ingest time. Any UPDATE/DELETE on audit tables fails at the DB layer.
- *"What about agents that import the OpenAI client directly?"* — L1 (semantic_adapter) blocks via the SDK. L2 (egress_proxy_hard_block, what this post is about) blocks at the HTTP layer. L3 (provider_key_gateway, future) keeps the API key entirely server-side. Pick the trust model you need.

---

### Cross-post checklist

- [ ] dev.to: tags `#llm`, `#agents`, `#opensource`, `#rust`
- [ ] Medium: import the dev.to draft, swap headers to Medium's H1/H2 conventions
- [ ] r/programming: lead with title #3 (the 3 AM story)
- [ ] r/MachineLearning: frame as "model-cost control" — drop the "agent" word from the title; gain technical-ML eyes
- [ ] Link back to the HN post in the closing paragraph for the long-tail SEO juice
