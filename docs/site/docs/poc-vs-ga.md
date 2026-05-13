---
description: >-
  Honest assessment of which Agentic SpendGuard capabilities are production-ready
  today versus blocked behind the GA hardening slices, with pointers to
  every open gate and the roadmap entry that closes it.
---

# POC vs GA gates

What's production-ready in this POC, and what's explicitly deferred to
GA hardening.

## ✅ Production-shaped (validated end-to-end)

- mTLS gRPC across all services
- Postgres SERIALIZABLE + transactional `audit_outbox`
- 3-stage commit lifecycle (estimated → reported → invoiced)
- Atomic reservation + TTL release
- Contract DSL hot-path evaluator (`<5ms`)
- 6 framework adapters (Pydantic-AI, LangChain, LangGraph, OpenAI
  Agents, real OpenAI / Anthropic)
- Cross-provider USD-denominated budget
- Operator dashboard + control plane API
- Helm chart + Terraform AWS module

## ⛔ GA gates (NOT yet production-ready)

These are the items that block a real production deployment:

### Multi-pod work distribution

- `sidecar.replicas > 1` produces `producer_sequence` races on the
  `audit_outbox_global_keys` table.
- `outbox-forwarder.replicas > 1` causes double-forward of the same
  row (no leader election yet).
- `ttl-sweeper.replicas > 1` similarly.
- **Fix**: leader election (k8s Lease primitive or DB row-lock based)
  + per-instance producer_sequence partitioning.

### Fencing acquire RPC

- POC seeds a fencing scope with `current_epoch=1` directly in SQL.
- A production deploy needs `Ledger.AcquireFencingLease()` RPC that
  CAS-increments the epoch on takeover; sidecar startup must call
  this before issuing any reserve.
- Without this, sidecar restarts can reuse stale leases.

### Real signing keys

- POC uses `strict_signatures=false` in canonical_ingest; producer
  signatures are placeholder `b''`.
- Production needs real Ed25519 key rotation + KMS-backed signing
  per Stage 2 §17.

### CI quarantine durability

- `audit_quarantine` migration (canonical 0003) is a placeholder.
- ORPHAN_OUTCOME reaper is deferred — outcomes without a matching
  decision currently sit in audit_outbox forever.

### Chaos test suite (Stage 2 §13)

- 7 scenarios specified but not automated:
  - Network partition during ReserveSet commit
  - Postgres failover mid-decision
  - Sidecar OOM mid-publish
  - etc.

### Real provider webhook integration

- Demo's webhook receiver verifies a mock-llm HMAC.
- OpenAI doesn't ship billing webhooks → needs `/v1/usage` poller.
- Anthropic enterprise webhook needs their signing key.
- Per-provider adapter is operator-by-operator work.

### Pricing auto-update poller

- `pricing_table` infrastructure shipped in Phase 4 O3.
- Periodic refresh against provider docs is deferred.
- Static YAML works for POC; production needs daily sync.

### Multi-region failover

- Stage 2 spec covers the design (cross-region replication + failover
  policy); implementation is GA.

## 🟡 Incomplete primitives

- **Refund / Dispute / Compensate (Step 10)** — Contract §5.1a spec
  locked; ledger SP not implemented. Mechanical extension of Step 9
  invoice-reconcile pattern.
- **CEL evaluator** — POC uses declarative when/then. Full CEL
  predicate language is on the v1 roadmap.
- **Bundle hot-reload** — POC only loads on startup. Hot-reload with
  last-known-good fallback is GA.
- **Multi-tier approval flow** — `REQUIRE_APPROVAL` is terminal in
  POC. Operator integration (Slack / PagerDuty / etc.) is product
  work, not infrastructure.

## What this means for users

- **Try the POC**: clone + `make demo-up`. Everything works.
- **Run in dev k8s**: Helm chart works. Single-pod replica defaults
  prevent multi-pod data hazards.
- **Run in production**: NOT YET. The fencing-acquire + multi-pod
  gates need to land first. Expect another major slice of work
  before "I'm betting my agent's budget on this" is true.

See [GA hardening slices](roadmap/ga-hardening-slices.md) for the
design, implementation, test acceptance, and review gates that split
these blockers into independently shippable PRs.

We mark every POC limitation in code + docs explicitly so an operator
who reads end-to-end can audit what they're getting.
