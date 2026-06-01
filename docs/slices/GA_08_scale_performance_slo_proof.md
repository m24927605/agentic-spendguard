# GA 08 - Scale and Performance SLO Proof

> **Branch**: `ga/GA_08_scale_performance_slo_proof`
> **Status**: R2 findings under fix
> **Spec ancestor(s)**: `ga-readiness-spec-v1alpha1.md`
> **Estimated change size**: medium-large; load harness, DB plan checks, performance evidence

---

## §0. TL;DR

Prove production-like scale invariants with real-stack load, high-cardinality tenant/run scenarios, p99 latency evidence, outbox lag, connection pool budget, and DB query plan checks. The local compose harness is a real-stack smoke gate; Contract §14 latency SLO certification remains the benchmark gate that enforces p99 < 50ms.

## §1. Architectural Context

SLICE_15 and HARDEN_02 produced benchmark evidence, but GA requires sustained production-like cardinality and database plan proof beyond shim-only paths.

## §2. Scope

- Real-stack load harness
- Scenario files for tenant/run/provider/plugin mix
- DB explain plan checks
- Connection pool budget documentation
- Performance results evidence
- run_cost_projector recovery hot-path fix when GA load exposes invalid canonical DB SQL
- run_cost_projector cold-cache recovery index when review exposes an unbounded canonical lookup

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
- Update `services/run_cost_projector/src/recovery.rs` and `signal_1.rs` only if the real-stack gate exposes SQL/RLS issues.

## §5. Schema / Config / API Impact

Schema changes are allowed only for index-backed plan correctness. If DB plans require an index, add a forward migration and include Postgres 16 migration verification.

Implementation adds one index-only migration. The load gate exposed invalid parameterized `SET LOCAL` use and an audit recovery query against `audit_outbox` while run_cost_projector is configured with the canonical DB. GA_08 fixes this by using `SELECT set_config('app.current_tenant_id', $1, true)` and replaying from `canonical_events`.

R2 review added `0021_canonical_events_run_recovery_index.sql` so `recover_from_audit_chain()` can seek the latest decision row by `(tenant_id, run_id_mirror, agent_id)` instead of scanning tenant-month decision volume.

## §6. Audit / Security / Operational Impact

Load tests must verify zero audit loss and must not disable security or replay protection for speed.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| benchmark p99 exceeds Contract §14 SLO | Slice cannot merge unless SLO is explicitly revised by Staff+ |
| local compose p99 exceeds smoke limit | Slice cannot merge until smoke threshold or environment issue is resolved |
| Seq scan over production-sized table | Review blocks unless intentionally bounded |
| Pool exhaustion deadlocks | Review blocks |
| Audit row count mismatch | Slice cannot merge |

## §8. Acceptance Gates

- `benchmarks/ga-load/run.sh --scenario benchmarks/ga-load/scenarios/local-100-tenants.yaml`
- Contract §14 SLO evidence cross-links to `spendguard-predictor-upgrade-benchmarks`, whose binary exits non-zero if SpendGuard decision p99 exceeds 50,000us
- `psql "$DATABASE_URL" -f scripts/db/explain-ga-plans.sql`
- Performance evidence includes p50, p95, p99, max, errors, and environment
- Audit row integrity probe passes after load

R1/R2 local evidence on 2026-06-01:

- Source commit under test: refreshed after R2 fixes
- `git_dirty=false`
- 100/100 operations completed; 100 logical workloads, 4 providers, 20 agents
- canonical delta 200; ledger outbox delta 200; pending outbox rows 0
- `verify_audit_columns.py` exit 0; DB plan gate exit 0
- Local compose smoke p99 is recorded as evidence, not as Contract §14 certification.
- Contract §14 benchmark ancestor: `docs/reviews/hardening/HARDEN_02/predictor-benchmark/RESULTS.md` reports SpendGuard p99 15,407us at burst 100, under the 50,000us gate enforced by the benchmark binary.

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
| Backend Architect | run_cost_projector must use canonical_events for replay because its configured DB is canonical, not ledger | Invalid `audit_outbox` recovery fallback fixed in-slice |
| Database Optimizer | Parameterized RLS tenant scope must use `set_config(..., true)`, not bind parameters inside `SET LOCAL` | Signal 1 and recovery SQL now match the locked RLS pattern |
| Performance/Database Architect | Local compose has one certified tenant; 100 logical workloads are represented through distinct run/agent/provider/model/prompt buckets | Security tenant assertions are not fabricated for scale evidence |
| Performance Architect | Local Docker p99 limits are smoke thresholds, not production Contract §14 certification | Scenario renamed `local_smoke_limits`; benchmark SLO evidence remains separate |
| Backend Architect | Projector fail-open timeout must stay inside the 50ms decision budget by default | Production/default timeout is 25ms; local smoke run overrides to 500ms explicitly |
| Database Optimizer | canonical recovery requires a seek path by tenant/run/agent | Added `canonical_events_run_recovery_idx` and GA plan gate coverage |

## §14. Merge Checklist

- [x] Load harness passes local scenario
- [x] DB explain checks pass
- [x] Evidence recorded
- [ ] AIT review clean or arbitration recorded
- [ ] Memory updated
