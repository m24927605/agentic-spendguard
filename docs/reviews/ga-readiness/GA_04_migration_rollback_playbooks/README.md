# GA 04 Review Evidence - Migration and Rollback Playbooks

## Scope

GA_04 adds a checked-in migration inventory, operator playbooks for migration and rollback decisions, and Postgres 16 verification hooks.

## Acceptance Evidence

| Gate | Result |
|---|---|
| `scripts/release/verify-migration-inventory.sh` | PASS - verified `docs/operations/migration-inventory-v1alpha1.txt` |
| `CONTAINER=spendguard-ga04-migrations EVIDENCE_PREFIX=/tmp/spendguard-ga04 scripts/verify-migrations-postgres16.sh` | PASS - applied 77 direct deploy migrations on Postgres 16.14 |
| `helm template spendguard charts/spendguard --set chart.profile=demo` | PASS |
| `helm template spendguard charts/spendguard -f charts/spendguard/values-production.example.yaml` | PASS |
| `scripts/release/build-release-bundle.sh --output /tmp/spendguard-ga04-release-bundle-r2` | PASS |
| `scripts/release/check-release-bundle.sh /tmp/spendguard-ga04-release-bundle-r2` | PASS |
| `POSTGRES_IMAGE=postgres:15-alpine CONTAINER=spendguard-ga04-pg15-negative ... scripts/verify-migrations-postgres16.sh` | PASS - failed closed with `expected Postgres 16.x` |
| Migration playbook backup/restore checkpoints | Covered in `docs/operations/migration-playbook.md` |
| Rollback decision tree and forward-fix warnings | Covered in `docs/operations/rollback-playbook.md` |

## Reproducibility Record

| Item | Value |
|---|---|
| Evidence refresh commit | `e0b708c7aa97ba47562a439d8e1f9e842626f14f` before this evidence-note update |
| Postgres image | `postgres:16-alpine` |
| Server version evidence | `/tmp/spendguard-ga04-postgres-version.txt`: `server_version_num=160014`, `server_version=16.14` |
| Helm demo render | `/tmp/spendguard-ga04-helm-demo.yaml` |
| Helm production render | `/tmp/spendguard-ga04-helm-production.yaml` |
| Release bundle | `/tmp/spendguard-ga04-release-bundle-r2` |
| Postgres 15 negative transcript | `/tmp/spendguard-ga04-pg15-negative.out` |

## Postgres 16 Smoke Output

| Database | Evidence file | Result |
|---|---|---|
| ledger | `/tmp/spendguard-ga04-ledger-smoke.txt` | `audit_outbox`, `tokenizer_t1_samples`, and prediction columns present |
| canonical_ingest | `/tmp/spendguard-ga04-canonical-smoke.txt` | `canonical_events`, `canonical_event_replay_dedup`, and mirror columns present |
| control_plane | `/tmp/spendguard-ga04-control-plane-smoke.txt` | `predictor_plugin_endpoints`, `control_plane_audit_outbox`, and forwarder RLS policy present |

## Review Rounds

| Round | Reviewer | Outcome |
|---|---|---|
| R1 | codex CLI via AIT fallback | Pending |
| R1 fixes | implementer | Moved write freeze before backups, enforced Postgres 16.x server version, made release bundle inventory consume the checked-in inventory, and added reproducible evidence references. |

## Staff+ Decisions

| Role | Decision |
|---|---|
| Database Architect | Migration inventory is checksum-pinned and covers direct deploy migrations only. |
| Security Engineer | Immutable audit, signing, RLS, replay, and ledger financial data are forward-fix only. |
| Backend Architect | Existing migration files are never edited after release; fixes require higher-numbered migrations. |
