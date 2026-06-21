---
title: "Helm 部署"
description: >-
  用自带的 Helm chart 把 Agentic SpendGuard 部署到 Kubernetes —— sidecar
  DaemonSet、ledger、canonical-ingest、dashboard，以及 chart.profile=production
  的输入校验：pod 起来之前就要求提供数据库 URL 的 Secret。
---


```bash
helm install spendguard ./charts/spendguard \
    --set postgres.existingSecret="spendguard-postgres-urls" \
    --set secrets.tls.existingSecret="spendguard-tls" \
    --set secrets.bundles.existingSecret="spendguard-bundles"
```

前置条件见 [charts/spendguard/README.md](https://github.com/m24927605/agentic-spendguard/blob/main/charts/spendguard/README.md)
（PKI Secret 格式、bundle Secret 格式、webhook
HMAC secret，以及 Postgres URL Secret 的各个 key）。

POC 限制：
- sidecar / outbox-forwarder / ttl-sweeper 默认强制 `replicas=1`。
  见 [POC vs GA gates](../poc-vs-ga.md)。
- Migration job 只是个占位；ledger + canonical 的 SQL
  migration 请用你顺手的工具来跑（sqitch / flyway / golang-migrate）。
