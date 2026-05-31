# SpendGuard Rollback Playbook

> Owner: platform operations
> Rule: immutable audit data is never deleted, rewritten, or down-migrated for rollback convenience.

## 1. Decision Tree

| State | Action |
|---|---|
| New application not deployed, migrations not applied | Cancel rollout. No database action. |
| New application deployed, migrations not applied | Roll back application image and config using the release bundle. |
| Additive migrations applied, old app is schema-compatible | Roll back application image only. Keep database schema. |
| Migration failed before commit and left no schema changes | Fix the migration command/environment, then rerun the same file after database owner approval. |
| Migration committed partial schema changes | Ship a higher-numbered forward-fix migration. Do not edit the shipped migration. |
| Migration touched immutable audit/signing/RLS state | Forward-fix only, or restore to a new database from the preflight backup. |
| Data corruption suspected | Freeze writes, preserve evidence, restore to a new database, reconcile audit chain before traffic returns. |

If the safe action is unclear, freeze writes and escalate to Staff+ database, security, and backend owners. Do not improvise destructive SQL.

## 2. Allowed Rollback Actions

These actions are allowed during a rollback:

- Revert Kubernetes Deployment image tags and config references to the prior release.
- Disable new feature flags that have not changed persisted state.
- Re-run idempotent verification queries.
- Restore a full database into a new database or cluster from the backup checkpoint.
- Ship a forward-fix migration with a new version number.

## 3. Prohibited Rollback Actions

These actions are not allowed in production:

- Editing an existing committed migration file after release.
- Applying `migrations/down` files to live immutable audit databases.
- Truncating or deleting rows from `audit_outbox`, `canonical_events`, `canonical_event_replay_dedup`, `control_plane_audit_outbox`, ledger transaction/entry tables, or signing key registries.
- Disabling RLS or using BYPASSRLS to bypass tenant controls.
- Rewriting CloudEvent payloads, producer IDs, schema bundles, signing metadata, or audit hashes.

## 4. Forward-Fix-Only Classes

| Class | Examples | Required response |
|---|---|---|
| Immutable audit | `audit_outbox`, `canonical_events`, audit quarantine rows | Higher-numbered forward-fix migration or restore to new database |
| Signing provenance | signing key registry, signature metadata, schema bundle references | Security owner approval and audit-chain verification |
| RLS and tenant isolation | RLS policies, writer roles, tenant-scoped settings | Forward-fix migration plus tenant isolation test |
| Replay protection | `canonical_event_replay_dedup` and producer/event identity constraints | Forward-fix migration; never clear dedup history in place |
| Ledger financial state | units, accounts, transactions, entries, reservations | Restore to new database if corrupted; never hand-edit balances |

## 5. Application Rollback Procedure

1. Announce rollback and freeze new release activity.
2. Confirm current database state from the migration inventory and release transcript.
3. Render the prior release chart and values.
4. Apply only application/config rollback changes.
5. Verify pods are running the prior image digest.
6. Run audit-chain and smoke checks.
7. Keep the database at the newer schema unless the restore decision tree explicitly selects restore to a new database.

## 6. Restore-To-New-Database Procedure

Use restore when immutable data corruption or incompatible schema state makes forward-fix unsafe.

1. Keep production writes frozen.
2. Create new databases or a new cluster.
3. Restore the backup checkpoint with `pg_restore --exit-on-error`.
4. Verify dump checksums from `SHA256SUMS`.
5. Run schema, RLS, and audit-chain checks.
6. Point application Secrets to the restored databases through the approved secret manager.
7. Start read-only traffic first, then restore writes after incident commander approval.
8. Preserve the failed databases for forensics until security and database owners release them.

## 7. Down Migration Handling

Down migrations in this repository are test and developer recovery aids. They are not production rollback instructions. A production rollback must prefer app/config rollback, forward-fix migration, or restore-to-new-database.

## 8. Incident Record

Every rollback decision records:

- Release SHA and migration file at fault
- Whether any immutable table was touched
- Backup and restore artifact identifiers
- Decision tree branch selected
- Staff+ approvers
- Audit-chain verification result
- Follow-up forward-fix migration, if any
