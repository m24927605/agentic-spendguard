---
title: "Docker Compose 部署(POC)"
---

跑通 Agentic SpendGuard 拓扑最快的一条路。

```bash
cd deploy/demo
docker compose down -v --remove-orphans
docker compose up -d \
    postgres pricing-seed-init bundles-init pki-init \
    canonical-seed-init manifest-init endpoint-catalog \
    ledger canonical-ingest sidecar webhook-receiver \
    outbox-forwarder ttl-sweeper dashboard control-plane
```

容器清单:

| 服务 | 端口 | 用途 |
|---|---|---|
| postgres | 5432 (internal) | ledger + canonical 数据库 |
| ledger | 50051 (mTLS) | 原子 ledger SP |
| canonical-ingest | 50052 (mTLS) | 审计链持久化 |
| sidecar | UDS | adapter gRPC |
| webhook-receiver | 8443 (mTLS) | provider webhook 入口 |
| outbox-forwarder | (none) | 轮询 audit_outbox |
| ttl-sweeper | (none) | 轮询过期的 reservation |
| dashboard | 8090 | 运维 UI |
| control-plane | 8091 | 租户开通 API |

这是单机 POC。上 k8s 看 [Helm](helm.md)。
