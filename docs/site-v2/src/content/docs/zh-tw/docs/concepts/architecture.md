---
title: "六層架構"
---

Agentic SpendGuard 把整套職責拆成 6 個基礎層，每次做決策都嚴格照這個順序跑一遍：

```
T (Trace) → L (Ledger) → C (Contract) → D (Decision) → E (Evidence) → P (Proof)
```

| Layer | 職責 | 關鍵不變式 |
|---|---|---|
| **T** Trace | 記錄事件身分（run_id、step_id、llm_call_id） | 每個事件都有一個全域唯一的 id |
| **L** Ledger | 原子化的預算保留（reservation）與 commit | 每筆 tx 都維持 per-unit 餘額正確 |
| **C** Contract | 熱路徑（hot-path）上的政策評估 | 決策耗時 <5ms |
| **D** Decision | 8 階段的交易狀態機 | 第 1～4 階段一律原子化 |
| **E** Evidence | 稽核鏈（audit chain）的持久性 | 沒有 audit row 就不會有任何 effect（§6.1） |
| **P** Proof | 逐事件簽章與驗證 | Cosign 簽過的 bundle 搭配 Ed25519 事件 |

完整規格請參考 source repo 裡的 `docs/contract-dsl-spec-v1alpha1.md` 與
`docs/stage2-poc-topology-spec-v1alpha1.md`。
