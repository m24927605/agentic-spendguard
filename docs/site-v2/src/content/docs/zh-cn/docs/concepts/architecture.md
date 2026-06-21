---
title: "六层架构"
---

Agentic SpendGuard 把职责拆成 6 个原语层,每次决策都严格按这个顺序执行:

```
T (Trace) → L (Ledger) → C (Contract) → D (Decision) → E (Evidence) → P (Proof)
```

| 层 | 职责 | 关键不变式 |
|---|---|---|
| **T** Trace | 捕获事件标识(run_id、step_id、llm_call_id) | 每个事件都有全局唯一 id |
| **L** Ledger | 预算的原子预留 + 提交 | 每笔 tx 都保持 per-unit 余额守恒 |
| **C** Contract | 热路径策略求值 | 决策耗时 <5ms |
| **D** Decision | 8 阶段事务状态机 | 阶段 1-4 始终原子 |
| **E** Evidence | 审计链持久化 | 没有审计行就不产生任何 effect(§6.1) |
| **P** Proof | 逐事件签名 + 验证 | Cosign 签名的 bundle + Ed25519 事件 |

完整规格见源仓库里的 `docs/contract-dsl-spec-v1alpha1.md` 和
`docs/stage2-poc-topology-spec-v1alpha1.md`。
