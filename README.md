<div align="center">

# 🛡️ SpendGuard

**Runtime safety rails for AI agents. Stop the bill before it lands.**

[![Status](https://img.shields.io/badge/status-Phase%205%20GA%20hardening-success)](docs/PHASE_4_VALIDATION_REPORT.md)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.91-orange)](deploy/demo/runtime/Dockerfile.ledger)
[![Postgres](https://img.shields.io/badge/postgres-15%2B-336791)](services/ledger/migrations/)
[![gRPC](https://img.shields.io/badge/wire-gRPC%20%2B%20mTLS-purple)](proto/)

</div>

---

## The problem

Your agent runs out of budget at 3 AM. By the time anyone notices, it's already retried the same `gpt-4o` call 47 times, each one charging the provider, none of them returning useful work. Or worse: an LLM-driven tool call leaks a request that costs $400 in tokens, and your post-hoc usage dashboard catches it 6 hours later.

The standard answer — "track usage, send alerts" — is reconciliation, not control. You see the bill *after* it lands.

SpendGuard inverts this. Every decision boundary an agent crosses, a sidecar evaluates first:

```
agent → SDK → sidecar(UDS gRPC) → ledger ──────► allow
                                  ↓
                                  └─► reservation (Stripe-style auth/capture)
                                  └─► STOP        (audit-chained, signed)
                                  └─► REQUIRE_APPROVAL  (paused → operator resolves)
                                  └─► DEGRADE     (mutate-then-allow)
```

If your agent isn't allowed to spend that much on that model under that contract right now — **the LLM call never happens**. The reservation is reserved. The commit clears it. Errors release it. Every step is an append-only signed audit row.

---

## Why this exists

Three pillars, picked deliberately:

| | What | What it isn't |
|---|---|---|
| **Predict** | Pre-decision boundary check: contract DSL + budget reservation BEFORE the upstream LLM call | Not a post-hoc dashboard |
| **Control** | L0 → L3 enforcement strength, fail-closed by default, KMS-signed audit chain | Not advisory rate limits |
| **Optimize** | Multi-provider token-kind normalization (OpenAI / Anthropic / Bedrock / Azure / Gemini), pricing freeze per bundle | Not a billing reconciler |

Continuous-learning auto-optimization is intentionally **out of scope** — it's a ceiling, not a moat.

---

## Quick start (30 seconds, no AWS needed)

```bash
git clone git@github.com:m24927605/agentic-spendguard.git
cd agentic-spendguard
make demo-up
```

That spins up the full stack (Postgres, sidecar, ledger, canonical_ingest, control plane, dashboard, webhook receiver, TTL sweeper, outbox forwarder) under Docker Compose and runs a Pydantic-AI agent against it. Open `http://localhost:8090` for the operator dashboard.

### Other demo modes

```bash
make demo-up DEMO_MODE=decision               # plain CONTINUE flow
make demo-up DEMO_MODE=deny                   # hard-cap → STOP
make demo-up DEMO_MODE=approval               # REQUIRE_APPROVAL → resume()
make demo-up DEMO_MODE=ttl_sweep              # reservation TTL release
make demo-up DEMO_MODE=agent_real             # real OpenAI call gated by sidecar
make demo-up DEMO_MODE=agent_real_anthropic   # real Anthropic call
make demo-up DEMO_MODE=agent_real_langgraph   # LangGraph integration
make demo-up DEMO_MODE=agent_real_openai_agents  # OpenAI Agents SDK
make demo-up DEMO_MODE=agent_real_agt         # Microsoft AGT composite (AGT + SpendGuard)
make demo-up DEMO_MODE=multi_provider_usd     # multi-provider USD normalization
```

---

## Capability levels

SpendGuard advertises **L0 → L3** enforcement strength in the adapter handshake. Pick what fits your trust model:

| Level | What it does | Where the agent can cheat |
|------:|---|---|
| **L0 advisory_sdk** | SDK logs decisions to sidecar | Agent code that bypasses the SDK |
| **L1 semantic_adapter** | SDK refuses to make the upstream call on STOP | Agent that imports the LLM client directly |
| **L2 egress_proxy_hard_block** | Network egress proxy rejects un-gated traffic | (none — agent must use the proxy) |
| **L3 provider_key_gateway** | Provider API keys live in a gateway; agent never sees them | (none — provider rotates keys) |

POC default is **L3** for first-customer pilots. Lower levels are advertised for backward-compat with older adapters.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│  agent process                                                       │
│  ┌──────────┐  ┌─────────────────────────────────────────────────┐  │
│  │ pydantic │  │  spendguard SDK (Python)                        │  │
│  │  -AI /   │──┤  • derives stable idempotency_key per call      │  │
│  │ langchain│  │  • catches DecisionStopped / ApprovalRequired   │  │
│  │ /openai- │  │  • async e.resume(client) on approval           │  │
│  │ agents / │  └────────────────┬────────────────────────────────┘  │
│  │  AGT     │                   │ UDS gRPC                          │
│  └──────────┘                   ▼                                   │
└──────────────────────┬─────────────────────────────────────────────┘
                       │  same pod / same host
                       ▼
       ┌────────────────────────────────────────────────────┐
       │  sidecar    (Rust, tonic over UDS)                 │
       │  • per-pod fencing lease   (active/standby)        │
       │  • L3 contract DSL evaluator  (CEL + rules)        │
       │  • mTLS gRPC client → ledger + canonical-ingest    │
       │  • signs every audit row (Ed25519 OR KMS ECDSA)    │
       └─────────────┬───────────────────────┬──────────────┘
                     │ mTLS gRPC             │ mTLS gRPC
                     ▼                       ▼
       ┌─────────────────────┐   ┌────────────────────────┐
       │  ledger             │   │  canonical_ingest      │
       │  • Postgres-backed  │   │  • signature verify    │
       │  • Stripe-style     │   │   (Ed25519 + ECDSA)    │
       │    auth/capture/    │   │  • per-decision_id     │
       │    release          │   │    canonical ordering  │
       │  • append-only      │   │  • 3 storage classes   │
       │    audit_outbox     │   │  • orphan reaper       │
       │  • idempotent SP    │   │  • backpressure-aware  │
       │    (post_ledger_tx) │   │                        │
       └──────┬──────────────┘   └────────────────────────┘
              │                              ▲
              │  audit_outbox.pending_forward│
              ▼                              │
       ┌─────────────────────┐               │
       │ outbox_forwarder    │───────────────┘
       │ (leader-elected     │
       │  daemon, k8s lease) │
       └─────────────────────┘

       ┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
       │ control_plane   │    │ dashboard        │    │ webhook_receiver│
       │ (REST API for   │    │ (operator UI:    │    │ (provider       │
       │  tenants /      │    │  budgets,        │    │  HMAC-verified  │
       │  budgets /      │    │  decisions,      │    │  HTTPS webhook  │
       │  approvals)     │    │  audit export)   │    │  → ledger ops)  │
       └─────────────────┘    └──────────────────┘    └─────────────────┘

       ┌─────────────────┐    ┌──────────────────┐
       │ ttl_sweeper     │    │ usage_poller     │
       │ (releases       │    │ (OpenAI /        │
       │  expired        │    │  Anthropic admin │
       │  reservations)  │    │  API → ledger)   │
       └─────────────────┘    └──────────────────┘
```

Every service exposes `/metrics` (Prometheus text format, per-handler counters broken out by ok/err). Every external surface is mTLS; every audit row is signed.

---

## Project status

**Phase 5 GA hardening — shipping.** Design-partner pilot underway.

Recent ship train (round-2 followup):
- ✅ Per-service Prometheus `/metrics` endpoints on all 8 services
- ✅ Helm production env-mapping wired for the 5 deployable services
- ✅ Real AWS KMS signing (ECDSA P-256, IRSA-compatible) + ECDSA verifier in canonical_ingest
- ✅ Approval bundling end-to-end: producer SP + sidecar resume path + Python SDK + DEMO_MODE=approval
- ✅ Rust toolchain bumped 1.88 → 1.91 (foundation for aws-sdk-kms 1.x)
- ✅ Retention sweeper crate (prompt + raw provider payload redaction)
- ✅ Audit-chain immutability triggers + fencing CAS + per-unit balance invariants verified
- ⏳ Real-cluster kind validation (operator-side, post-merge)

42 PRs merged in the last round-2 pass. See [`docs/PHASE_4_VALIDATION_REPORT.md`](docs/PHASE_4_VALIDATION_REPORT.md) for the validation matrix.

---

## Service map

| Service | What it does | Port |
|---|---|---:|
| [`ledger`](services/ledger/) | Postgres-backed double-entry ledger + audit transactional outbox | 50051 |
| [`sidecar`](services/sidecar/) | Per-pod UDS gRPC server; contract evaluator; mTLS clients | (UDS) |
| [`canonical_ingest`](services/canonical_ingest/) | Per-decision_id canonical ordering + 3 storage classes | 50052 |
| [`control_plane`](services/control_plane/) | REST API for tenants / budgets / approvals | 8091 |
| [`dashboard`](services/dashboard/) | Read-only operator UI (budgets / decisions / outbox / audit export) | 8090 |
| [`outbox_forwarder`](services/outbox_forwarder/) | Closes the audit-chain loop (ledger.audit_outbox → canonical_ingest) | — |
| [`ttl_sweeper`](services/ttl_sweeper/) | Releases expired reservations via Ledger.Release(reason=TTL_EXPIRED) | — |
| [`webhook_receiver`](services/webhook_receiver/) | Translates provider HTTPS webhooks → Ledger gRPC ops (HMAC-verified) | 8443 |
| [`usage_poller`](services/usage_poller/) | OpenAI / Anthropic admin-usage API → `provider_usage_records` | — |
| [`signing`](services/signing/) | Producer signing trait (LocalEd25519Signer + KmsSigner + verifier) | — |

Each service has its own `README.md` linking to specs.

---

## SDK

[![PyPI](https://img.shields.io/pypi/v/spendguard-sdk?label=spendguard-sdk)](https://pypi.org/project/spendguard-sdk/)
[![Python](https://img.shields.io/pypi/pyversions/spendguard-sdk)](https://pypi.org/project/spendguard-sdk/)

```bash
pip install spendguard-sdk
# or with a framework integration:
pip install 'spendguard-sdk[pydantic-ai]'
pip install 'spendguard-sdk[langchain]'
pip install 'spendguard-sdk[langgraph]'
pip install 'spendguard-sdk[openai-agents]'
pip install 'spendguard-sdk[agt]'
```

```python
from spendguard import SpendGuardClient, ApprovalRequired, DecisionStopped

async with SpendGuardClient(socket_path="/var/run/spendguard/adapter.sock",
                            tenant_id=TENANT) as sg:
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
        # Contract refused. Surface to operator + abort run.
        raise
    except ApprovalRequired as e:
        # Pause, get operator approval, then:
        resume_outcome = await e.resume(sg)
```

Framework integrations live under [`sdk/python/src/spendguard/integrations/`](sdk/python/src/spendguard/integrations/): Pydantic-AI, LangChain, LangGraph, OpenAI Agents SDK, Microsoft AGT.

---

## Wire spec

Protobuf wire contracts under [`proto/spendguard/`](proto/spendguard/):

- `common/v1/common.proto` — `Idempotency`, `Fencing`, `PricingFreeze`, `CloudEvent`, `UnitRef`, `BudgetClaim`
- `ledger/v1/ledger.proto` — 17 RPCs (ReserveSet / Release / CommitEstimated / ProviderReport / InvoiceReconcile / RecordDeniedDecision / AcquireFencingLease / GetApprovalForResume / MarkApprovalBundled / ...)
- `sidecar_adapter/v1/adapter.proto` — Handshake / RequestDecision / ConfirmPublishOutcome / EmitTraceEvents / ResumeAfterApproval / StreamDrainSignal
- `canonical_ingest/v1/canonical_ingest.proto` — AppendEvents (idempotent, ordered per `(decision_id, sequence)`)

---

## Deploy

**Docker Compose (demo / local dev):** `deploy/demo/compose.yaml` — full stack with PKI bootstrap, manifest signing, mTLS internal, all on one network.

**Kubernetes (Helm):** `charts/spendguard/` — DaemonSet sidecar + Deployments for ledger / canonical_ingest / control_plane / dashboard / webhook_receiver. `chart.profile=production` enforces required-input gates (bundle hashes, trust-root SPKI, real Postgres URL) at template render time. Real-cluster end-to-end validation still pending (see issue [#3](https://github.com/m24927605/agentic-spendguard/issues/3)).

**Signing modes:**
- `local` — Ed25519 PKCS8 PEM mounted from K8s Secret (demo / on-prem)
- `kms` — AWS KMS-backed ECDSA P-256 via IRSA (production)
- `disabled` — empty signatures (refuses to construct outside `SPENDGUARD_PROFILE=demo`)

---

## Specs (source of truth)

Read these before changing wire format or invariants:

- [`docs/agent-runtime-spend-guardrails-complete.md`](docs/agent-runtime-spend-guardrails-complete.md) — the full design doc
- [`docs/trace-schema-spec-v1alpha1.md`](docs/trace-schema-spec-v1alpha1.md) — CloudEvent / audit chain
- [`docs/ledger-storage-spec-v1alpha1.md`](docs/ledger-storage-spec-v1alpha1.md) — double-entry model, idempotency, replay
- [`docs/contract-dsl-spec-v1alpha1.md`](docs/contract-dsl-spec-v1alpha1.md) — Contract DSL (CEL subset) + decision boundary semantics
- [`docs/sidecar-architecture-spec-v1alpha1.md`](docs/sidecar-architecture-spec-v1alpha1.md) — fencing, drain, capability handshake
- [`docs/stage2-poc-topology-spec-v1alpha1.md`](docs/stage2-poc-topology-spec-v1alpha1.md) — Phase 1 SaaS topology + durability invariants

All locked at v1alpha1 — schema bumps land via additive proto changes (backwards-compatible).

---

## Contributing

This is a pilot codebase shipping under active design-partner engagement. Outside contributions are welcome but the wire spec + audit invariants are append-only — open an issue first if you're about to touch `proto/` or `migrations/`.

---

## License

Apache 2.0. See [`LICENSE`](LICENSE).
