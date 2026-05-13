# spendguard-ledger

Append-only double-entry ledger + audit transactional outbox for the
Agentic SpendGuard Phase 1 first-customer (K8s SaaS-managed) POC.

## Spec map

- Wire contract: `proto/spendguard/ledger/v1/ledger.proto`
- Storage model: `docs/ledger-storage-spec-v1alpha1.md` (LOCKED)
- Audit invariant: `docs/contract-dsl-spec-v1alpha1.md` §6 + `docs/stage2-poc-topology-spec-v1alpha1.md` §4
- Per-unit balance: Ledger §3
- Pricing freeze: Ledger §13 (4-layer) + Stage 2 §9.4 (build-time, not hot path)

## Crate layout

```
src/
├── lib.rs                           re-exports + tonic-include_proto
├── main.rs                          binary: pool + verify durability + serve
├── config.rs                        env-driven Config
├── server.rs                        Ledger trait impl (RPC -> handler dispatch)
├── handlers/
│   ├── mod.rs
│   ├── reserve_set.rs               ReserveSet end-to-end (Phase 2B Step 1)
│   ├── release.rs                   stub (Status::unimplemented)
│   └── replay.rs                    ReplayAuditFromCursor + QueryDecisionOutcome
├── domain/
│   ├── mod.rs
│   ├── error.rs                     DomainError + Postgres SQLSTATE -> proto.Error.Code
│   ├── lock_order.rs                sha256-based lock_order_token derivation
│   └── minimal_replay.rs            Ledger §7 minimal replay construction
└── persistence/
    ├── mod.rs
    ├── pool.rs                      sqlx pool + sync replica config probe
    ├── post_transaction.rs          stored-proc invocation + idempotency lookup
    └── replay.rs                    audit_outbox cursor + decision outcome query

migrations/                          Postgres schema (Ledger §5 + Stage 2 §4.3)
├── 0001_ledger_units.sql
├── 0002_ledger_shards.sql
├── 0003_budget_window_instances.sql
├── 0004_ledger_accounts.sql
├── 0005_pricing_snapshots.sql
├── 0006_fencing_scopes.sql
├── 0007_ledger_transactions.sql
├── 0008_ledger_entries.sql          PARTITION BY recorded_month
├── 0009_audit_outbox.sql            partition-safe; per-decision unique
├── 0010_projections.sql
├── 0011_immutability_triggers.sql   defense-in-depth (trigger + role + procedure)
└── 0012_post_ledger_transaction.sql server-side derivation + fencing CAS

build.rs                             tonic-build proto codegen
Cargo.toml                           tonic 0.12 + sqlx 0.8 + tokio + ed25519-dalek
```

## Audit invariant in code

The ReserveSet path satisfies "no audit, no effect":

1. handler validates request + computes `lock_order_token`
2. handler invokes `post_ledger_transaction(...)` — single Postgres tx that
   - acquires fencing_scopes lock (FOR UPDATE) and CAS-checks epoch
   - INSERTs ledger_transactions, ledger_entries, audit_outbox atomically
   - commits with `synchronous_commit=on` + sync_standby quorum
3. only after commit ack does the gRPC response return `ReserveSetSuccess`
4. sidecar gates `publish_effect` on receipt of `Success`

If commit fails (sync replica unreachable, fencing stale, balance violation,
pricing unknown, etc.), the response carries an `Error` and no audit row
exists; sidecar must NOT publish.

## What's implemented in Phase 2B Step 1

- ReserveSet (handler + stored proc + audit_outbox write)
- ReplayAuditFromCursor (sidecar crash recovery)
- QueryDecisionOutcome (recovery state machine support)
- Domain error mapping (Postgres SQLSTATE -> proto.Error.Code)
- Lock order token derivation + verification
- Minimal replay construction
- Postgres durability config probe at startup

## What's deferred to vertical slice expansion

- Release handler (compensating release entries)
- CommitEstimated / ProviderReport / InvoiceReconcile (commit state machine)
- RefundCredit / DisputeAdjustment / Compensate
- Outbox forwarder process (separate binary; reads pending_forward=TRUE,
  pushes to Canonical Ingest)
- Ledger-side account auto-creation (`get_or_create_account(...)`)
- Chaos test suite (per_unit_balance / immutability / sequence_allocator /
  fencing split-brain / replay-during-in-flight / sync_replica_quorum_loss)
- Integration tests against testcontainers Postgres

## Ops notes (Phase 1 first customer)

- Postgres 16 SERIALIZABLE; primary in us-west-2a + 2 sync replicas in 2b/2c
- `synchronous_commit=on`, `synchronous_standby_names='ANY 1 (replica_b, replica_c)'`
- Pool size: 32 connections (POC); tune for sync replica latency
- WAL archive to S3 + 35-day PITR
- Per-tenant DB: max 5 (Stage 2 §9.3); migration to shared+RLS triggered before
  customer 3 onboarded
- Pricing snapshots are populated by the Platform Pricing Authority DB build
  pipeline at contract bundle deployment (cold path); never queried at decision
  time

## Building

`cargo build` (Rust toolchain not present in current workspace; use Docker
or install via rustup).
