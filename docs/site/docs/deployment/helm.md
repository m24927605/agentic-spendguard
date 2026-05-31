---
description: >-
  Deploy Agentic SpendGuard on Kubernetes with the included Helm chart — sidecar
  DaemonSet, ledger, canonical-ingest, dashboard, and the chart.profile=production
  input gates that require database URL Secrets before they reach a pod.
---

# Helm deployment

```bash
helm install spendguard ./charts/spendguard \
    --set postgres.existingSecret="spendguard-postgres-urls" \
    --set secrets.tls.existingSecret="spendguard-tls" \
    --set secrets.bundles.existingSecret="spendguard-bundles"
```

See [charts/spendguard/README.md](https://github.com/m24927605/agentic-flow-cost-evaluation/blob/main/charts/spendguard/README.md)
for prerequisites (PKI Secret format, bundle Secret format, webhook
HMAC secret, and Postgres URL Secret keys).

POC limits:
- `replicas=1` enforced by default for sidecar / outbox-forwarder /
  ttl-sweeper. See [POC vs GA gates](../poc-vs-ga.md).
- Migration job is a placeholder; apply ledger + canonical SQL
  migrations via your preferred tool (sqitch / flyway / golang-migrate).
