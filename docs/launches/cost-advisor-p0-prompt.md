# Session Prompt — Cost Advisor P0 Implementation

> Self-contained prompt for a fresh Claude Code session.
> **Goal**: execute Phase 0 (prep) of the Cost Advisor implementation per
> `docs/specs/cost-advisor-spec.md` §9 — schema audit + FindingEvidence
> proto + migrations + control_plane integration design.
> **Estimated**: 4 days (per spec §9; +3-10 days contingency if schema audit reveals missing fields).
> **Status going in**: spec passed codex challenge round 4 with GREEN_LIGHT_FOR_P0
> after 4 rounds of adversarial review.

---

## Prompt to paste into a fresh session

```
任務上下文
=========
你正在 agentic-spendguard 專案實作 Cost Advisor 的 P0（prep phase）。
Cost Advisor 是 SpendGuard closed-loop control 的新 feature：
讀取 audit chain → 偵測 waste pattern → 產生 proposed contract DSL patches
→ 走既有 control_plane approval queue → operator approve → 下次
sidecar reload 生效。

不是 standalone product。沒有新 dashboard / 新 gRPC / 新 SDK / 新 digest。
這個關鍵戰略決定是 codex round 3 的 fundamental rescope，已 round 4 GREEN。

工作目錄：/Users/michael.chen/products/agentic-spendguard
GitHub：https://github.com/m24927605/agentic-spendguard

關鍵戰略決定（不要再質疑，已 codex 4 輪 GREEN）
==============================================
1. **不是 standalone product**：feature of existing closed loop（codex r3 rescope）
2. **Detection 用 SQL，narrative 才用 LLM**：成本可控，質量靠規則庫
3. **真競品**：AgentGuard / AgentBudget（不是 LangSmith / Helicone — 那些是 observability）
4. **真受眾**：platform engineering / AI infra leads / compliance teams
5. **誠實 cost claim**：$0.005-0.13/tenant/day Tier-3 LLM；不假裝 $0.01
6. **失敗分類由 canonical_ingest 擁有**（spec §5.1.2），不在 rule SQL 內 ad-hoc
7. **Findings → proposed contract patches → 既有 approval queue**（不要新 UI）

關鍵檔案（讀這些建立 context）
=============================
- `docs/specs/cost-advisor-spec.md` — **主要參考**，822 行 v3，已 GREEN
  特別讀：§1.1 closed-loop diagram, §4.0 FindingEvidence schema, §5.1.2
  failure classifier ownership, §9 phasing, §11.5 A2 contingency, §11.5 A7 storage
- `services/canonical_ingest/` — 將擴充以 own failure classification
- `services/control_plane/` — 將擴充 approval queue schema
- `services/retention_sweeper/` — 將整合自動 purge cost_findings
- `proto/spendguard/` — 新增 cost_advisor/v1/cost_advisor.proto
- `benchmarks/real-stack-e2e/REAL_LANGCHAIN_E2E.md` — V1 已驗 stack 真能跑

啟動程序
========
1. cd /Users/michael.chen/products/agentic-spendguard
2. git pull origin main 確保最新（spec at commit 8c0fda4 或之後）
3. cat docs/specs/cost-advisor-spec.md | head -100 重新建立 context
4. 開新 branch：git switch -c feat/cost-advisor-p0

P0 實作流程
===========

### Step 1：Schema reality check audit（spec §11.5 A2 — 1 day）

跑 SQL 查 `canonical_events.payload` 實際 shape，確認規則需要的欄位是否
都在且 populated > 80%。

```sql
-- 範例：檢查 prompt_hash 是否存在 + populate 率
SELECT
    COUNT(*) AS total,
    COUNT(payload->>'prompt_hash') AS has_prompt_hash,
    COUNT(payload->>'agent_id') AS has_agent_id,
    COUNT(payload->>'run_id') AS has_run_id,
    COUNT(payload->>'tool_name') AS has_tool_name,
    COUNT(payload->>'model_family') AS has_model_family,
    COUNT(payload->>'committed_micros_usd') AS has_committed
FROM canonical_events
WHERE detected_at > NOW() - INTERVAL '7 days';
```

**Branch decision**（per §11.5 A2）：
- All fields > 80% populated → 進 Step 2 as planned
- 1-2 fields missing → backfill enrichment job (+3 days)
- 3+ fields missing → restrict v0 rules + revise §5.1 (+5 days + scope cut)
- PII in unexpected fields → integrate retention_sweeper redaction (+5-10 days)

**產出**：`docs/specs/cost-advisor-p0-audit-report.md`，記錄結果 + branch 決定 + 修正過的 §5.1 rule 列表（如需要）

### Step 2：FindingEvidence proto + 規則 trait（1 day）

- 新增 `proto/spendguard/cost_advisor/v1/cost_advisor.proto`
  - `FindingEvidence` message 對應 spec §4.0 JSONSchema
  - `Metric`（typed: name/value/unit/source_field/pii_classification/derivation/ci95）
  - `WasteEstimate`（micros_usd/method/confidence/explanation）
  - `Scope`（agent/run/tool/tenant_global）
  - `FailureClass` enum（8 classes per spec §5.1）
- 新增 `services/cost_advisor/` Rust crate skeleton
  - `Cargo.toml` + `src/lib.rs`
  - `CostRule` trait 對應 spec §5.4（8 methods）
  - `CostRule` 的 `SqlCostRule` adapter（loads `.sql` files）

**產出**：proto + crate skeleton + 1 個 dummy `CostRule` impl 通過 cargo check

### Step 3：Migrations（1 day）

- `services/canonical_ingest/migrations/<NN>_add_failure_class.sql`
  - 新增 `canonical_events.failure_class TEXT` column，default NULL
  - Backfill 計畫（一次性 batch job，per spec §5.1.2）
- `services/cost_advisor/migrations/01_cost_findings.sql`
  - 對應 spec §4.1 schema（含 fingerprint UNIQUE index）
- `services/cost_advisor/migrations/02_cost_baselines.sql`
  - 對應 spec §6 schema
- `services/control_plane/migrations/<NN>_add_proposal_source.sql`
  - 既有 approval queue table 加 `proposal_source` enum + `proposed_dsl_patch` field
- 整合 `services/retention_sweeper/` — 加 `cost_findings` 到 retention rules

**產出**：4 個 migration 檔，跑過本機 docker-compose 確認 apply 成功

### Step 4：control_plane integration design doc（1 day）

寫 `services/cost_advisor/docs/control-plane-integration.md`，內容：
- proposed_contract_patches table schema 變更（與 control_plane owner 確認）
- proposal lifecycle：cost_advisor 寫入 → operator review → approve/deny → 觸發 contract_bundle CD pipeline
- 既有 dashboard 如何 filter `proposal_source = 'cost_advisor'`
- 鑑權：哪個 service identity 可寫 proposal？（mTLS）

不是寫 code，是落筆設計 + 對齊 owner。

執行慣例
========
- **Branch**: `feat/cost-advisor-p0`
- **Commit message**: 開頭加 `[CA-P0]` 對應 todo 追蹤，例如：
  `feat(cost-advisor): FindingEvidence proto + CostRule trait skeleton [CA-P0]`
- **Codex challenge**: P0 完成後不需跑（spec 已 GREEN；P1 實作 rule 才需要）
- **更新 TODO**: `docs/SPENDGUARD_VIRAL_PLAYBOOK.todo.md` 加新條目 CA-P0 並標進度
- **不要動的**：既有 control_plane / canonical_ingest / retention_sweeper code 邏輯（只加 schema/欄位 + integration 點）

完成標準（逐項回報）
====================
- [ ] Schema audit report 產出，branch 決定明確
- [ ] FindingEvidence proto + CostRule trait skeleton 編譯過
- [ ] 4 個 migration apply 成功
- [ ] control-plane integration design doc 對齊
- [ ] todo 文件更新 CA-P0 ✅
- [ ] 一句話總結：P0 結束後 v0.1 (P1) 實作可不可以直接開始

不要做
======
- 不要實作 rule SQL（那是 P1）
- 不要寫 LLM narrative wrapper（那是 P3）
- 不要建新 dashboard tab（per rescope，禁止）
- 不要建新 gRPC service（per rescope）
- 不要在 P0 內試圖跑 e2e demo（沒有實作 rule 還沒東西可 demo）
- 不要 over-promise：P0 是 prep，產出是 schema + skeleton + design doc

開始
====
請執行 Step 1 schema audit 並回報結果，等我說 "go" 再進 Step 2。
```

---

## Notes for the human running this prompt

- **Pre-req**: docker compose 可以跑（用過去 V1 確認過的 setup）
- **Pre-req**: postgres 必須有真資料 — 如果是空 stack，schema audit 會回 "0 events"，需要先跑幾個 demo 模式產生 canonical_events 才能 audit
- **Schedule risk**: spec §11.5 A2 已內建 contingency；但若 audit 揭露 critical PII 問題，P0 可能 +5-10 天
- **Pre-req**: 確認 `~/.env` 有 `OPENAI_API_KEY`（demo 跑 agent_real_* 模式需要產真 audit data）

## Related artifacts

- Strategic plan: `../SPENDGUARD_VIRAL_PLAYBOOK.md`
- Open work tracker: `../SPENDGUARD_VIRAL_PLAYBOOK.todo.md`
- Spec (the bible): `../specs/cost-advisor-spec.md`
- V1 e2e proof (real stack works): `../../benchmarks/real-stack-e2e/REAL_LANGCHAIN_E2E.md`
- Codex r1-r4 review history: §14 of the spec

## After P0 completes

The natural next session is **P1 implementation** (5-6 days per §9). P1 prompt
should be written when P0 completes (so it can reflect actual Step 1 audit outcome
and any §5.1 rule list revisions).
