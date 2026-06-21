---
title: "决策生命周期"
---

每一次 LLM / 工具调用边界都会触发一笔 8 阶段的决策事务：

| # | 阶段 | 位置 | 输出 |
|---|---|---|---|
| 1 | snapshot | sidecar（in-process） | snapshot_hash |
| 2 | evaluate | sidecar Contract DSL | matched_rules_hash |
| 3 | prepare_effect | sidecar（纯计算） | effect_hash |
| 4 | reserve | ledger 原子操作 | reservation_id |
| 5 | audit_decision | 并入 reserve | audit_outbox 行 |
| 6 | publish_effect | adapter（in-process） | 应用变更 |
| 7 | commit_or_release | ledger | commit_estimated / release |
| 8 | audit_outcome | 并入 commit/release | audit_outbox 行 |

硬性不变式：阶段 1–5 原子完成（单个 Postgres 事务）；sidecar 在 publish 中途崩溃时，靠 `effect_hash` 幂等性重放阶段 6。

正式规范见 `docs/contract-dsl-spec-v1alpha1.md` §6。
