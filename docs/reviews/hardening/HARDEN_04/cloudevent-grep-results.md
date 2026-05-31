# HARDEN_04 CloudEvent Grep Results

Date: 2026-05-31
Branch: `harden/HARDEN_04_spec_impl_drift_reconciliation`

## Spec Types Reviewed

```sh
rg -o 'spendguard\.[A-Za-z0-9_.:-]+' docs/stats-aggregator-spec-v1alpha1.md docs/contract-dsl-spec-v1alpha2.md docs/calibration-report-spec-v1alpha1.md docs/slices/SLICE_13_calibration_report_cli.md | sort -u
```

```text
docs/calibration-report-spec-v1alpha1.md:spendguard.audit.calibration.report_generated.v1alpha1
docs/calibration-report-spec-v1alpha1.md:spendguard.audit.decision
docs/calibration-report-spec-v1alpha1.md:spendguard.audit.outcome
docs/calibration-report-spec-v1alpha1.md:spendguard.audit.prediction_drift_alert.v1alpha1
docs/contract-dsl-spec-v1alpha2.md:spendguard.ai
docs/contract-dsl-spec-v1alpha2.md:spendguard.contract.policy_changed
docs/contract-dsl-spec-v1alpha2.md:spendguard.exceptions
docs/slices/SLICE_13_calibration_report_cli.md:spendguard.audit.calibration.report_generated.v1alpha1
docs/stats-aggregator-spec-v1alpha1.md:spendguard.audit.decision
docs/stats-aggregator-spec-v1alpha1.md:spendguard.audit.outcome
docs/stats-aggregator-spec-v1alpha1.md:spendguard.audit.prediction_drift_alert.v1alpha1
```

`spendguard.ai` and `spendguard.exceptions` are not CloudEvent types. The CloudEvent types requiring proof are listed below.

## Type Verification

| CloudEvent type | Status | Evidence |
|---|---|---|
| `spendguard.audit.prediction_drift_alert.v1alpha1` | Emitted and queried. | `services/stats_aggregator/src/drift_detector.rs` defines `PREDICTION_DRIFT_ALERT_EVENT_TYPE`; `services/calibration_report/src/sql_queries.rs` filters the same type; `services/stats_aggregator/tests/cycle_e2e_postgres.rs` seeds/verifies the same type. |
| `spendguard.audit.outcome` | Emitted and queried. | Ledger handlers and migrations emit/store this unversioned type; calibration-report SQL and stats-aggregator SQL read it. |
| `spendguard.audit.decision` | Emitted and queried. | Ledger `reserve_set`, `record_denied_decision`, replay, invoice reconcile, and calibration-report SQL use this unversioned type. |
| `spendguard.audit.calibration.report_generated.v1alpha1` | Emitted by calibration-report self-audit. | `services/calibration_report/src/self_audit.rs` defines `EVENT_TYPE_REPORT`; README uses the same type. |
| `spendguard.contract.policy_changed` | Planned/future marker only. | Present only in contract-dsl Trace text; no current emission site exists. It is not used by stats_aggregator or predictor hot path. |

## Key Grep Excerpts

```text
$ rg -n "spendguard\.audit\.prediction_drift_alert\.v1alpha1|PREDICTION_DRIFT_ALERT_EVENT_TYPE|prediction_drift_alert" services/stats_aggregator services/calibration_report docs/stats-aggregator-spec-v1alpha1.md
docs/stats-aggregator-spec-v1alpha1.md:105:**Output** ... `spendguard.audit.prediction_drift_alert.v1alpha1`
docs/stats-aggregator-spec-v1alpha1.md:390:type: spendguard.audit.prediction_drift_alert.v1alpha1
services/calibration_report/src/sql_queries.rs:349:    AND event_type = 'spendguard.audit.prediction_drift_alert.v1alpha1'
services/stats_aggregator/src/drift_detector.rs:46:pub const PREDICTION_DRIFT_ALERT_EVENT_TYPE: &str =
services/stats_aggregator/src/drift_detector.rs:47:    "spendguard.audit.prediction_drift_alert.v1alpha1";
services/stats_aggregator/tests/cycle_e2e_postgres.rs:408:        "spendguard.audit.prediction_drift_alert.v1alpha1",
```

```text
$ rg -n "spendguard\.audit\.calibration\.report_generated\.v1alpha1|spendguard\.calibration\.report_generated" docs/calibration-report-spec-v1alpha1.md services/calibration_report/src/self_audit.rs services/calibration_report/README.md
services/calibration_report/README.md:147:`spendguard.audit.calibration.report_generated.v1alpha1` CloudEvent
services/calibration_report/src/self_audit.rs:77:const EVENT_TYPE_REPORT: &str = "spendguard.audit.calibration.report_generated.v1alpha1";
docs/calibration-report-spec-v1alpha1.md:340:... `spendguard.audit.calibration.report_generated.v1alpha1` CloudEvent ...
```

```text
$ rg -n "spendguard\.audit\.outcome|spendguard\.audit\.decision" services/ledger services/calibration_report services/stats_aggregator docs/stats-aggregator-spec-v1alpha1.md
services/ledger/src/handlers/commit_estimated.rs:172:        "event_type":                       "spendguard.audit.outcome",
services/ledger/src/handlers/record_denied_decision.rs:119:        "event_type":                    "spendguard.audit.decision",
services/ledger/src/handlers/reserve_set.rs:182:        "event_type":                    "spendguard.audit.decision",
services/ledger/src/handlers/release.rs:153:        "event_type":                       "spendguard.audit.outcome",
services/calibration_report/src/sql_queries.rs:123:  AND event_type = 'spendguard.audit.decision'
services/calibration_report/src/sql_queries.rs:200:   AND outcome.event_type = 'spendguard.audit.outcome'
services/stats_aggregator/src/aggregation.rs:247:        WHERE event_type = 'spendguard.audit.outcome'
```

```text
$ rg -n "spendguard\.contract\.policy_changed" docs services crates proto
docs/contract-dsl-spec-v1alpha2.md:213:- audit event `spendguard.contract.policy_changed` ...
```

No current code emits `spendguard.contract.policy_changed`; it remains documented as a future Trace event, not an implemented predictor-upgrade event.
