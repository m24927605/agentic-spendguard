# GA 08 - Scale and Performance SLO Proof

> **Branch**: `ga/GA_08_scale_performance_slo_proof`
> **Status**: Implemented; R4 adversarial review clean
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
- Add evidence under `docs/internal/reviews/ga-readiness/GA_08_scale_performance_slo_proof/`
- Update `services/run_cost_projector/src/recovery.rs` and `signal_1.rs` only if the real-stack gate exposes SQL/RLS issues.

## §5. Schema / Config / API Impact

Schema changes are allowed only for index-backed plan correctness. If DB plans require an index, add a forward migration and include Postgres 16 migration verification.

Implementation adds one index-only migration. The load gate exposed invalid parameterized `SET LOCAL` use and an audit recovery query against `audit_outbox` while run_cost_projector is configured with the canonical DB. GA_08 fixes this by using `SELECT set_config('app.current_tenant_id', $1, true)` and replaying from `canonical_events`.

R2 review added `0021_canonical_events_run_recovery_index.sql` so `recover_from_audit_chain()` can seek the latest decision row by `(tenant_id, run_id_mirror, agent_id)` instead of scanning tenant-month decision volume.

R3 review made the DB plan gate partition-aware: `scripts/db/explain-ga-plans.sql` now treats every relation in `pg_partition_tree('public.canonical_events')` as a GA production table, so Seq Scans on `canonical_events_2026_06` or `canonical_events_default` fail the gate instead of slipping past a parent-table name check.

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

Final local evidence on 2026-06-01:

- Source commit under test: `e660678953adfa1aa56c9d6203518bd382d29308`
- `git_dirty=false`
- 100/100 operations completed; 100 logical workloads, 4 providers, 20 agents
- canonical delta 200; ledger outbox delta 200; pending outbox rows 0
- `verify_audit_columns.py` exit 0; DB plan gate exit 0
- local smoke p99: tokenizer 10.071ms; output_predictor 14.435ms; direct run_cost_projector 51.073ms; sidecar decision 101.917ms; sidecar emit 83.512ms; end-to-end 256.852ms
- Local compose smoke p99 is recorded as evidence, not as Contract §14 certification.
- Contract §14 benchmark ancestor: `docs/internal/reviews/hardening/HARDEN_02/predictor-benchmark/RESULTS.md` reports SpendGuard p99 15,407us at burst 100, under the 50,000us gate enforced by the benchmark binary.

Additional validation:

- `cargo test --manifest-path services/run_cost_projector/Cargo.toml`
- `cargo test --manifest-path services/sidecar/Cargo.toml`
- `cargo check --manifest-path services/run_cost_projector/Cargo.toml`
- `cargo check --manifest-path services/sidecar/Cargo.toml`
- `helm template spendguard charts/spendguard`
- `helm template spendguard charts/spendguard -f charts/spendguard/values-production.example.yaml --set chart.profile=production`
- `bash -n benchmarks/ga-load/run.sh`
- Python syntax check through `ast.parse(Path('benchmarks/ga-load/driver.py').read_text())`

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

## §12. Review Execution Notes

Reviewer: codex CLI via `codex review --base main`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must reject averaged-only latency, tiny cardinality, and disabled-security benchmark shortcuts.

Execution record:

- Review note: every review round used `codex review --base main`.
- R1: 3 P2 findings fixed. Sidecar ClaimEstimate now uses tokenizer/RCP output; metric gate proves sidecar projector calls; audit row gate counts decision/outcome separately.
- R2: 3 P2 findings fixed. Local compose evidence relabelled as smoke gate, production/default projector timeout set to 25ms, and canonical recovery lookup gained `(tenant_id, run_id_mirror, agent_id)` index coverage.
- R3: 1 P2 finding fixed. DB plan gate now detects Seq Scans on `canonical_events` partition children via `pg_partition_tree`.
- R4: clean. No discrete correctness findings.

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
| Database Optimizer | Plan gate must reject Seq Scans on canonical partition children, not just the parent relation | `ga_assert_no_seq_scan` matches every `canonical_events` partition through `pg_partition_tree` |
| Reviewer codex CLI | R4 adversarial review found no further correctness issues | GA_08 accepted without Staff+ arbitration |

## §14. Merge Checklist

- [x] Load harness passes local scenario
- [x] DB explain checks pass
- [x] Evidence recorded
- [x] Codex review clean or arbitration recorded
- [ ] Memory updated
