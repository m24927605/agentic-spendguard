---
title: "Docker Compose 部署(POC)"
---

要把一套可運作的 Agentic SpendGuard 拓樸跑起來,這是最快的路徑。

```bash
cd deploy/demo
docker compose down -v --remove-orphans
docker compose up -d \
    postgres pricing-seed-init bundles-init pki-init \
    canonical-seed-init manifest-init endpoint-catalog \
    ledger canonical-ingest sidecar webhook-receiver \
    outbox-forwarder ttl-sweeper dashboard control-plane
```

各個 container:

| Service | Port | 用途 |
|---|---|---|
| postgres | 5432 (internal) | ledger + canonical 資料庫 |
| ledger | 50051 (mTLS) | atomic ledger SP |
| canonical-ingest | 50052 (mTLS) | 稽核鏈持久化 |
| sidecar | UDS | adapter gRPC |
| webhook-receiver | 8443 (mTLS) | provider webhook 入口 |
| outbox-forwarder | (none) | 輪詢 audit_outbox |
| ttl-sweeper | (none) | 輪詢過期的 reservation |
| dashboard | 8090 | 維運人員操作介面 |
| control-plane | 8091 | tenant 開通 API |

這是單機的 POC。要跑在 k8s 上請參考 [Helm](helm.md)。
