# Phase 2B Checkpoint — POC State + Phase 3 Plan

**Date**:2026-05-09
**Branch**:`main` at commit `dc7a7b6`(Outbox Forwarder closure)

---

## 1. What Phase 2B Built

### 1.1 Six Primitive Layers (T → L → C → D → E → P)

| Layer | Status | Demo Coverage |
|---|---|---|
| **T** Trace | ✅ in code | CloudEvent v1.0 flat shape;sidecar emits;webhook receiver synthesizes;TTL sweeper synthesizes;outbox forwarder reconstructs |
| **L** Ledger | ✅ end-to-end | Postgres SERIALIZABLE + audit_outbox transactional outbox;account_kinds {available,reserved_hold,committed_spend,refund_credit,debt,adjustment,dispute_adjustment};reserve→commit→provider_report→invoice_reconcile + release lifecycle |
| **C** Contract DSL | 🟡 schema-only | Bundle build pipeline cold-path proven;sidecar loads bundle at startup;**hot-path evaluation deferred to Phase 3 wedge** |
| **D** Decision | ✅ end-to-end | sidecar `RequestDecision` with claims + idempotency + fencing;commit lifecycle state machine (estimated → provider_reported → invoice_reconciled);release path symmetric |
| **E** Evidence | ✅ end-to-end | audit_outbox dual-row(decision + outcome) for Step 9;global_keys idempotency suffix;outbox forwarder pushes to CI canonical_events |
| **P** Proof | 🟡 partial | Audit chain durable end-to-end;CI quarantine non-durable (POC gap);ORPHAN reaper deferred;no per-event signature verification (strict_signatures=false POC) |

### 1.2 Services (10 running in demo)

| Service | Role | Status |
|---|---|---|
| postgres | Ledger DB + canonical DB | ✅ |
| pki-init | Root CA + per-service certs (idempotent) | ✅ |
| bundles-init | Contract / schema / pricing bundles → runtime.env | ✅ |
| canonical-seed-init | Seeds canonical schema_bundles after bundles-init | ✅ NEW |
| manifest-init / endpoint-catalog | Endpoint catalog SSE | ✅ |
| ledger | mTLS gRPC server;all SP transactions | ✅ |
| canonical-ingest | mTLS gRPC server;AppendEvents + canonical_events | ✅ |
| sidecar | UDS adapter + mTLS to ledger;decision lifecycle | ✅ |
| webhook-receiver | HTTPS POST;HMAC verify;dedupe;mTLS to ledger;routes provider_report / invoice_reconcile | ✅ |
| ttl-sweeper | Polls expired reservations;Release(TTL_EXPIRED) | ✅ NEW |
| outbox-forwarder | Polls audit_outbox.pending_forward;AppendEvents to CI | ✅ NEW |

### 1.3 Demo Modes (5;all PASS)

| Mode | Lifecycle | Verifies |
|---|---|---|
| `decision`(default) | reserve→commit→provider_report | Step 8 |
| `invoice` | + invoice_reconcile | Step 9 dual-row audit |
| `agent` | Pydantic-AI Agent + MockLLM | Step 7 |
| `release` | reserve→RUN_ABORTED→release | Step 7.5 |
| `ttl_sweep`(SIDECAR_TTL_SECONDS=5) | reserve→TTL→sweeper auto-release | TTL_EXPIRED path |

**All 5 modes** also exercise Outbox Forwarder closure(audit_outbox → canonical_events)。

---

## 2. Key Architectural Decisions

### 2.1 D9: Provider Webhook Receiver = Only Provider Entry
- Implemented as real HTTPS service(Stage 2 §8.2.3)
- HMAC-SHA256 over raw body bytes;Postgres webhook_dedupe with PK (provider, event_kind, provider_account, provider_event_id)
- Routes to ledger gRPC;audit goes via ledger.audit_outbox(per §11.3 NOT direct CI emit)

### 2.2 Step 9 Dual-Row Audit Pattern
- InvoiceReconcile is FINAL state(Contract §5 any_to_invoice_reconciled)
- Caller signs audit.outcome;handler synthesizes audit.decision(deterministic UUID derive via sha256(outcome_id || ":decision")[0..16])
- audit_outbox_global_keys gets idempotency_key suffix `:decision` / `:outcome` to satisfy UNIQUE
- ledger_transactions.audit_decision_event_id anchors to OUTCOME row(mirror Step 7 commit convention)

### 2.3 Audit Chain Closure
- audit_outbox accumulates atomically with ledger transactions
- Outbox forwarder polls pending_forward=TRUE → AppendEvents to canonical_ingest
- canonical_events table is the immutable audit log
- Happy path(APPENDED/DEDUPED)clears pending;non-success keeps pending(no silent drop)

### 2.4 Fencing Scopes
- 3 separate scopes per workload(per Stage 2 §4.4):
  - `33333333-...` sidecar(scope_type=reservation/budget_window)
  - `...-050` webhook receiver(control_plane_writer)
  - `...-060` ttl-sweeper(control_plane_writer;Migration 0019 scope-by-reason)
- Outbox forwarder doesn't need fencing(read-only on audit_outbox + UPDATE forwarding fields only;immutability trigger allows)

---

## 3. Known POC Limitations

### 3.1 GA-blocking
- Fencing acquire RPC + sidecar startup CAS recovery
- Multi-claim ReserveSet(POC single-claim only)
- Real signing key infrastructure(strict_signatures=false in canonical_ingest)
- CI quarantine durability(0003_audit_quarantine non-durable)
- ORPHAN_OUTCOME reaper(deferred)
- Multi-pod scaling(producer_sequence races)

### 3.2 Single-instance assumptions
- TTL Sweeper:1 pod
- Outbox Forwarder:1 pod
- Webhook Receiver:1 pod with shared HMAC secret

### 3.3 Spec features not in POC
- Step 10 Refund / Dispute / Compensate handlers(機械擴充;deferred)
- TTL via contract rule reservation.ttl(POC env override)
- Per-tenant pricing override
- Cross-region failover

---

## 4. Phase 3 Plan

### 4.1 Wedge: Contract DSL Evaluation Engine

**Why first**:per memory `project_three_pillars.md`,Predict + Control + Optimize 三柱中 Predict 完全沒實作;Control 90% 蓋了。Contract eval = Predict ∩ Control 的接點 = 真 wedge。

**Scope**:
- Sidecar hot-path evaluator(reads contract bundle,applies Contract DSL §5 commitStateMachine + §5.1a refund_policy + §6 reservation policy + §7 reservation TTL rules)
- Decision-time policy enforcement(budget_exhausted DENY,threshold-based alerts,multi-tier approval gates)
- Demo:budget claim exceeds → contract rule DENY → sidecar 拒 ReserveSet

**Estimate**:5-7 Codex design rounds + ~10-15 implementation turns(roughly Step 9 量級)。

### 4.2 GA Gates(After Wedge)

- Fencing acquire RPC
- Multi-claim ReserveSet(CommitEstimatedSet / ReleaseSet)
- Real signing key rotation
- Multi-pod work distribution
- Chaos test suite per Stage 2 §13

### 4.3 Step 10 Refund / Dispute / Compensate(優先級低)

- Refund design 已 LOCKED(2 rounds);impl 約 Step 9 一半
- Dispute(state machine)+ Compensate(operator op)pattern adaptation

---

## 5. POC Current State Numbers

- **Code**:8 Rust services(ledger / canonical_ingest / sidecar / webhook_receiver / ttl_sweeper / outbox_forwarder + 2 init helpers);~35K lines Rust + ~5K SQL
- **Demo wall time**:~25-30s per mode end-to-end(build + up + verify)
- **audit_outbox volume per invoice demo**:6 rows(1 deposit + 1 reserve + 1 commit_outcome + 1 provider_report + 2 invoice dual)
- **Ledger SP signatures**:7 stored procedures(post_ledger_transaction + commit_estimated + provider_reported + invoice_reconciled + release + ttl_sweeper extension + post_release_transaction modifier)
- **Codex review rounds invested**:~25 rounds across 5 components(Step 9 alone took 7;subsequent components averaged 2-3 rounds — pattern locking accelerates convergence)

---

## 6. Reading Order for New Engineers

1. `docs/contract-dsl-spec-v1alpha1.md` §5(commitStateMachine)+ §5.1a(refund/dispute)+ §6(stage 7 commit_or_release)+ §7(reservation TTL)
2. `docs/ledger-storage-spec-v1alpha1.md` §3(per-unit balance)+ §10(operation kinds + account_kinds)
3. `docs/stage2-poc-topology-spec-v1alpha1.md` §0.2(D9 / D12 invariants)+ §4(audit_outbox)+ §11(webhook receiver)+ §13(chaos tests)
4. `services/ledger/src/handlers/{reserve_set,commit_estimated,provider_report,invoice_reconcile,release,refund_credit}.rs` — lifecycle handlers
5. `services/ledger/migrations/0007-0019` — append-only ledger schema + SPs
6. `deploy/demo/Makefile` + `compose.yaml` + `demo/run_demo.py` — demo flow

---

## 7. Demo Quick Start

```bash
cd deploy/demo
docker compose down -v --remove-orphans
DEMO_MODE=invoice make demo-up
```

Expected:Step 9 lifecycle PASS + Outbox forwarder closure PASS。

For TTL sweeper demo(needs short sidecar TTL):
```bash
docker compose down -v --remove-orphans
export SIDECAR_TTL_SECONDS=5
DEMO_MODE=ttl_sweep make demo-up
```
