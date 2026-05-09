# Audit chain

```
sidecar / webhook ─► ledger SP ─► audit_outbox row
                                       │
                                       └─► outbox-forwarder ─► canonical_events
```

- `audit_outbox` is **transactional** with the ledger write — no decision
  reaches the wire without a row pending.
- `outbox-forwarder` polls `pending_forward=TRUE` and pushes to
  canonical_ingest via mTLS gRPC.
- `canonical_events` is partition-immutable; only the
  `pending_forward` / `forwarded_at` columns are mutable post-insert.

Recovery: a sidecar crash before the SP commits leaves nothing on
disk. After commit, `outbox-forwarder` retries push indefinitely until
APPENDED or DEDUPED.

See `docs/stage2-poc-topology-spec-v1alpha1.md` §11.3.
