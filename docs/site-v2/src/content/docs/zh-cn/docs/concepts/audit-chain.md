---
title: "审计链"
---

```
sidecar / webhook ─► ledger SP ─► audit_outbox row
                                       │
                                       └─► outbox-forwarder ─► canonical_events
```

- `audit_outbox` 与 ledger 写入处于**同一事务**——没有 pending 行落库,任何决策都不会真正发到网络上。
- `outbox-forwarder` 轮询 `pending_forward=TRUE` 的行,通过 mTLS gRPC 推送到 canonical_ingest。
- `canonical_events` 分区不可变;插入后只有 `pending_forward` / `forwarded_at` 两列可改。

恢复:SP commit 之前 sidecar 挂掉,磁盘上什么都不会留下。commit 之后,`outbox-forwarder` 会一直重试推送,直到 APPENDED 或 DEDUPED。

详见 `docs/stage2-poc-topology-spec-v1alpha1.md` §11.3。
