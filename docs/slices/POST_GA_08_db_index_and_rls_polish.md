# POST_GA 08 - DB Index and RLS Polish

> **Branch**: `post-ga/POST_GA_08_db_index_and_rls_polish`
> **Status**: draft
> **Spec ancestor(s)**: `post-ga-backlog-spec-v1alpha1.md`, `ledger-storage-spec-v1alpha1.md`, `output-predictor-service-spec-v1alpha1.md`
> **Issues**: #146, #163, #164, #166
> **Estimated change size**: medium; SQL, RLS, runbooks, planner evidence

---

## §0. TL;DR

Polish lower-priority DB/RLS issues: revoke public read access, avoid nil
UUID sentinel collisions, document advisory-lock keepalive recovery, and
remove or justify low-cardinality indexes with planner evidence.

## §1. Architectural Context

GA hardening established RLS and migration safety. Remaining DB items
are defense-in-depth and planner hygiene. They should improve security
and operability without changing business semantics.

## §2. Scope

- #146: `REVOKE SELECT FROM PUBLIC` for `tokenizer_t1_samples`
- #163: nil UUID sentinel collision risk in RLS policy
- #164: TCP keepalive runbook for Postgres advisory lock recovery
- #166: evaluate `output_distribution_cache_freshness_idx` usefulness

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| Broad schema redesign | Not required |
| Stats drift runtime dedup | POST_GA_06 |
| Production DBA automation | Future operations work |

## §4. File-Level Changes

- Add forward SQL migrations under affected service migration dirs
- Update RLS policies/tests where sentinel behavior changes
- Add or update runbook under `docs/operations/runbooks/`
- Add EXPLAIN/planner evidence under `docs/reviews/post-ga/POST_GA_08_db_index_and_rls_polish/`
- Update migration inventory if required by repo practice

## §5. Schema / Proto

Schema changes must be forward-only. If replacing a nil UUID sentinel,
prefer explicit nullable handling or a dedicated setting that cannot
collide with a tenant UUID. No proto changes expected.

## §6. Audit-Chain Impact

No audit-chain row shape changes. RLS changes must not let tenants read
or write other tenants' samples. Migration smoke must prove policies
exist after apply.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Public role can read tokenizer samples | Migration/test fails |
| Nil UUID collides with legitimate tenant | Policy avoids sentinel ambiguity |
| Advisory lock DB connection stalls | Runbook provides detection and recovery |
| Freshness index is unused | Drop or document with planner evidence |
| Migration order broken | Postgres 16 smoke fails |

## §8. Acceptance Gates

- Postgres 16 migration apply smoke for affected migrations
- RLS tests for public revoke and tenant isolation
- EXPLAIN evidence for index keep/drop decision
- `git diff --check`
- Helm demo/production render if chart docs/config touched
- Evidence under `docs/reviews/post-ga/POST_GA_08_db_index_and_rls_polish/`

## §9. Review Checklist

1. Are grants least-privilege after migration?
2. Does the RLS policy avoid sentinel collision?
3. Is advisory-lock recovery actionable for operators?
4. Is the index decision backed by planner evidence?
5. Are migrations additive and ordered?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Full DB role redesign | GA roles already hardened |
| Automated DBA remediation | Operational roadmap |

## §11. Risk / Rollback

Grant and RLS changes can break readers. Include explicit role tests and
document required roles. Index drops can regress queries; keep EXPLAIN
evidence and roll back with a forward migration to recreate indexes if
needed.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer should inspect grants, RLS predicates, migration order, and
planner evidence.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Software Architect | Keep DB polish isolated from runtime behavior | POST_GA_08 |
| Backend Architect | RLS changes must preserve writer set_config pattern | §6 |
| Security Engineer | Public revoke is defense-in-depth but still testable | #146 |
| Database Optimizer | Index decision requires EXPLAIN, not intuition | #166 |
| SRE/Operations Architect | Advisory-lock keepalive belongs in runbook | #164 |

## §14. Merge Checklist

- [ ] SQL smoke passes
- [ ] RLS/grant tests pass
- [ ] EXPLAIN evidence recorded
- [ ] AIT review clean or Staff+ arbitration recorded
- [ ] Memory updated
