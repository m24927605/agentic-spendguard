# GA 05 - Observability Dashboards

> **Branch**: `ga/GA_05_observability_dashboards`
> **Status**: implementation review
> **Spec ancestor(s)**: `ga-readiness-spec-v1alpha1.md`
> **Estimated change size**: medium; metrics inventory and dashboard assets

---

## §0. TL;DR

Close the metric inventory and ship a dashboard pack that references real SpendGuard metrics for predictor, audit, ledger, plugin, and control-plane health.

## §1. Architectural Context

Existing observability docs and Prometheus rules predate the predictor upgrade hardening. GA needs dashboards that reflect actual emitted metrics, not aspirational names.

## §2. Scope

- Metric inventory
- Replacement of GA SLO placeholder metrics in output_predictor and run_cost_projector
- Grafana dashboard JSON
- Dashboard documentation
- Scrape validation script
- Predictor-specific SLO panels

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| Alert thresholds and runbooks | GA_06 |
| Long-running soak evidence | GA_07 |

## §4. File-Level Changes

- Add or update `deploy/observability/grafana-dashboard.json`
- Add `docs/operations/metrics-inventory.md`
- Update `deploy/observability/README.md`
- Add `scripts/observability/validate-dashboard-metrics.sh`
- Update output_predictor metrics emission where GA dashboard/SLO panels require real values
- Update run_cost_projector metrics emission where GA dashboard/SLO panels require real values
- Add evidence under `docs/reviews/ga-readiness/GA_05_observability_dashboards/`

## §5. Schema / Config / API Impact

Metric names become an operator contract. GA_05 also adds a narrow ledger migration, `0053_audit_outbox_pending_age_idx.sql`, to index `audit_outbox(recorded_at) WHERE pending_forward = TRUE` for the audit lag gauge.

## §6. Audit / Security / Operational Impact

Dashboards must expose audit lag, canonical ingest rejects, replay dedup, drift alerts, SVID failures, and predictor latency without leaking PII.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Dashboard references missing metric | Validator exits non-zero |
| Dashboard JSON invalid | Validator exits non-zero |
| Metric label leaks prompt text | Review blocks |
| Predictor panel lacks p99 | Review blocks |

## §8. Acceptance Gates

- `scripts/observability/validate-dashboard-metrics.sh`
- Dashboard JSON parses
- Metric inventory maps every dashboard metric to service and endpoint
- No GA SLO dashboard panel uses placeholder `0` metrics
- `make demo-up DEMO_MODE=default` if scrape validation requires live metrics

## §9. Review Checklist

1. Does every metric exist or have a non-GA marker?
2. Are predictor p99 and audit lag visible?
3. Are labels low-cardinality and PII-free?
4. Does dashboard JSON import cleanly?
5. Does docs text match Prometheus rule names?
6. Are output_predictor and run_cost_projector GA metrics real rather than placeholders?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Managed Grafana provisioning | Environment-specific |

## §11. Risk / Rollback

Revert dashboard and inventory files. No runtime behavior changes unless metric emission changes are required by review.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must verify metrics are real and labels do not introduce cardinality or PII risk.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| SRE/Operations Architect | Dashboards and alerts are separate slices | GA_05 owns dashboards only |
| Performance/Database Architect | p99 and lag panels are mandatory | Tail metrics required |
| R1 codex adversarial review | Inventory endpoints, cache ratio, stale lag, output predictor live scrape, and Grafana link had to be fixed | Real endpoint validator, `increase` cache ratio, leader gauge, live scrape evidence, and empty dashboard links adopted |
| R2 codex adversarial review | Leader-filtered lag could hide no-leader backlog growth | Every outbox-forwarder pod refreshes pending oldest-row age; leader count is shown separately |
| R4 codex adversarial review | Raw pipe characters in inventory label cells could break Markdown parsing and weaken validator checks | Label enums use comma separators and validator now rejects inventory rows that do not have exactly seven cells |
| R5 Staff+ arbitration | Rust formatting still regressed after max review rounds; DB reviewer required proof that every-pod lag polling is index-backed | Panel voted fix anyway. Rustfmt applied only to GA_05-touched files; `0053_audit_outbox_pending_age_idx.sql` added; fresh demo bootstrap and EXPLAIN verified index-only lag plan |

## §14. Merge Checklist

- [x] Metrics inventory exists
- [x] Dashboard JSON validates
- [x] Metric validator passes
- [x] AIT review clean or arbitration recorded
- [ ] Memory updated
