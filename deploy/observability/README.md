# SpendGuard Observability Pack

GA_05 ships the reference Grafana dashboard and metric inventory for predictor-era operations. The dashboard references only metrics emitted by service `/metrics` endpoints; alerts and detailed runbooks remain in the GA_06 slice.

## Files

| File | Purpose |
|---|---|
| `deploy/observability/grafana-dashboard.json` | Grafana dashboard for predictor, run projector, audit, ingest, plugin, control-plane, and ledger health |
| `docs/operations/metrics-inventory.md` | Metric-to-service inventory and low-cardinality label contract |
| `scripts/observability/validate-dashboard-metrics.sh` | Dashboard JSON and metric inventory validator |
| `deploy/observability/prometheus-rules.yaml` | Pre-existing alert-rule pack; GA_06 reconciles alert thresholds and runbooks |

## Validate

```bash
scripts/observability/validate-dashboard-metrics.sh
```

The validator checks that Grafana JSON parses, dashboard PromQL metrics appear in `docs/operations/metrics-inventory.md`, each inventory source path contains the metric string, predictor/projector placeholder metrics are gone, and the required p99, audit lag, replay dedup, and SVID failure panels are present.

## Import

```bash
# Prometheus operator rules, reconciled further by GA_06.
kubectl apply -f deploy/observability/prometheus-rules.yaml
```

Import `deploy/observability/grafana-dashboard.json` through Grafana's dashboard import flow and bind `${DS_PROMETHEUS}` to the cluster Prometheus datasource.

## Dashboard Coverage

| Area | Panels |
|---|---|
| Predictor latency and health | Output Predictor p99, Strategy B Cache Hit Ratio, Output Predictor RPC Rate |
| Run-level projection | Run Cost Projector p99, Run Cost Projector RPC Rate |
| Audit chain | Audit Outbox Lag, Outbox Forwarder Leaders, Replay Dedup Rate |
| Canonical ingest | Canonical Ingest Rejects |
| Drift | Drift Alerts |
| Customer plugin path | Strategy C Fall-to-B, SVID and Tenant Isolation Failures |
| Platform APIs | Control Plane and Ledger Errors |

## Metric Discipline

Dashboard labels are restricted to bounded enums such as `outcome`, `route`, `reason`, `mode`, `handler`, and histogram `le`. Metrics must not add tenant ids, run ids, decision ids, prompt text, or model text as labels.
