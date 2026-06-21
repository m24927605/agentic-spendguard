---
title: "決策生命週期"
---

每一個 LLM / tool 呼叫的邊界,都會觸發一筆 8 個階段的決策交易:

| # | 階段 | 在哪裡執行 | 產出 |
|---|---|---|---|
| 1 | snapshot | sidecar(in-process) | snapshot_hash |
| 2 | evaluate | sidecar Contract DSL | matched_rules_hash |
| 3 | prepare_effect | sidecar(純運算) | effect_hash |
| 4 | reserve | ledger 原子操作 | reservation_id |
| 5 | audit_decision | 併入 reserve | audit_outbox row |
| 6 | publish_effect | adapter(in-process) | 套用變更 |
| 7 | commit_or_release | ledger | commit_estimated / release |
| 8 | audit_outcome | 併入 commit/release | audit_outbox row |

最關鍵的硬不變式:階段 1–5 是原子地一起發生(同一筆 Postgres
transaction);萬一 sidecar 在 publish 中途掛掉,會靠 `effect_hash`
的 idempotency 把階段 6 重放一次。

正式規格請看 `docs/contract-dsl-spec-v1alpha1.md` §6。
