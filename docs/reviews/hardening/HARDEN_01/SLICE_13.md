# HARDEN_01 Retrospective — SLICE_13 calibration_report_cli

- Slice doc: `docs/slices/SLICE_13_calibration_report_cli.md`
- Merge commit: `83466fa`
- Merge base / first parent: `019c62f`
- Topic branch tip / second parent: `8857ba5`
- Diff command: `git diff 83466fa^1..83466fa`
- Diff size: 22 files, +5043/-7

## Review Focus

- Canonical proof-mode SQL correctness
- Drift alert event type matching
- Run-level recommendation counts
- verify-chain/self-audit behavior

## Findings

### Blocker — Canonical proof queries filtered event types that are not emitted

`services/calibration_report/src/sql_queries.rs` filtered decision and outcome rows as `spendguard.audit.decision.v1alpha1` and `spendguard.audit.outcome.v1alpha1`. The sidecar and canonical_ingest migrations use unversioned `spendguard.audit.decision` and `spendguard.audit.outcome`. Canonical proof mode would therefore return empty tier distribution and calibration ratios on real audit rows.

Fix: HARDEN_01 changes those SQL filters to the emitted unversioned event types and adds unit tests that reject the stale versioned filters.

### Major — Drift alert query used stale non-audit event type and stale payload shape

The report queried `spendguard.prediction.drift_alert.v1alpha1`, but stats_aggregator emits `spendguard.audit.prediction_drift_alert.v1alpha1` to route through ImmutableAuditLog. The query also expected `payload_json->>'bucket'`, while the emitted payload contains `model`, `agent_id`, and `prompt_class`.

Fix: HARDEN_01 changes the event type and builds a bucket label from the emitted payload keys, with tests pinning the audit-routed type.

### Major — RUN_* counts expected separate CloudEvent types

The report counted `spendguard.audit.run_budget_projection_exceeded.v1alpha1` and `spendguard.audit.run_drift_detected.v1alpha1`, but SLICE_09/10 encode RUN_* as `reason_codes` inside `spendguard.audit.decision` events.

Fix: HARDEN_01 changes run-level counts to read `payload_json->'reason_codes' ? 'RUN_*'`, with tests preventing separate-event-type regressions.

## Residual Checks Routed Later

- HARDEN_02 must run calibration-report against demo-produced canonical rows.
- HARDEN_04 must reconcile spec text that still mentions stale event-type names.

