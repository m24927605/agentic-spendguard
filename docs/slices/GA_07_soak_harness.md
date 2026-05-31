# GA 07 - Long-Running Soak Harness

> **Branch**: `ga/GA_07_soak_harness`
> **Status**: design
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

## §14. Merge Checklist

- [ ] Soak harness exists
- [ ] 30m local soak passes
- [ ] Evidence recorded
- [ ] AIT review clean or arbitration recorded
- [ ] Memory updated
