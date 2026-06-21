---
title: "Helm 部署"
description: >-
  用內附的 Helm chart 把 Agentic SpendGuard 部署到 Kubernetes — 包含 sidecar
  DaemonSet、ledger、canonical-ingest、dashboard，以及 chart.profile=production
  的 input gate（要求 database URL Secret 都備齊才放行到 pod）。
---


```bash
helm install spendguard ./charts/spendguard \
    --set postgres.existingSecret="spendguard-postgres-urls" \
    --set secrets.tls.existingSecret="spendguard-tls" \
    --set secrets.bundles.existingSecret="spendguard-bundles"
```

前置條件（PKI Secret 格式、bundle Secret 格式、webhook HMAC secret，以及 Postgres URL Secret 的 key）請參考 [charts/spendguard/README.md](https://github.com/m24927605/agentic-spendguard/blob/main/charts/spendguard/README.md)。

POC 限制：
- sidecar / outbox-forwarder / ttl-sweeper 預設強制 `replicas=1`。詳見 [POC vs GA gates](../poc-vs-ga.md)。
- Migration job 只是個 placeholder；ledger + canonical 的 SQL migration 請用你慣用的工具（sqitch / flyway / golang-migrate）自行套用。
