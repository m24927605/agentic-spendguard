# GA 07 - Long-Running Soak Harness

> **Branch**: `ga/GA_07_soak_harness`
> **Status**: implementation complete; adversarial review R5 arbitrated and fixed
> **Spec ancestor(s)**: `ga-readiness-spec-v1alpha1.md`
> **Estimated change size**: medium; soak scripts and evidence format

---

## §0. TL;DR

Build and run a long-duration soak harness that periodically verifies audit chain integrity, lag, memory, stats freshness, replay cleanup, and plugin cert behavior.

## §1. Architectural Context

HARDEN proved correctness through targeted demos and tests. GA also needs sustained-runtime evidence that background workers, caches, outbox forwarding, and cert rotation remain healthy over time.

## §2. Scope

- Soak harness script
- Soak scenario configuration
- Periodic metrics snapshots
- Verify-chain or DB audit probes during soak
- Evidence summary format

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| Multi-day cloud soak | Future external validation |
| Full chaos testing | Future reliability phase |

## §4. File-Level Changes

- Add `benchmarks/ga-soak/`
- Add `scripts/soak/ga-soak.sh`
- Add `docs/operations/soak-runbook.md`
- Add evidence under `docs/reviews/ga-readiness/GA_07_soak_harness/`

## §5. Schema / Config / API Impact

No schema changes expected.

## §6. Audit / Security / Operational Impact

Soak must continuously prove no audit loss and no fail-open behavior during transient operational stress.

The local profile is a quiescent steady-state soak after one real demo traffic boot. It therefore treats canonical ingest freshness as telemetry and fails on canonical row-count regression or verify-chain failure rather than requiring new canonical rows during the no-traffic interval.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Service restarts during soak | Harness records restart and final status |
| Audit verify probe fails | Harness exits non-zero |
| Outbox lag unbounded | Harness exits non-zero |
| RSS growth exceeds threshold | Harness exits non-zero |

## §8. Acceptance Gates

- `scripts/soak/ga-soak.sh --duration 30m --profile local` for slice merge
- Soak harness supports `--duration 24h` for release gate
- Evidence includes periodic snapshots
- Relevant demo mode boots before soak

## §9. Review Checklist

1. Does the harness run real services?
2. Does it collect periodic evidence?
3. Does it check audit integrity during the run?
4. Does it fail on unbounded lag or memory growth?
5. Does it document the 24h release-grade invocation?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Mandatory 72h cloud soak | Requires external environment and cost budget |

## §11. Risk / Rollback

Revert scripts/docs/evidence. No runtime code changes unless review finds missing metrics that must be added.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must reject shim-only or final-status-only soak evidence.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Performance/Database Architect | Soak must include periodic snapshots | Evidence format requires them |
| SRE/Operations Architect | 30m local gate plus 24h release command | Practical slice gate and stronger release gate |
| Implementer | Use real docker-compose demo stack, SVID/mTLS tests, verify-chain, outbox/leader metrics, stats cycles, and container memory snapshots | `scripts/soak/ga-soak.sh` writes JSONL snapshots and JSON summary |
| Implementer | Corrected no-traffic freshness gate after a 30m run exposed a harness false positive | Canonical count regression + verify-chain are blockers; canonical freshest age remains telemetry |
| Implementer | Ran exact 30m local gate | PASS: 27 snapshots, final elapsed 1814s, pending 0, lag 0, stats cycles 31 |
| Codex adversarial reviewer R1 | Snapshot probe failures could fail open under Bash `errexit` suppression; generated summary lacked GA §7 evidence metadata | Fixed explicit probe return guards and added commit, branch, date, command line, environment profile, machine descriptor, cluster descriptor, and git cleanliness metadata |
| Implementer | Reran exact 30m local gate after R1 fix | PASS: 28 snapshots, final elapsed 1838s, pending 0, lag 0, stats cycles 31, `git_dirty=false` |
| Codex adversarial reviewer R2 | Stopped containers could fail `docker stats` or metrics probes before inspect details were recorded; zero intervals could busy-loop | Snapshot probes now capture DB/metrics failures into failure evidence, still record inspect status, and reject non-positive duration/interval values |
| Implementer | Ran R2 targeted gates | PASS: zero interval rejected; stopped tokenizer produced a structured fail summary with exited/unhealthy status; 30s happy-path smoke passed |
| Codex adversarial reviewer R3 | Required 30m evidence was stale relative to final script changes; `docker inspect` failures could still return before structured evidence | Fixed inspect failure handling to record concise failure evidence and reran the exact 30m local gate on clean source commit `ae318aa3cc1f7cd30902c2447b4f21343abf8b0a` |
| Implementer | Ran R3 targeted gates | PASS: removed tokenizer before snapshot produced structured failures for docker stats, tokenizer metrics, and concise docker inspect missing-object output; 30m local gate passed with 27 snapshots, pending 0, lag 0, stats cycles 31, `git_dirty=false` |
| Codex adversarial reviewer R4 | Metrics and HTTP probes could hang without bounded timeouts; pre-snapshot SVID test failures could exit before writing summary evidence | Added curl/wget timeouts, hardened missing option values, and routed Rust/Python preflight test failures through `ga_soak_summary.json` |
| Implementer | Ran R4 targeted gates | PASS: missing `--duration` value exits 2 with usage; fake cargo failure writes `result=fail`, `snapshot_count=0`, and preflight failure detail; exact 30m local gate passed on clean source commit `31631db760531022774c49d38d51ee5a4fb89e2a` with 27 snapshots, pending 0, lag 0, stats cycles 31, `git_dirty=false` |
| Codex adversarial reviewer R5 | Stats cache checks could pass with `output_distribution_cache_rows=0`; soak timer started before Rust/Python preflight and stats warmup | Max review rounds reached; Staff+ panel arbitration required by §12 |
| Staff+ arbitration panel | Software Architect, Backend Architect, Security Engineer, Database Optimizer, and domain expert all voted fix-in-slice | Implemented final fixes instead of deferring: stats aggregation now joins sparse outcome rows to decision mirrors, soak preflight waits for required stats cache rows/freshness, and sustained-window timing starts after all preflight gates |
| Implementer | Ran R5 targeted gates | PASS: full stats_aggregator test suite; helm demo/production template; output-cache negative; run-cache negative; slow-preflight timing; 30s happy-path smoke |
| Implementer | Ran exact 30m local gate after R5 arbitration fix | PASS: clean source commit `89a233153e68d7863dc2ab28dfea2a6dee466ff7`, 28 snapshots, snapshot window 1800s, pending 0, lag 0, stats cycles 31, output cache rows 1 freshness 8s, run cache rows 2 freshness 9s, `git_dirty=false` |

## §14. Merge Checklist

- [x] Soak harness exists
- [x] 30m local soak passes
- [x] Evidence recorded
- [x] AIT review clean or arbitration recorded
- [ ] Memory updated
