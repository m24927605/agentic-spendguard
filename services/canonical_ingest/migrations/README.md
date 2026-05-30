# canonical_ingest migrations

Append-only migration files for the canonical_ingest service Postgres
database. See `services/canonical_ingest/src/persistence/` for the
runtime consumers and `services/canonical_ingest/migrations/down/` for
the rollback companions where shipped.

## File-numbering convention

Files apply in lexicographic order. Numbering uses a 4-digit
zero-padded sequence: `NNNN_short_description.sql`. The runner wraps
each file in its own transaction (see SLICE_01 R5 + 0013 round-2 fix
m3) — no explicit BEGIN/COMMIT inside the SQL bodies.

The migration index as of SLICE_06 R2:

| #    | Slice         | What                                                                   |
|------|---------------|------------------------------------------------------------------------|
| 0000 | Phase 1       | pgcrypto + uuid-ossp extension installs                                |
| 0001 | Phase 1       | schema_bundles registry                                                |
| 0002 | Phase 1       | canonical_events partitioned (recorded_month range)                    |
| 0003 | Phase 1       | audit_quarantine                                                       |
| 0004 | Phase 1       | ingest_offset_allocator                                                |
| 0005 | Phase 1       | immutability triggers + canonical_ingest_application_role / reader_role |
| 0006 | Phase 1       | pricing_table                                                          |
| 0007 | Phase 1       | audit_signature_quarantine                                             |
| 0008 | Phase 1       | S7 validity_window_reasons                                             |
| 0009 | Phase 1       | signing_keys_registry                                                  |
| 0010 | Phase 1       | S13 pricing_audit                                                      |
| 0011 | Phase 1       | add canonical_events.failure_class                                     |
| 0012 | Phase 1       | cost_advisor_safe_decode                                               |
| 0013 | SLICE_01      | canonical_events prediction columns (mirror of 0046_audit_outbox)      |
| —    | —             | **0014 intentionally skipped — see below**                             |
| 0015 | SLICE_01      | audit_outcome_quarantine prediction columns                            |
| 0016 | SLICE_06      | output_distribution_cache (R2 B1+M16+M17: FOR ALL RLS + REVOKE PUBLIC) |
| 0017 | SLICE_06      | run_length_distribution_cache (mirror RLS shape)                       |
| 0018 | SLICE_06 R2   | canonical_events aggregator mirror columns (R2 B4 Option A)            |

## The 0014 gap

**R2 M6 (DB F4)**: there is no `0014_*.sql`. SLICE_01 reserved the
slot for a follow-up migration that wasn't needed by the time the
slice locked, so 0015 shipped in the next slot. The gap is intentional
and the migration runner skips it cleanly because it iterates the
glob in lexicographic order, not by sequential integer.

**Going forward**: new migrations take the next free number. Do NOT
backfill 0014 — that would shift the ordering relative to any operator
who has already replayed 0015+ against a real database, and Postgres
schema migrations have no notion of "patches applied between 13 and
15" beyond the timestamp in the runner's bookkeeping table.

## Down migrations

Per SLICE_03 R2 M3 convention, down migrations are written **only**
when (a) the migration body is non-trivial and (b) the operator-side
recipe to roll back isn't a simple `DROP TABLE ... CASCADE` /
`ALTER TABLE ... DROP COLUMN`. The current down/ directory holds:

* `down/0013_canonical_events_prediction_columns_down.sql`
* `down/0015_audit_outcome_quarantine_prediction_columns_down.sql`

0016 / 0017 / 0018 have no down/ files. Rollback recipe for each:

```sql
-- 0016 / 0017
DROP TABLE output_distribution_cache CASCADE;
DROP TABLE run_length_distribution_cache CASCADE;

-- 0018
ALTER TABLE canonical_events
    DROP COLUMN IF EXISTS prompt_class_fingerprint,
    DROP COLUMN IF EXISTS prompt_class,
    DROP COLUMN IF EXISTS run_id_mirror,
    DROP COLUMN IF EXISTS agent_id,
    DROP COLUMN IF EXISTS model;
DROP INDEX IF EXISTS canonical_events_aggregator_bucket_idx;
DROP INDEX IF EXISTS canonical_events_aggregator_run_length_idx;
```

## RLS contract (SLICE_06 R2)

Tables 0016 + 0017 enforce per-tenant Row-Level Security via:

1. `ENABLE ROW LEVEL SECURITY` + `FORCE ROW LEVEL SECURITY`
2. `FOR ALL` policy with both `USING` (SELECT/UPDATE/DELETE) and
   `WITH CHECK` (INSERT/UPDATE) keyed on
   `current_setting('app.current_tenant_id', TRUE)::uuid`
3. NULL session variable → `'00000000-0000-0000-0000-000000000000'`
   (nil UUID) so a missing SET LOCAL fails closed (0 rows visible,
   inserts fail the WITH CHECK clause).
4. `REVOKE SELECT FROM PUBLIC` (R2 M16 belt-and-suspenders).
5. Writer (`services/stats_aggregator/src/aggregation.rs` +
   `run_length.rs`) calls
   `SELECT set_config('app.current_tenant_id', $1, true)` IMMEDIATELY
   after `pool.begin()` for every per-tenant transaction.
6. Reader (`services/output_predictor/src/cache.rs`) calls the same
   set_config inside the lookup transaction.

The R1 shape claimed `BYPASSRLS` for the writer but the role
attribute was never granted; under FORCE RLS that meant every UPSERT
failed. R2 widened the policy to FOR ALL and added SET LOCAL on every
write path.
