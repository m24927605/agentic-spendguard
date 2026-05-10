# SpendGuard observability pack (Phase 5 S23)

Bundle of SLO definitions + alert rules + dashboard + runbook
templates that operators apply to their k8s cluster.

## Files

| File                       | Purpose                                              |
|----------------------------|------------------------------------------------------|
| `prometheus-rules.yaml`    | PrometheusRule CRD with alert rules per SLO          |
| `grafana-dashboard.json`   | Reference Grafana dashboard panels                   |
| `slos.md` (in docs)        | SLO numeric targets + drill scenarios                |

## Apply

```bash
# Prometheus operator must be installed.
kubectl apply -f deploy/observability/prometheus-rules.yaml

# Import grafana-dashboard.json via Grafana UI Dashboards → Import.
```

## Tuning

Every threshold in `prometheus-rules.yaml` reflects the spec
defaults. Operators adjust per their workload:

| Threshold                                          | Default | Notes                                    |
|----------------------------------------------------|---------|------------------------------------------|
| Decision p99 latency                               | 250ms   | Tighten for low-latency products         |
| Decision error rate                                | 0.1%    | Stricter for revenue-critical workflows  |
| Ledger commit error rate                           | 0.05%   | Reflects sync-replica + audit invariant  |
| Outbox p99 lag                                     | 60s     | Workflow latency budget                  |
| Canonical ingest rejection rate                    | 0.5/s   | Should be ~0; spike indicates rotation   |
| Pricing snapshot staleness                         | 24h     | Tighten for high-volatility providers    |
| Provider reconciliation lag                        | 4h      | Loosen if provider APIs are slow         |
| Approval p99                                       | 5min    | Business-hours metric; tune to SLA       |
| Fencing takeovers per hour                         | 1       | Lease flap signal                        |

## Required metrics

The rules reference metrics that are emitted by the services.
canonical_ingest's `:9091/metrics` (S8) is the reference
implementation. The full required-metric list lives in
`docs/site/docs/operations/slos.md` with shipped vs followup
markers.

## Runbook stubs

Each alert points at `docs/operations/runbooks/<slo-id>-<name>.md`.
Phase 5 S23 ships the structure; the per-alert deep dives are the
S23-followup. Templates per alert are documented in slos.md.
