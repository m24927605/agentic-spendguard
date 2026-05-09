# Docker Compose deployment (POC)

The fastest path to a working SpendGuard topology.

```bash
cd deploy/demo
docker compose down -v --remove-orphans
docker compose up -d \
    postgres pricing-seed-init bundles-init pki-init \
    canonical-seed-init manifest-init endpoint-catalog \
    ledger canonical-ingest sidecar webhook-receiver \
    outbox-forwarder ttl-sweeper dashboard control-plane
```

Containers:

| Service | Port | Purpose |
|---|---|---|
| postgres | 5432 (internal) | ledger + canonical DBs |
| ledger | 50051 (mTLS) | atomic ledger SP |
| canonical-ingest | 50052 (mTLS) | audit chain durable |
| sidecar | UDS | adapter gRPC |
| webhook-receiver | 8443 (mTLS) | provider webhook entry |
| outbox-forwarder | (none) | polls audit_outbox |
| ttl-sweeper | (none) | polls expired reservations |
| dashboard | 8090 | operator UI |
| control-plane | 8091 | tenant provisioning API |

This is a single-machine POC. For k8s see [Helm](helm.md).
