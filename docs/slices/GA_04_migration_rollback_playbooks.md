# GA 04 - Migration and Rollback Playbooks

> **Branch**: `ga/GA_04_migration_rollback_playbooks`
> **Status**: design
> **Spec ancestor(s)**: `ga-readiness-spec-v1alpha1.md`
> **Estimated change size**: medium; scripts and operator docs

---

## §0. TL;DR

Create migration inventory, forward-fix policy, rollback decision tree, and Postgres 16 verification evidence for GA operators.

## §1. Architectural Context

The project has many service migrations. HARDEN verified apply order, but GA needs a documented operator procedure for backups, forward-only migrations, partial failure, and rollback limits.

## §2. Scope

- Migration inventory script
- Migration playbook
- Rollback playbook
- Classification of reversible vs forward-fix-only changes
- Postgres 16 verification evidence

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| Rewriting shipped migrations | Not allowed |
| Building a full migration orchestrator | Future deployment automation |

## §4. File-Level Changes

- Add `docs/operations/migration-playbook.md`
- Add `docs/operations/rollback-playbook.md`
- Add `scripts/release/verify-migration-inventory.sh`
- Possibly update `scripts/verify-migrations-postgres16.sh`
- Add evidence under `docs/reviews/ga-readiness/GA_04_migration_rollback_playbooks/`

## §5. Schema / Config / API Impact

No schema changes expected. This slice documents and verifies existing migrations.

## §6. Audit / Security / Operational Impact

Playbooks must preserve immutable audit data, RLS boundaries, and signing provenance during rollback or forward-fix.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Migration inventory misses a service | Script exits non-zero |
| Existing migration modified accidentally | Review rejects |
| Rollback would delete immutable audit data | Playbook marks forward-fix only |
| Postgres verification fails | Slice cannot merge |

## §8. Acceptance Gates

- `scripts/release/verify-migration-inventory.sh`
- `CONTAINER=spendguard-ga04-migrations scripts/verify-migrations-postgres16.sh`
- Migration playbook includes backup/restore checkpoints
- Rollback playbook includes decision tree and forward-fix-only warnings

## §9. Review Checklist

1. Does inventory cover ledger, canonical_ingest, and control_plane migrations?
2. Are irreversible migrations honestly labeled?
3. Are immutable audit tables never deleted in rollback instructions?
4. Does verification run on fresh Postgres 16?
5. Are partial failure recovery steps explicit?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Cloud-managed backup automation | Provider-specific future work |

## §11. Risk / Rollback

Docs/scripts only. Revert if the inventory or playbook is wrong.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must reject any false rollback claim for immutable audit data.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Performance/Database Architect | Migration inventory must be deterministic | Script gate required |
| Security Engineer | Audit immutability overrides rollback convenience | Forward-fix-only classification |

## §14. Merge Checklist

- [ ] Migration inventory script passes
- [ ] Postgres 16 migration verification passes
- [ ] Migration and rollback playbooks exist
- [ ] AIT review clean or arbitration recorded
- [ ] Memory updated
