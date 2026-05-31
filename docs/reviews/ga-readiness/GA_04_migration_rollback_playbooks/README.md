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
| `scripts/release/build-release-bundle.sh --output /tmp/spendguard-ga04-release-bundle-current` | PASS |
| `scripts/release/check-release-bundle.sh /tmp/spendguard-ga04-release-bundle-current` | PASS |
| `POSTGRES_IMAGE=postgres:15-alpine CONTAINER=spendguard-ga04-pg15-negative ... scripts/verify-migrations-postgres16.sh` | PASS - failed closed with `expected Postgres 16.x` |
| Migration playbook backup/restore checkpoints | Covered in `docs/operations/migration-playbook.md` |
| Rollback decision tree and forward-fix warnings | Covered in `docs/operations/rollback-playbook.md` |

## Reproducibility Record

| Item | Value |
|---|---|
| Release commit artifact | `/tmp/spendguard-ga04-release-bundle-current/commit.txt`; must match `git rev-parse HEAD` from the checked-out release branch |
| Postgres image | `postgres:16-alpine`; digest recorded in `/tmp/spendguard-ga04-postgres-version.txt` |
| Server version evidence | `/tmp/spendguard-ga04-postgres-version.txt`: `image_repo_digest=postgres@sha256:...`, `server_version_num=160014`, `server_version=16.14` |
| Helm demo render | `/tmp/spendguard-ga04-helm-demo.yaml` |
| Helm production render | `/tmp/spendguard-ga04-helm-production.yaml` |
| Release bundle | `/tmp/spendguard-ga04-release-bundle-current` |
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
| R1 | codex CLI via AIT fallback | 3 Major and 1 Minor; all fixed in follow-up commits. |
| R1 fixes | implementer | Moved write freeze before backups, enforced Postgres 16.x server version, made release bundle inventory consume the checked-in inventory, and added reproducible evidence references. |
| R2 | codex CLI via AIT fallback | 1 Major and 1 Minor; fixed by rebuilding current-HEAD bundle evidence and recording the Postgres image repo digest. |
| R3 | codex CLI via AIT fallback | 1 Major; fixed the migration playbook checklist so actual rollback backups and restore rehearsal are explicitly post-freeze/post-cutoff. |
| R4 | codex CLI via AIT fallback | 1 Major; fixed rollback playbook wording to use the post-freeze, post-cutoff backup checkpoint. |

## Staff+ Decisions

| Role | Decision |
|---|---|
| Database Architect | Migration inventory is checksum-pinned and covers direct deploy migrations only. |
| Security Engineer | Immutable audit, signing, RLS, replay, and ledger financial data are forward-fix only. |
| Backend Architect | Existing migration files are never edited after release; fixes require higher-numbered migrations. |
