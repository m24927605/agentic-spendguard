# SpendGuard GA Metrics Inventory

Status: GA_05 locked metric inventory for `deploy/observability/grafana-dashboard.json`.

Every dashboard metric below is emitted by a service `/metrics` endpoint and uses bounded labels. Tenant ids, run ids, decision ids, prompt text, model text, and provider-owned identifiers are not exported as labels.

| Metric | Service | Endpoint | Source | Labels | Dashboard panel | PII/Cardinality |
|---|---|---|---|---|---|---|
| `spendguard_output_predictor_predict_latency_seconds_bucket` | output_predictor | `:9100/metrics` | `services/output_predictor/src/main.rs` | `le` | Output Predictor p99 | Bounded histogram bucket |
| `spendguard_output_predictor_predict_total` | output_predictor | `:9100/metrics` | `services/output_predictor/src/main.rs` | `outcome=ok,err` | Output Predictor RPC Rate | Bounded enum |
| `spendguard_output_predictor_cache_hit_total` | output_predictor | `:9100/metrics` | `services/output_predictor/src/main.rs` | none | Strategy B Cache Hit Ratio | No labels |
| `spendguard_output_predictor_cache_lookup_total` | output_predictor | `:9100/metrics` | `services/output_predictor/src/main.rs` | none | Strategy B Cache Hit Ratio | No labels |
| `spendguard_run_cost_projector_project_latency_seconds_bucket` | run_cost_projector | `:9102/metrics` | `services/run_cost_projector/src/main.rs` | `le` | Run Cost Projector p99 | Bounded histogram bucket |
| `spendguard_run_cost_projector_project_total` | run_cost_projector | `:9102/metrics` | `services/run_cost_projector/src/main.rs` | `outcome=ok,err` | Run Cost Projector RPC Rate | Bounded enum |
| `spendguard_run_cost_projector_terminate_run_total` | run_cost_projector | `:9102/metrics` | `services/run_cost_projector/src/main.rs` | `outcome=ok,err` | Run Cost Projector RPC Rate | Bounded enum |
| `spendguard_outbox_pending_oldest_age_seconds` | outbox_forwarder | `:9096/metrics` | `services/outbox_forwarder/src/metrics.rs` | none | Audit Outbox Lag | No labels |
| `spendguard_outbox_forwarder_is_leader` | outbox_forwarder | `:9096/metrics` | `services/outbox_forwarder/src/metrics.rs` | none | Outbox Forwarder Leaders | No labels |
| `spendguard_ingest_events_deduped_total` | canonical_ingest | `:9091/metrics` | `services/canonical_ingest/src/metrics.rs` | `route=enforcement,observability` | Replay Dedup Rate | Bounded enum |
| `spendguard_ingest_events_rejected_invalid_signature_total` | canonical_ingest | `:9091/metrics` | `services/canonical_ingest/src/metrics.rs` | `route=enforcement,observability` | Canonical Ingest Rejects | Bounded enum |
| `spendguard_ingest_events_quarantined_total` | canonical_ingest | `:9091/metrics` | `services/canonical_ingest/src/metrics.rs` | `reason` from fixed quarantine enum | Canonical Ingest Rejects | Bounded enum |
| `spendguard_stats_aggregator_drift_alerts_total` | stats_aggregator | `:9101/metrics` | `services/stats_aggregator/src/main.rs` | none | Drift Alerts | No labels |
| `spendguard_tokenizer_drift_alert_oncall_escalation_total` | tokenizer | `:9099/metrics` | `services/tokenizer/src/main.rs` | none | Drift Alerts | No labels |
| `customer_predictor_call_total` | output_predictor | `:9100/metrics` | `services/output_predictor/src/main.rs` | `outcome=success,fall_to_b` | Strategy C Fall-to-B | Bounded enum |
| `customer_predictor_failure_mode_total` | output_predictor | `:9100/metrics` | `services/output_predictor/src/main.rs` | `mode` from Strategy C failure enum | Strategy C Fall-to-B; SVID and Tenant Isolation Failures | Bounded enum |
| `customer_predictor_tenant_isolation_violation_total` | output_predictor | `:9100/metrics` | `services/output_predictor/src/main.rs` | none | SVID and Tenant Isolation Failures | No labels |
| `spendguard_control_plane_route_calls_total` | control_plane | `:9094/metrics` | `services/control_plane/src/metrics.rs` | `route`, `outcome=ok,err` from fixed endpoint enum | Control Plane and Ledger Errors | Bounded enum |
| `spendguard_ledger_handler_calls_total` | ledger | `:9092/metrics` | `services/ledger/src/metrics.rs` | `handler`, `outcome=ok,err` from fixed handler enum | Control Plane and Ledger Errors | Bounded enum |

## Derived Dashboard Signals

The dashboard derives ratio and p99 panels from raw metric contracts rather than emitting precomputed gauges:

| Signal | PromQL shape | Raw metric contract |
|---|---|---|
| Output predictor p99 | `histogram_quantile(0.99, sum(rate(..._bucket[5m])) by (le))` | cumulative histogram buckets |
| Run cost projector p99 | `histogram_quantile(0.99, sum(rate(..._bucket[5m])) by (le))` | cumulative histogram buckets |
| Strategy B cache hit ratio | `increase(hit_total[5m]) / clamp_min(increase(lookup_total[5m]), 1)` | monotonic counters |
| Audit lag | `max(spendguard_outbox_pending_oldest_age_seconds)` | every pod refreshes the Postgres-backed oldest pending row age, so no-leader states do not mask backlog growth |
| Outbox leader count | `sum(spendguard_outbox_forwarder_is_leader)` | expected to be exactly 1 in a healthy deployment |
| SVID failure proxy | `customer_predictor_failure_mode_total{mode="tls_error"}` | Strategy C TLS failure enum |

## Validation

Run:

```bash
scripts/observability/validate-dashboard-metrics.sh
```

The validator parses the Grafana JSON, extracts PromQL metric references, verifies that every dashboard metric appears in this inventory, verifies the source path contains the metric name, and rejects placeholder `vector(0)` dashboard panels plus the legacy predictor/projector placeholder lines.
