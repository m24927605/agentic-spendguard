# Stage 1 Cross-Spec Integration Audit

> **目的**：4 個 LOCKED specs 的跨 spec 整合性 audit（single-pass，不 iterate）。  
> **日期**：2026-05-07  
> **方法**：Explore agent 全新讀過 4 specs；對照 12 個 checklist 領域；不重新設計，只找 drift。  
> **範圍**：contract-dsl / trace-schema / sidecar-architecture / ledger-storage v1alpha1 specs

---

## 0. POC Go / No-Go 判定

| 範疇 | 判定 |
|---|---|
| **Critical findings** | 3 個（必補後才可 POC） |
| **Minor findings** | 3 個（建議補但非 blocker） |
| **Nice-to-have** | 2 個（cosmetic） |
| **POC readiness** | ⚠️ **CONDITIONAL GO** — 應用 3 個 critical patches 後即可進 POC |
| **Spec 整體健康度** | 12 個 cross-reference 中 10 個 valid、1 個 broken、1 個 drift |

---

## 1. Critical Findings（必補）

### 🔴 Finding 1: `audit_decision_id` Terminology Drift

**What**：同一概念在 3 個 spec 用不同名稱：
- Contract §6 line 249 + §13 line 670: `audit_decision_id`
- Trace §11.1 line 776: `spendguard.canonical.audit.decision_event_id`
- Ledger §5.2 line 314: `audit_decision_id` (column name)

**Impact**：實作層 query join 與 audit replay 將需要 aliasing/transformation；audit trail 跨 spec 不統一。

**Patch**：統一為 `audit_decision_event_id`（採 Trace 命名）：
- Contract §6, §13：`audit_decision_id` → `audit_decision_event_id`
- Ledger §5.2 DDL：column rename
- Trace §11.1：保持原命名（已正確）

---

### 🔴 Finding 2: Refund/Dispute 在 Contract Spec 缺失

**What**：Ledger §10 (lines 887-905) 在 v2 新增 `refund_credit` / `dispute_adjustment` operation_kinds，含完整 state machine。但：
- Contract §5 (lines 175-211) 只列 4 commit states，無 refund/dispute 路徑
- Contract DSL 無法表達「為 dispute 寫 policy」「refund 觸發退款」

**Impact**：POC 無法處理 provider 退款或 dispute；第一個遇到 Stripe disputes 的客戶會 break。Audit trail 將有 Contract DSL 預期外的 ledger entries。

**Patch**：Contract §5 新增 5.1a「Refund and Dispute Handling」：
```yaml
refund_policy:
  trigger: provider_issues_credit
  effect: emit refund_credit operation → increases available_budget
  audit: emit refund_event with provider_credit_id

dispute_policy:
  trigger: customer_disputes_charge
  flow:
    case_open: emit dispute_adjustment → pending resolution
    case_resolved_in_favor: emit refund_credit
    case_resolved_against: emit compensating_entry
```

Contract §13 audit schema 加 `refund_event` + `dispute_event` 欄位。

---

### 🔴 Finding 3: Decision Transaction Stage 持久化目標不明確

**What**：
- Contract §6: 8 stages 抽象命名 + persisted_in 描述
- Sidecar §12.1: ownership matrix 將 stage 對應至 component（local_sidecar / remote_ledger / etc.）
- 但 `audit_decision` stage 「persists to remote_durable_store」— 哪個 remote store？
- Sidecar §6.2 durability matrix 說「依 deployment mode」（k8s_saas → journal；lambda → ingest），但 Contract §6 沒交叉引用此依賴

**Impact**：實作冷啟恢復時不知去哪裡 query missing stage outputs；audit replay 不可靠。

**Patch**：
- Contract §6 加 §6.3「Stage Persistence Deployment Matrix」明示 `audit_decision` 的 remote store 由 Sidecar §6.2 決定
- Sidecar §12.1 明確標示：「audit_decision 持久化目標 = §6.2 durability_mode_selection 對應之 store」
- Ledger §5.2 ledger_transactions 可選加 `stage_output_kind` enum 啟用 stage artifact 儲存

---

## 2. Minor Findings（建議補）

### 🟡 Finding 4: pricing_version 三層 freeze 未在 Trace 完整體現

**What**：
- Ledger §13 承諾「三層 freeze」：pricing_version + price_snapshot_hash + fx_rate_version + unit_conversion_version
- Trace §10.4 `ingest_time_pinned` 列表只含 `pricing_version` 與 `commit_state`
- Ledger §5.3 ledger_entries 表存全 4 layers，但 Trace 不一定 emit 全部

**Impact**：Trace event 可能不含 fx_rate_version / unit_conversion_version → Ledger 須推算或重算 → replay 不確定性。

**Patch**：Trace §10.4 `ingest_time_pinned` 列表加：
- `spendguard.canonical.llm_call.pricing.price_snapshot_hash`
- `spendguard.canonical.llm_call.pricing.fx_rate_version`
- `spendguard.canonical.llm_call.pricing.unit_conversion_version`

---

### 🟡 Finding 5: Capability Flags Phase 1 約束未在 Contract 引用

**What**：
- Ledger §18 + Sidecar §12.5：Phase 1 ledger 只 advertise `single_writer_per_budget`，**不能** advertise `strong_global`
- Contract §14 latency budget（50ms p99）的可達性**依賴此約束**
- 但 Contract 無提及 capability flag 限制

**Impact**：Contract 可能被部署在 eventual consistency ledger（Phase 3+），違反 50ms p99 假設；無 audit 阻止錯誤配置。

**Patch**：
- Contract §3.2 modeSemantics 加 footnote：「Hard enforcement (enforce mode) 須 ledger advertise strong_global 或 single_writer_per_budget（Ledger §18）。Shadow mode 容忍 eventual consistency。」
- Contract §14 加：「Warm p99 達成依賴 single_writer_per_budget 或 strong_global ledger capability。」

---

### 🟡 Finding 6: HMAC Salt Rotation Window 跨 Spec 未對齊

**What**：
- Trace §6: yearly + 12-month dual_period
- Sidecar §15: yearly + 12-month dual key period（producer signing keys）
- Contract §13 mentions `hmac_sha256_with_tenant_salt` 但無 cross-reference rotation policy
- Sidecar §5 IPC handshake 未含 hash key epoch negotiation

**Impact**：Rotation window 內的請求若 sidecar 與 ledger 對 acceptance keys 不一致 → validation 失敗。

**Patch**：
- Sidecar §5 protocol_handshake 加：「sidecar_announces: active_hash_key_epochs」
- Trace §6 dual_period 補充：「12 個月 dual_read 期間 old + new HMAC keys 都 valid」

---

## 3. Nice-to-Have（cosmetic）

### ⚪ Finding 7: 「Phase 1 first customer」術語不統一

- Sidecar §13: `phase_1_first_customer`
- Ledger §19: `Phase 1`
- Contract §0.2: `first customer design partner onboarding`

**Patch**：統一為「Phase 1 (first customer design partner)」。

---

### ⚪ Finding 8: 跨 region failover audit event 在 Contract 缺少

**What**：
- Sidecar §10 + Ledger §12 都定義 `region_failover_invoked` / `region_failover_promoted` audit event
- Contract §13 audit schema 未列 region_failover event type

**Patch**：Contract §13 加 optional event type：「region_failover_promoted (Phase 2+)」。

---

## 4. Cross-Reference Validity Matrix

| Reference | From → Target | Section | Status |
|---|---|---|---|
| Contract §6 stages | Sidecar §12.1 | Contract §6 | ✅ VALID |
| Contract §5 commit states | Trace §10.4 | Contract §5 | ✅ VALID |
| Trace §10.4 三 amounts | Ledger §20.3 | Trace §10.4 | ✅ VALID |
| Sidecar §6.2 durability matrix | Ledger §20.4 | Sidecar §6.2 | ✅ VALID |
| Ledger §18 capability flags | Sidecar §12.5 | Ledger §18 | ✅ VALID |
| Contract §8 sub-agent trust | Sidecar §3.1 + §5 | Contract §8 | ✅ VALID |
| Trace §13 producer signing | Sidecar §12.4 | Trace §13 | ✅ VALID |
| Ledger §5 budget_window_instances | Trace §8 namespace | Ledger §5 | ✅ VALID |
| **Contract §13 refund/dispute events** | **Ledger §10** | **Contract §13** | 🔴 **BROKEN**（Finding 2） |
| **audit_decision_id term** | **All specs** | **Multiple** | 🟡 **DRIFT**（Finding 1） |

10 / 12 references valid；1 broken；1 drift。

---

## 5. 術語統一建議

| 概念 | 目前用詞 | 建議 canonical | 涉及 spec |
|---|---|---|---|
| 預訂中容量 | reserved_hold / reserved | **reserved_hold**（Ledger 命名） | Contract / Ledger §2.2 / Trace |
| 已扣款 | committed_spend / committed | **committed_spend** | Ledger §2.2 / Trace |
| 估算金額 | estimated_amount / estimated | **estimated_amount**（Trace §10.4 命名） | Trace §10.4 / Ledger §5.3 |
| 子 agent 授權 token | budget_grant / grant / jwt_access_token | **budget_grant**（Contract §8 命名） | Contract §8 / Trace §7.2 / Sidecar §3.1 |
| 決策 audit 標記 | audit_decision_id / decision_event_id / audit.decision_event_id | **audit_decision_event_id** | Finding 1 |
| 狀態追蹤 | posting_state / current_state | 釐清 scope：posting_state for ledger_transactions；current_state for reservations | Ledger §5.2 |

---

## 6. Companion Integration Completeness Matrix

| 配對 | Explicit 整合 | Gap / 單向引用 |
|---|---|---|
| Contract ↔ Trace | Contract §13 audit schema vs Trace §11.1 anchors | Contract 未引用 Trace §10.6 golden corpus；Trace 未引用 Contract §13 必填欄位 |
| Contract ↔ Sidecar | Contract §6 stages vs Sidecar §12.1；Contract §14 latency vs Sidecar §12.2 | Contract 未引用 Sidecar §12.5 capability flags 約束 |
| Contract ↔ Ledger | Contract §5 commit states vs Ledger §20.3；Contract §6 audit vs Ledger §20.2 | **Contract 未引用 Ledger §10 refund/dispute operation kinds**（Finding 2） |
| Trace ↔ Sidecar | Trace §10.1 ingest vs Sidecar §12.3；Trace §13 producer vs Sidecar §12.4 | 大致對稱 |
| Trace ↔ Ledger | Trace §10.4 amounts vs Ledger §20.3；Trace §12 schema_bundle vs Ledger §20.5 | Ledger 未驗證 schema_bundle 完整性 |
| Sidecar ↔ Ledger | Sidecar §6.2 durability vs Ledger §20.4；Sidecar §9 fencing vs Ledger §20.5 | Ledger 未 acknowledge Sidecar §8 endpoint discovery protocol |

**Gap 結論**：Contract 是最 under-integrated 的 spec（不引用 capability flags、不引用 refund/dispute、不引用 durability selection）。建議增加「Requester Dependencies」subsection 至 Contract Companion Integration。

---

## 7. 建議 v1.1 Patch Scope

### Critical（POC blocker）
1. **Finding 1**：Rename `audit_decision_id` → `audit_decision_event_id` 跨 Contract + Ledger
2. **Finding 2**：Contract §5 加 refund/dispute；§13 audit schema 補 refund_event + dispute_event
3. **Finding 3**：Contract §6 加 §6.3 Stage Persistence Deployment Matrix；Sidecar §12.1 明確標示

### Recommended（POC 前完成更好）
4. **Finding 4**：Trace §10.4 加 3 個 pricing freeze fields
5. **Finding 5**：Contract §3.2 + §14 加 capability flags 約束 footnote
6. **Finding 6**：Sidecar §5 加 hash key epoch negotiation；Trace §6 補充 dual_read 期間 keys 都 valid

### Nice-to-have（POC 後）
7. **Finding 7**：統一 Phase 1 命名
8. **Finding 8**：Contract §13 加 region_failover event type

---

## 8. Multi-Spec Governance 建議

POC 後實施「Companion Spec Integration Checklist」：每個 spec 的 §X.2 Companion Integration 必須：
1. 列出每個 upstream/downstream spec 名稱
2. 引用 sections（含 file + line number）
3. 標註 bidirectional（A↔B）vs unidirectional（A→B only）
4. 標記 broken reference 或 drift

避免未來 spec release 時 drift 累積。

---

## 9. 結論

**POC Go/No-Go 決定**：⚠️ **CONDITIONAL GO**

- 應用 Findings 1-3 的 critical patches 後即可開始 POC（estimated 2-4 hour 編輯）
- Findings 4-6 minor patches 建議 POC 前一併完成（額外 1 hour）
- Findings 7-8 cosmetic 可延後

**整體評估**：4 個 spec 在 13 輪 Codex 反饋下達成驚人一致性。Drift 範圍小、修補 cost 低。Contract spec 是最 under-integrated 的（不直接引用其他 specs 的關鍵約束），建議補三處 cross-reference 後 POC ready。

---

*Audit report version: stage1-integration-audit-v1 | Generated: 2026-05-07 | Scope: 4 LOCKED specs cross-spec integration | Method: single-pass Explore agent + checklist validation*
