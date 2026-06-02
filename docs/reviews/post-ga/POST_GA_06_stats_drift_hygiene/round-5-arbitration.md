# POST_GA_06 Round 5 Staff+ Arbitration

Reviewer: codex CLI direct adversarial fallback after AIT nested-wrapper failure.

## Round 5 Finding

- Major: append errors left no active cooldown row, and each retry minted a
  fresh CloudEvent id. If canonical_ingest committed the immutable
  `prediction_drift_alert` but the client observed a timeout or transport
  error, the next cycle could bypass canonical_ingest replay dedupe and append
  duplicate immutable alert rows.

## Panel Votes

| Role | Vote | Decision |
|---|---|---|
| Software Architect | Fix anyway | Persist a pending/replayable alert reservation; retry exact same CloudEvent bytes/id |
| Backend Architect | Fix anyway | Treat commit-then-timeout as unknown outcome; same event id must be reused until `APPENDED`/`DEDUPED` |
| Security Engineer | Fix before merge | Duplicate immutable alert rows weaken audit-chain reliability and incident response |
| Database Optimizer | Fix in-slice | Keep dedupe out of canonical_events hot path; store pending attempt in mutable cooldown state |
| Predictor Domain Expert | Fix in-slice | Duplicate drift alerts corrupt operator signal and violate POST_GA_06 dedup promise |

Final arbitration: fix in-slice. Do not run a sixth review round; Staff+
arbitration is final per slice workflow.

## Implementation

- Migration 0022 now supports two row states:
  - pending emission reservation: `pending_event_id`, `pending_event_time`,
    `pending_event_proto`, `pending_z_score`, `pending_created_at`,
    `pending_expires_at`
  - active cooldown: `last_emitted_at`, `suppress_until`, `last_z_score`
- `PostgresDriftAlertCooldownStore::reserve_emission` runs under tenant RLS,
  reuses unexpired pending CloudEvent proto bytes, or stores a newly signed
  candidate before append.
- `detect_and_emit` sends the reserved attempt and records active cooldown
  only after `sink.emit` returns durable success. `APPENDED` and `DEDUPED`
  remain accepted success states.
- `record_emitted` clears pending fields when the active 24h cooldown is
  written.

## Verification

- `cargo fmt --manifest-path services/stats_aggregator/Cargo.toml`
- `cargo test --manifest-path services/stats_aggregator/Cargo.toml`
  - 32 lib tests passed
  - 1 main test passed
  - 9 Postgres integration tests passed
