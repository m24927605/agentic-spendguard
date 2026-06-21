---
title: "稽核鏈"
---

```
sidecar / webhook ─► ledger SP ─► audit_outbox row
                                       │
                                       └─► outbox-forwarder ─► canonical_events
```

- `audit_outbox` 跟 ledger 寫入屬於同一筆 **transaction**——只要還沒有 row 進到 pending 狀態,任何決策都不會被送出去。
- `outbox-forwarder` 會輪詢 `pending_forward=TRUE` 的 row,透過 mTLS gRPC 推送到 canonical_ingest。
- `canonical_events` 是 partition-immutable 的;insert 之後只有 `pending_forward` / `forwarded_at` 這兩個欄位可以改。

復原:如果 sidecar 在 SP commit 之前就掛掉,disk 上不會留下任何東西。一旦 commit 完成,`outbox-forwarder` 就會無限重試推送,直到 APPENDED 或 DEDUPED 為止。

詳見 `docs/stage2-poc-topology-spec-v1alpha1.md` §11.3。
