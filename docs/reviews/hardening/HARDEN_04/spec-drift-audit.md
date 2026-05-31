# HARDEN_04 Spec Drift Audit

Date: 2026-05-31
Branch: `harden/HARDEN_04_spec_impl_drift_reconciliation`

## Commands Run

```sh
rg -n 'recorded_at|cloudevent_payload|prediction_drift|drift_alert|STOP fallback|graceful STOP' docs services crates proto
rg -n 'spendguard\.prediction\.drift_alert|spendguard\.audit\.prediction_drift_alert|recorded_at|cloudevent_payload|payload_json|ingest_at' docs/stats-aggregator-spec-v1alpha1.md services/stats_aggregator services/canonical_ingest/migrations services/calibration_report/src/sql_queries.rs
rg -n 'STOP_RUN_PROJECTION|StopRunProjection|DECISION_UNSPECIFIED|unknown decision|unsupported.*decision|fail.*closed|DecisionStopped' services/egress_proxy/src services/sidecar/src sdk/python/src/spendguard/client.py proto/spendguard/sidecar_adapter/v1/adapter.proto docs/contract-dsl-spec-v1alpha2.md
rg -n 'reserved / actual|predicted_strategy_tokens / actual_output_tokens|P95 < 0\.95|C P95 < 0\.95|expected high ratio|systematic over-reservation|over-reservation outlier' docs/calibration-report-spec-v1alpha1.md services/calibration_report docs/slices/SLICE_13_calibration_report_cli.md
rg -n '\(tenant_id, model, agent_id, prompt_class_fingerprint\)|GROUP BY[^\n]*prompt_class_fingerprint|group by[^\n]*prompt_class_fingerprint|bucket key[^\n]*prompt_class_fingerprint' docs/stats-aggregator-spec-v1alpha1.md services/stats_aggregator services/canonical_ingest -g '*.rs' -g '*.sql' -g '*.md'
```

## Findings And Disposition

| Finding | Disposition |
|---|---|
| `docs/stats-aggregator-spec-v1alpha1.md` referenced the draft drift event type without the audit prefix. | Corrected to `spendguard.audit.prediction_drift_alert.v1alpha1`. Authoritative code is `services/stats_aggregator/src/drift_detector.rs` commit `f8dc34c`; durable append-result handling is hardened by HARDEN_03 merge `16f0194`. |
| `docs/stats-aggregator-spec-v1alpha1.md` showed hot aggregation SQL decoding `cloudevent_payload` and filtering on `recorded_at`. | Corrected to mirror columns plus `ingest_at` / `recorded_month`. Authoritative schema and implementation are canonical_ingest migration commit `8436cd4` and stats aggregation commit `5dcc0da`. |
| `docs/calibration-report-spec-v1alpha1.md` still used stale canonical query examples: `recorded_at`, `decision.cloudevent_payload`, non-audit drift alert type, and non-audit report-generated type. | Corrected to `event_time`, decoded `payload_json`, `spendguard.audit.prediction_drift_alert.v1alpha1`, and `spendguard.audit.calibration.report_generated.v1alpha1`. Authoritative implementation commits are `dabc6fb`, `15a3f3d`, and `c4fbab6`. |
| `docs/slices/SLICE_13_calibration_report_cli.md` still had the pre-HARDEN_04 self-audit type and `cloudevent_payload` row wording. | Corrected to the shipped audit-prefixed self-audit type and `payload_json`. |
| `docs/contract-dsl-spec-v1alpha2.md` described old-client fallback too permissively. | Reworded to fail-closed wire behavior while retaining SLICE_02 internal compatibility history. Authoritative behavior is egress_proxy commit `3035b54`, sidecar commit `c50b911`, SLICE_09 commit `cc20cb4`, and Python SDK HARDEN_03 commit `307eed4`. |
| `proto/spendguard/sidecar_adapter/v1/adapter.proto` comment implied old clients could gracefully continue. | Comment corrected only; no schema or generated code change. |
| AIT Round 1 found calibration-ratio direction drift: docs described reserved/actual while canonical SQL and formatters use `actual_output_tokens / predicted_<strategy>_tokens`. | Corrected the spec, README, text formatter label, recommendation wording, Strategy C threshold, and regression tests to the shipped actual/predicted metric. `cargo test --manifest-path services/calibration_report/Cargo.toml` passed with 94 unit tests, 7 CLI smoke tests, 3 sample-output tests, and 13 scenario tests. |
| AIT Round 1 found stats-aggregator §3.2 still naming `prompt_class_fingerprint` as the bucket key. | Corrected §3.2/§3.3 to key on `prompt_class` and define `prompt_class_fingerprint` as non-key audit metadata. The authoritative schema is `0016_output_distribution_cache.sql` primary key plus migration `0018` comments and stats_aggregator aggregation code. |
| AIT Round 2 found high Strategy A actual/predicted ratios rendered as expected because formatter special-cased A before threshold checks. | Fixed text and markdown formatters to apply warning/critical P95 checks before the Strategy A conservative-ratio label. Rule 1 now recommends on any strategy with P95 > 1.50, including a failed Strategy A ceiling. `cargo test --manifest-path services/calibration_report/Cargo.toml` passed with 97 unit tests, 7 CLI smoke tests, 3 sample-output tests, and 13 scenario tests. |

## Reviewed Remaining Grep Hits

| Hit | Decision |
|---|---|
| `docs/contract-dsl-spec-v1alpha2.md` still mentions `audit_outbox.cloudevent_payload`. | Kept. That table really has `cloudevent_payload` (`services/ledger/migrations/0009_audit_outbox.sql`, commit `ca83792`); the text now also names downstream `canonical_events.payload_json`. |
| `docs/calibration-report-spec-v1alpha1.md` still mentions `audit_outbox.cloudevent_payload`. | Kept. That line explicitly contrasts the ledger-only column with the canonical `payload_json` query path. |
| `proto/spendguard/sidecar_adapter/v1/adapter.proto` has `ForecastHorizon.recorded_at`. | Kept. This is a proto field for forecast sample recording, not the `canonical_events.ingest_at` column used by stats_aggregator. |
| `services/stats_aggregator/src/*` comments mention "not recorded_at". | Kept. The comments explicitly document why `ingest_at` is used. |
| `pass-through` remains in the contract spec and proto comments. | Kept only where it means internal evaluator compatibility or Signal 3 data flow; wire fallback text now says fail closed. |
| `spendguard.prediction.drift_alert` appears in calibration-report tests as a negative assertion. | Kept. The test asserts the legacy non-audit type is not used. |
| `services/canonical_ingest/migrations/0018_canonical_events_aggregator_mirror_columns.sql` mentions "bucket key -- NOT prompt_class_fingerprint". | Kept. It is the implementation-side proof for HARDEN_04's stats bucket-key correction. |

## Final Focused Grep Results

```text
$ rg -n "spendguard\.prediction\.drift_alert|cloudevent_payload|recorded_at|STOP fallback|graceful STOP|gracefully continu|pass-through|spendguard\.calibration\.report_generated" docs/stats-aggregator-spec-v1alpha1.md docs/contract-dsl-spec-v1alpha2.md docs/calibration-report-spec-v1alpha1.md docs/slices/SLICE_13_calibration_report_cli.md proto/spendguard/sidecar_adapter/v1/adapter.proto
docs/contract-dsl-spec-v1alpha2.md:93:> **Fail-closed ...
docs/contract-dsl-spec-v1alpha2.md:145:- ledger `audit_outbox.cloudevent_payload` and downstream `canonical_events.payload_json` ...
docs/contract-dsl-spec-v1alpha2.md:276:  // than gracefully continuing. Implemented by SLICE_02 commit `c50b911`
docs/contract-dsl-spec-v1alpha2.md:446:SLICE_02 merge `d5c5434` ...
docs/calibration-report-spec-v1alpha1.md:212:`audit_outbox.cloudevent_payload` column; see calibration-report commit
docs/contract-dsl-spec-v1alpha2.md:93:> **Fail-closed ...
docs/contract-dsl-spec-v1alpha2.md:145:- ledger `audit_outbox.cloudevent_payload` and downstream `canonical_events.payload_json` ...
docs/contract-dsl-spec-v1alpha2.md:276:  // than gracefully continuing. Implemented by SLICE_02 commit `c50b911`
docs/contract-dsl-spec-v1alpha2.md:446:SLICE_02 merge `d5c5434` ...
proto/spendguard/sidecar_adapter/v1/adapter.proto:358:  // === SLICE_09 additive: Signal 3 pass-through ===
proto/spendguard/sidecar_adapter/v1/adapter.proto:374:  // SLICE_02 stubbed RUN_* code pass-through but did NOT add this field
proto/spendguard/sidecar_adapter/v1/adapter.proto:409:    // gracefully continue; supported SDKs map STOP_RUN_PROJECTION to
proto/spendguard/sidecar_adapter/v1/adapter.proto:487:  google.protobuf.Timestamp recorded_at = 2;
```

The remaining hits are intentional and documented above. No stale stats-aggregator `recorded_at`, stale stats-aggregator `cloudevent_payload`, legacy `spendguard.prediction.drift_alert`, or legacy `spendguard.calibration.report_generated` normative reference remains in the touched predictor specs.

```text
$ rg -n "reserved / actual|predicted_strategy_tokens / actual_output_tokens|P95 < 0\.95|C P95 < 0\.95|expected high ratio|systematic over-reservation|over-reservation outlier" docs/calibration-report-spec-v1alpha1.md services/calibration_report docs/slices/SLICE_13_calibration_report_cli.md
<no matches>

$ rg -n "\(tenant_id, model, agent_id, prompt_class_fingerprint\)|GROUP BY[^\n]*prompt_class_fingerprint|group by[^\n]*prompt_class_fingerprint|bucket key[^\n]*prompt_class_fingerprint" docs/stats-aggregator-spec-v1alpha1.md services/stats_aggregator services/canonical_ingest -g '*.rs' -g '*.sql' -g '*.md'
services/canonical_ingest/migrations/0018_canonical_events_aggregator_mirror_columns.sql:127: ... Used as the stats_aggregator bucket key -- NOT prompt_class_fingerprint ...
```
