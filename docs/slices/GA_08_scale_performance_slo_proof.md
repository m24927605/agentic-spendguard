# GA 08 - Scale and Performance SLO Proof

> **Branch**: `ga/GA_08_scale_performance_slo_proof`
> **Status**: design
> **Spec ancestor(s)**: `ga-readiness-spec-v1alpha1.md`
> **Estimated change size**: medium-large; load harness, DB plan checks, performance evidence

---

## §0. TL;DR

Prove production-like scale behavior with real-stack load, high-cardinality tenant/run scenarios, p99 latency, outbox lag, connection pool budget, and DB query plan checks.

## §1. Architectural Context

SLICE_15 and HARDEN_02 produced benchmark evidence, but GA requires sustained production-like cardinality and database plan proof beyond shim-only paths.

## §2. Scope

- Real-stack load harness
- Scenario files for tenant/run/provider/plugin mix
- DB explain plan checks
- Connection pool budget documentation
- Performance results evidence

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| Cloud benchmark certification | Future external validation |
| Automatic autoscaling policy | Future platform work |

## §4. File-Level Changes

- Add `benchmarks/ga-load/`
- Add `benchmarks/ga-load/scenarios/*.yaml`
- Add `scripts/db/explain-ga-plans.sql`
- Add `docs/operations/performance-slo-proof.md`
- Add evidence under `docs/reviews/ga-readiness/GA_08_scale_performance_slo_proof/`

## §5. Schema / Config / API Impact

Schema changes are not expected. If DB plans require an index, add a forward migration and include Postgres 16 migration verification.

## §6. Audit / Security / Operational Impact

Load tests must verify zero audit loss and must not disable security or replay protection for speed.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| p99 exceeds SLO | Slice cannot merge unless SLO is explicitly revised by Staff+ |
| Seq scan over production-sized table | Review blocks unless intentionally bounded |
| Pool exhaustion deadlocks | Review blocks |
| Audit row count mismatch | Slice cannot merge |

## §8. Acceptance Gates

- `benchmarks/ga-load/run.sh --scenario benchmarks/ga-load/scenarios/local-100-tenants.yaml`
- `psql "$DATABASE_URL" -f scripts/db/explain-ga-plans.sql`
- Performance evidence includes p50, p95, p99, max, errors, and environment
- Audit row integrity probe passes after load

## §9. Review Checklist

1. Is the test real-stack, not shim-only?
2. Does evidence include p99 and max?
3. Are tenant/run cardinalities meaningful?
4. Are DB plans index-backed?
5. Does load preserve audit and replay invariants?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| 10K tenant cloud benchmark | Requires external cluster capacity |

## §11. Risk / Rollback

Revert harness/docs/evidence. If a migration is added, rollback follows GA_04 policy and may be forward-fix only.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must reject averaged-only latency, tiny cardinality, and disabled-security benchmark shortcuts.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Performance/Database Architect | Real-stack load and DB plan checks are required | GA_08 owns performance proof |
| Security Engineer | Load cannot disable replay or SVID controls | Security invariants preserved |

## §14. Merge Checklist

- [ ] Load harness passes local scenario
- [ ] DB explain checks pass
- [ ] Evidence recorded
- [ ] AIT review clean or arbitration recorded
- [ ] Memory updated
