# GA 06 - Alerting, Runbooks, and Drills

> **Branch**: `ga/GA_06_alerting_runbooks_drills`
> **Status**: design
> **Spec ancestor(s)**: `ga-readiness-spec-v1alpha1.md`
> **Estimated change size**: medium-large; alert rules, runbooks, drill scripts

---

## §0. TL;DR

Make alerts actionable by pairing Prometheus rules with runbooks and at least one reproducible incident drill.

## §1. Architectural Context

Existing alert rules contain runbook links, but GA requires those runbooks to exist and be validated against real failure paths.

## §2. Scope

- Alert rule validation
- Runbook files for GA-critical alerts
- Incident drill scripts
- Alert-to-runbook index
- Evidence from at least one real drill

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| PagerDuty/Opsgenie integration | Operator environment |
| Full chaos suite | Future reliability phase |

## §4. File-Level Changes

- Update `deploy/observability/prometheus-rules.yaml`
- Add `docs/operations/runbooks/*.md`
- Add `docs/operations/drills/*.md`
- Add `scripts/observability/validate-alert-runbooks.sh`
- Add `tests/e2e/outbox_lag_recovery.sh` or equivalent drill
- Add evidence under `docs/reviews/ga-readiness/GA_06_alerting_runbooks_drills/`

## §5. Schema / Config / API Impact

No public API changes.

## §6. Audit / Security / Operational Impact

Runbooks must preserve audit chain integrity and must not instruct operators to disable security controls as first-line mitigation.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Alert references missing runbook | Validator exits non-zero |
| Runbook has no mitigation | Review blocks |
| Drill does not reproduce alert condition | Slice cannot merge |
| Mitigation suggests unsafe audit deletion | Review blocks |

## §8. Acceptance Gates

- `scripts/observability/validate-alert-runbooks.sh`
- Prometheus rules parse
- At least one drill runs and writes evidence
- `helm template spendguard charts/spendguard --set chart.profile=demo`

## §9. Review Checklist

1. Does every page-level alert have a runbook?
2. Do runbooks include detection, diagnosis, mitigation, rollback, and evidence?
3. Is at least one drill executed?
4. Are unsafe mitigations forbidden?
5. Are alert expressions based on real metrics?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| On-call vendor integration | Operator-specific |

## §11. Risk / Rollback

Revert rules/runbooks/scripts. Runtime behavior changes only if drill helpers are deployed, which this slice should avoid.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must check runbook usefulness, not just file existence.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| SRE/Operations Architect | Runbooks must be drill-backed | Drill evidence required |
| Security Engineer | Runbooks cannot bypass audit immutability | Unsafe mitigations blocked |

## §14. Merge Checklist

- [ ] Alert/runbook validator passes
- [ ] Prometheus rules parse
- [ ] Drill evidence recorded
- [ ] AIT review clean or arbitration recorded
- [ ] Memory updated
