# spendguard-canonical-ingest

Per-decision sequence-enforced canonical event ingest for the Agentic SpendGuard
Phase 1 first-customer (K8s SaaS-managed) POC.

## Spec map

- Wire contract: `proto/spendguard/canonical_ingest/v1/canonical_ingest.proto`
- Schema model: `docs/trace-schema-spec-v1alpha1.md` §10 (sampling pipeline,
  storage classes, cross-region ordering)
- Producer trust: Trace §13
- Schema bundle: Trace §12
- Audit invariant + sequence: Stage 2 §4.8 (audit.outcome strictly after
  audit.decision; quarantine + 30s ORPHAN_OUTCOME)

## Crate layout

```
src/
├── lib.rs                           re-exports + tonic-include_proto
├── main.rs                          binary
├── config.rs                        env-driven Config
├── server.rs                        CanonicalIngest trait impl
├── handlers/
│   ├── append_events.rs             dedupe + classify + sequence + quarantine
│   ├── verify_schema_bundle.rs
│   └── query_audit_chain.rs         server streaming
├── domain/
│   ├── error.rs                     DomainError + SQLSTATE mapping
│   └── event_routing.rs             event_type -> StorageClass
└── persistence/
    ├── pool.rs
    ├── schema_bundle.rs             cache lookup + upsert
    ├── append.rs                    canonical_events + global_keys + quarantine
    └── query.rs                     audit-chain by decision_id / run_id

migrations/                          Postgres schema
├── 0000_extensions.sql              pgcrypto
├── 0001_schema_bundles.sql          cache from Bundle Registry
├── 0002_canonical_events.sql        partition by recorded_month + global mirror
├── 0003_audit_quarantine.sql        audit.outcome staging
├── 0004_ingest_offset_allocator.sql per-(region, shard) monotonic offset
└── 0005_immutability_triggers.sql   triggers + role grants

build.rs                             tonic-build proto codegen
Cargo.toml                           tonic 0.12 + sqlx 0.8 + tokio + ed25519-dalek
```

## Per-decision sequence enforcement

For each `(tenant_id, decision_id)`:

1. Exactly one `spendguard.audit.decision` event globally (UNIQUE in
   `canonical_events_global_keys`).
2. Optionally one `spendguard.audit.outcome` event (UNIQUE).
3. `audit.outcome` MUST land after `audit.decision`. Three-stage handling:
   - Handler pre-checks `has_preceding_decision(...)`. If decision missing,
     redirect outcome to `audit_outcome_quarantine` (status =
     `AWAITING_PRECEDING_DECISION`).
   - DB-level `assert_audit_outcome_has_preceding_decision` trigger on
     `canonical_events_global_keys` is defense-in-depth: it rejects with
     SQLSTATE `P0002` if outcome reaches the table without a preceding
     decision (handler maps to quarantine fallback).
   - Reaper background process (Phase 2B step 後段; not in this skeleton)
     scans quarantine on 1s tick; releases when decision arrives, marks
     `orphaned` after 30s.

## Storage class routing

Per Trace §10.2:
- `spendguard.audit.*`, `spendguard.tombstone` → `immutable_audit_log` (7yr SOX)
- `spendguard.ledger.*`, `spendguard.approval.*`, `spendguard.refund.*`,
  `spendguard.dispute.*`, `spendguard.decision`, `spendguard.rollback`,
  `spendguard.region_failover_promoted` → `canonical_raw_log` (7yr; hashes only)
- everything else → `profile_payload_blob` (tenant policy retention; RTBF)

POC stores all classes in the single `canonical_events` table with a
`storage_class` column. Phase 1 後段 splits classes into per-class backends
with separate retention + RTBF flows.

## What's implemented in this skeleton

- AppendEvents (dedupe, classify, per-decision sequence routing, backpressure)
- VerifySchemaBundle (existence + hash compare)
- QueryAuditChain (server streaming by decision_id or run_id)
- Per-tenant + per-decision indexes
- Immutability triggers (no UPDATE / DELETE on canonical_events / global_keys
  / schema_bundles; quarantine state-machine UPDATEs only)
- Defense-in-depth trigger for audit.outcome precedence
- Backpressure fail_closed for enforcement-route inserts past threshold

## What's deferred to vertical slice expansion

- Quarantine reaper background process (release when decision lands; mark
  orphaned after 30s)
- Producer signature verification (currently stubbed; will integrate with
  sidecar Producer Trust §13 keys via ledger fencing scope handshake)
- Tombstone events / RTBF flow
- Per-class backend split (separate retention buckets)
- Cross-region ingest offset coordination
- Chaos test suite (forwarder lag, signature rotation, partition rotation,
  quarantine release race, backpressure under burst)
