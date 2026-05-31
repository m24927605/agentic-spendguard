# SpendGuard Migration Playbook

> Owner: platform operations
> Scope: GA release database migrations for ledger, canonical_ingest, and control_plane
> Source of truth: `docs/operations/migration-inventory-v1alpha1.txt`

## 1. Preflight

Run these gates from the exact release commit before touching production:

```bash
scripts/release/verify-migration-inventory.sh
CONTAINER=spendguard-ga04-migrations EVIDENCE_PREFIX=/tmp/spendguard-ga04 scripts/verify-migrations-postgres16.sh
helm template spendguard charts/spendguard -f charts/spendguard/values-production.example.yaml >/tmp/spendguard-ga04-production-render.yaml
```

The migration inventory must match the release commit. A checksum mismatch means an already-shipped migration file changed and the release is blocked until Staff+ review decides whether to rebuild the bundle or cut a forward-fix migration.

## 2. Production Readiness Checks

Confirm all planning and access items before the maintenance window. Do not create the production rollback backup until the write freeze checkpoint in section 3.

| Check | Required evidence |
|---|---|
| Release commit | Full 40-character commit SHA from the signed release bundle |
| Inventory | `scripts/release/verify-migration-inventory.sh` output |
| Backup plan | Dump destination, provider snapshot procedure, encryption, retention, and operator access prechecked |
| Restore rehearsal environment | Empty isolated restore databases or cluster available; restore credentials tested with non-production data |
| App compatibility | New application image is backward compatible with the current schema until migration completion |
| Audit chain | Current immutable audit verification job is green |
| Pager coverage | Migration owner, database owner, security owner, and incident commander assigned |

## 3. Write Freeze Checkpoint

Enter write freeze before taking rollback backups. Drain or pause write-producing workers, stop scheduled jobs that create ledger/canonical/control-plane rows, and record a coordinated audit cutoff:

```sql
SELECT now() AS cutoff_at, current_database() AS database_name;
```

The incident commander records the cutoff timestamp for all three databases. If the database provider supports coordinated snapshots, take snapshots for ledger, canonical_ingest, and control_plane in the same maintenance step. If snapshots cannot be coordinated, logical dumps are still required, but the cutoff and paused write state become the consistency boundary.

## 4. Backup Checkpoint

Take backups only after the write freeze checkpoint:

```bash
export BACKUP_DIR=/secure/spendguard/backups/$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p "$BACKUP_DIR"

pg_dump --format=custom --no-owner --file="$BACKUP_DIR/ledger.dump" "$LEDGER_DATABASE_URL"
pg_dump --format=custom --no-owner --file="$BACKUP_DIR/canonical_ingest.dump" "$CANONICAL_INGEST_DATABASE_URL"
pg_dump --format=custom --no-owner --file="$BACKUP_DIR/control_plane.dump" "$CONTROL_PLANE_DATABASE_URL"

shasum -a 256 "$BACKUP_DIR"/*.dump > "$BACKUP_DIR/SHA256SUMS"
```

Also record provider-native snapshot identifiers for each database. Logical dumps are the portable restore artifact; provider snapshots are the fast restore artifact. The backup transcript must include the cutoff timestamp recorded in section 3.

## 5. Restore Rehearsal Checkpoint

Before production apply, restore the exact dumps or provider snapshots created after the write freeze checkpoint into isolated databases and run smoke checks:

```bash
createdb spendguard_restore_ledger
createdb spendguard_restore_canonical
createdb spendguard_restore_control_plane

pg_restore --exit-on-error --dbname=spendguard_restore_ledger "$BACKUP_DIR/ledger.dump"
pg_restore --exit-on-error --dbname=spendguard_restore_canonical "$BACKUP_DIR/canonical_ingest.dump"
pg_restore --exit-on-error --dbname=spendguard_restore_control_plane "$BACKUP_DIR/control_plane.dump"
```

The restore rehearsal is a hard gate and must use the post-freeze, post-cutoff backup artifacts from section 4. If restore fails, do not apply production migrations.

## 6. Apply Order

Apply only direct deploy migrations listed in `docs/operations/migration-inventory-v1alpha1.txt`; never apply files under `migrations/down` during a production rollout.

1. Confirm the write freeze checkpoint is still active.
2. Apply ledger migrations in lexical order from `services/ledger/migrations`.
3. Apply canonical_ingest migrations in lexical order from `services/canonical_ingest/migrations`.
4. Apply control_plane migrations in lexical order from `services/control_plane/migrations`.
5. Run service smoke checks and audit-chain verification before restoring full traffic.

Use `psql -v ON_ERROR_STOP=1` for every file. Stop immediately on the first error and move to the partial-failure section.

## 7. Immutable Tables

These tables are forward-fix only in production:

| Database | Tables |
|---|---|
| ledger | `audit_outbox`, ledger account/transaction/entry tables, approval and notification audit tables |
| canonical_ingest | `canonical_events`, `audit_outcome_quarantine`, `canonical_event_replay_dedup`, signing key registry tables |
| control_plane | `control_plane_audit_outbox`, predictor endpoint configuration audit rows |

Do not truncate, delete, rewrite, or run down migrations against immutable audit data to make a rollback easier. If a migration corrupts immutable state, restore into a new database from the backup checkpoint and reconcile through the audit-chain incident process.

## 8. Partial Failure Recovery

If a migration fails before commit, leave the application drained and inspect the database transaction state. If the failed migration committed partial objects, do not edit the shipped migration file. Create a forward-fix migration with a higher version number.

Recovery sequence:

1. Capture `psql` output, failing migration name, release SHA, and `SELECT now(), current_database(), current_user`.
2. Stop remaining migrations.
3. Run read-only schema inspection for the affected database.
4. Decide with the database owner and security owner whether the safe action is rerun, forward-fix, or restore to a new database.
5. Resume only after the inventory and Postgres 16 verification pass on the corrected commit.

## 9. Post-Apply Checks

Run these checks before ending the window:

```bash
scripts/release/verify-migration-inventory.sh
CONTAINER=spendguard-ga04-migrations EVIDENCE_PREFIX=/tmp/spendguard-ga04 scripts/verify-migrations-postgres16.sh
```

For production databases, also verify:

| Check | Query or evidence |
|---|---|
| Ledger prediction columns | `information_schema.columns` contains `predicted_a_tokens`, `run_projection_at_decision_atomic`, `prediction_strategy_used` on `audit_outbox` |
| Canonical mirror columns | `canonical_events` contains `payload_json`, `prediction_strategy_used`, `run_id_mirror` |
| Replay protection | `canonical_event_replay_dedup` exists |
| Control-plane RLS | `control_plane_audit_outbox_forwarder_update` policy exists |
| Audit chain | latest immutable audit verifier run exits 0 |

## 10. Evidence Bundle

Store the following with the release record:

- Release commit SHA and bundle checksum
- `docs/operations/migration-inventory-v1alpha1.txt`
- Migration command transcript
- Backup paths and snapshot identifiers
- Post-freeze cutoff timestamp
- Restore rehearsal transcript from the post-freeze backup artifacts
- Post-apply smoke outputs
- Incident decision record if any migration required rerun, forward-fix, or restore
