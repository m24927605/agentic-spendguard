# GA 08 - Scale and Performance SLO Proof

> **Branch**: `ga/GA_08_scale_performance_slo_proof`
> **Status**: implemented; adversarial review pending
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
- run_cost_projector recovery hot-path fix when GA load exposes invalid canonical DB SQL

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

Schema changes are not expected. If DB plans require an index, add a forward migration and include Postgres 16 migration verification.

Implementation did not add schema. The load gate exposed invalid parameterized `SET LOCAL` use and an audit recovery query against `audit_outbox` while run_cost_projector is configured with the canonical DB. GA_08 fixes this by using `SELECT set_config('app.current_tenant_id', $1, true)` and replaying from `canonical_events`.

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

Final local evidence on 2026-06-01:

- Source commit under test: `9198b05b8e43c3f87aab496735350a07f99dd138`
- `git_dirty=false`
- 100/100 operations completed; 100 logical workloads, 4 providers, 20 agents
- canonical delta 200; ledger outbox delta 200; pending outbox rows 0
- `verify_audit_columns.py` exit 0; DB plan gate exit 0
- p99: tokenizer 11.205ms, output_predictor 15.621ms, run_cost_projector 39.343ms, sidecar_decision 95.676ms, sidecar_emit 54.237ms, end_to_end 208.951ms

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
| Staff+ Panel | Final reset gate evidence with p99 and max latencies is sufficient for local GA_08 merge; cloud 10K tenant benchmark remains deferred | Evidence archived under `docs/reviews/ga-readiness/GA_08_scale_performance_slo_proof/` |

## §14. Merge Checklist

- [x] Load harness passes local scenario
- [x] DB explain checks pass
- [x] Evidence recorded
- [ ] AIT review clean or arbitration recorded
- [ ] Memory updated
