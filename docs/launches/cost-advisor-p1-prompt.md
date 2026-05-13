# Session Prompt — Cost Advisor P1 — Skinny Runtime + First Rule

> Self-contained prompt for a fresh Claude Code session.
> **Goal**: ship `services/cost_advisor/bin/` daemon + `idle_reservation_rate_v1` rule SQL + CLI `spendguard advise` + integration test against benchmark fixtures.
> **Estimated**: 4 days.
> **Tracker**: GitHub issue #50.
> **Status going in**: CA-P0 GREEN-merged (commit `42cb787`); **gated on CA-P0.5 (#48) + CA-P0.6 (#49) both shipped**. Do NOT start until both are merged.

---

## Prompt to paste into a fresh session

```
任務上下文
=========
你正在 agentic-spendguard 專案實作 Cost Advisor 的 P1 — 第一個能 fire 的
rule + runtime daemon + CLI。CA-P0 (commit 42cb787) 已完成 proto + crate
skeleton + 4 migrations + integration design doc + cost_findings_upsert SP。
CA-P0.5 (#48) 已給齊 sidecar enrichment payload 欄位。
CA-P0.6 (#49) 已給齊 reservations_with_ttl_status_v1 view。

P1 把 runtime + 第一條 rule 接上來，整個 closed loop 第一次能跑：
sidecar audit → cost_advisor 規則 detect → 提 proposal → operator 透過
control_plane REST API approve → bundle_registry 收 proposal → 下次 sidecar
reload 生效。

工作目錄：/Users/michael.chen/products/agentic-spendguard
GitHub：https://github.com/m24927605/agentic-spendguard
Issue：#50

關鍵戰略決定（不要再質疑）
=========================
1. v0.1 只 ship ONE 規則 idle_reservation_rate_v1（per audit report §8）
2. 其他 3 條（failed_retry_burn / runaway_loop / tool_call_repeated）走
   issue #51 CA-P1.5 — 不要在這個 issue 內做
3. CLI `spendguard advise` 寫 JSON 輸出（CLI subcommand）不寫 TUI
4. 規則 SQL 走 SqlCostRule adapter（CA-P0 已寫 trait + adapter），不要
   寫 native Rust rule
5. 接 control_plane REST API（POST /v1/approvals）走 mTLS service identity
   `cost-advisor:<workload_instance_id>`
6. 不要建新 dashboard tab（per spec §1.1 rescope）— operator 看既有
   /v1/approvals 列表加 ?proposal_source=cost_advisor filter

關鍵檔案
========
- services/cost_advisor/（CA-P0 已建 skeleton）
- proto/spendguard/cost_advisor/v1/cost_advisor.proto（FindingEvidence schema）
- docs/specs/cost-advisor-spec.md §5.1 + §5.4 + §6 + §9（lock 過）
- docs/specs/cost-advisor-p0-audit-report.md §8（current scope authority）
- services/cost_advisor/docs/control-plane-integration.md（proposal lifecycle）
- services/cost_advisor/migrations/01_cost_findings.sql（cost_findings_upsert SP signature）
- services/ledger/migrations/0039_reservations_with_ttl_status_view.sql（P0.6 view）

啟動程序
========
1. cd /Users/michael.chen/products/agentic-spendguard
2. git pull origin main
3. 驗證前置：git log --grep="CA-P0.5" --grep="CA-P0.6" -n 必須各 ≥ 1
4. git switch -c feat/cost-advisor-p1

P1 實作流程
===========

### Step 1：Rule SQL（半天）

`services/cost_advisor/rules/detected_waste/idle_reservation_rate_v1.sql`

```sql
-- Detects: tenants where TTL'd reservations / total reservations > X%
-- AND median ttl_seconds <= configured min_ttl_for_finding
-- per spec §5.1.
--
-- Reads: reservations_with_ttl_status_v1 view (CA-P0.6).
-- Time bucket: 1 day per spec §11.5 A1.

WITH bucket AS (
    SELECT
        tenant_id,
        $1::date AS bucket_day,  -- caller passes target day
        COUNT(*) AS total_reservations,
        COUNT(*) FILTER (WHERE derived_state = 'ttl_expired') AS ttl_expired,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY ttl_seconds) AS median_ttl
      FROM reservations_with_ttl_status_v1
     WHERE created_at >= $1::date
       AND created_at < ($1::date + INTERVAL '1 day')
       AND tenant_id = $2  -- caller passes target tenant
     GROUP BY tenant_id
)
SELECT
    encode(digest('idle_reservation_rate_v1|' || tenant_id::text ||
                  '|tenant_global|' || bucket_day::text, 'sha256'), 'hex')
        AS fingerprint,
    -- ... build FindingEvidence JSONB inline,
    -- ... return rows for the runtime to decode
  FROM bucket
 WHERE total_reservations > 0
   AND (ttl_expired::numeric / total_reservations) > 0.20  -- spec §5.1
   AND median_ttl <= 60;  -- placeholder min_ttl_for_finding; should
                          -- come from contract DSL rule config when
                          -- that surface exists. Stub for v0.1.
```

**產出**：SQL file + idle_reservation_rate.rs 從 placeholder 改成 real
（is_ready() 從 false 改成 true 透過 `include_str!` 載入 SQL）

### Step 2：Runtime daemon（1.5 天）

`services/cost_advisor/src/bin/cost_advisor.rs`：
- 讀 config from env (DATABASE_URL_LEDGER + DATABASE_URL_CANONICAL + tenant scope)
- 註冊 rules: 從 rules/ 目錄掃 .sql 檔，每個包成 SqlCostRule
- 每個 rule gate on is_ready()
- 主迴圈：for tenant in tenants: for rule in registered: rule.evaluate(...)
- 把 FindingEvidence proto 轉 JSONB, 算 severity, 算 waste_estimate, 算
  fingerprint，呼叫 cost_findings_upsert SP
- 對於 cost_findings.status='open' AND scope=tenant_global，建一個
  approval_requests row with proposal_source='cost_advisor':
  - proposed_dsl_patch: RFC-6902 patch tightening idle_reservation_ratio
  - proposing_finding_id: 剛 upsert 的 finding_id
  - requested_effect + decision_context: 按 integration doc §2.2 範例填
- mTLS connection 到 ledger DB（spendguard_ledger）
  cost_advisor_application_role 已在 P0 migration 預備

`services/cost_advisor/src/runtime.rs`：
- 主 evaluation loop（tokio）
- rule registration with is_ready() gate
- prometheus metrics endpoint per Phase 5 S23 pattern

`services/cost_advisor/src/orchestration.rs`：
- 跨 DB 寫入：先 canonical UPDATE referenced_by_pending_proposal=TRUE，
  再 ledger INSERT approval_requests
- compensating rollback if ledger INSERT fails
- per integration doc §9 reconciler design

**產出**：daemon 在 docker-compose 起得來，daemon health endpoint /readyz pass

### Step 3：CLI（半天）

`services/cost_advisor/src/bin/spendguard_advise.rs`（subcommand-style）：

```
spendguard advise --tenant <UUID> [--severity critical|warn|info] [--rule <id>] \
    [--since <ISO8601>] [--format json|table] [--propose-patches]
```

預設輸出 cost_findings.status='open' 的 finding list as JSON。
--propose-patches 額外輸出對應 approval_requests row 內的 proposed_dsl_patch。

**產出**：CLI binary + 3 個 integration test exercises 不同 flag

### Step 4：Integration test（1 天）

`benchmarks/cost_advisor/idle_reservation_e2e/`：
- LangChain agent runs against MockLLM
- contract bundle has TTL=5s reservation
- 跑 20 calls, 多數 TTL 過期（因為 mock 慢 reply）
- ttl_sweeper 釋放 reservations
- cost_advisor daemon poll 看到 ttl_expired_rate > 20%
- upsert finding to cost_findings
- 提 approval_requests row
- assert: SELECT FROM cost_findings WHERE rule_id='idle_reservation_rate_v1' has rows
- assert: SELECT FROM approval_requests WHERE proposal_source='cost_advisor' has matching row
- assert: proposed_dsl_patch is valid RFC-6902

**產出**：DEMO_MODE=cost_advisor make demo-up 通過

### Step 5：control_plane wiring verification（半天）

operator 走 existing REST API 把上面的 approval_requests row approve：
```bash
curl -X POST https://control-plane:8091/v1/approvals/$ID/resolve \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"target_state": "approved", "reason": "verified via demo"}'
```

assert：approval_requests.state='approved'
assert：approval_events 有 from_state='pending' to_state='approved' row
(bundle_registry CD pipeline 真接 NOTIFY/poll 是 issue #54 的事；P1 只要
驗證 approve flow 從 cost_advisor 進去 control_plane 出來 work)

**產出**：full closed loop demo 真跑

執行慣例
========
- **Branch**: `feat/cost-advisor-p1`
- **Commit prefix**: `[CA-P1]`
- **Codex challenge**: 完成後跑 2 輪
- **更新 issue**: #50 標進度

完成標準
========
- [ ] 1 條 rule SQL 寫好 + idle_reservation_rate descriptor is_ready()=true
- [ ] daemon binary docker compose 起來 + /readyz pass
- [ ] CLI binary 3 test exercises pass
- [ ] e2e benchmark cost_advisor 模式真跑 + 1 個 finding 真出來
- [ ] control_plane approve flow PASS
- [ ] 2 輪 codex GREEN
- [ ] issue #50 closed

不要做
======
- 不要 ship 第二條規則（CA-P1.5 / issue #51）
- 不要寫 LLM narrative wrapper（spec §9 P3 / 之後 issue）
- 不要新增 dashboard tab（per rescope）
- 不要跟 bundle_registry CD 整合（issue #54 owner-ack 還沒回；P1 只到
  approve flow，不到 contract bundle ship）
- 不要動 spec — audit report §8 是 current scope authority。如果要動就
  先在 issue #57 CA-spec-v4 處理

開始
====
請執行 Step 1 rule SQL 並回報，再進 Step 2 runtime。
```

---

## Notes for the human running this prompt

- **Pre-req hard gate**: CA-P0.5 (#48) + CA-P0.6 (#49) 都 merged
- **Pre-req**: docker-compose 跑 + 既有 DEMO_MODE=agent + DEMO_MODE=ttl_sweep 都 pass
- **可能 surprise**: control_plane REST API approve flow 對 proposal_source='cost_advisor' 行為若有特殊（owner-ack Q2 未回）— 先 check issue #53 狀態

## Related artifacts

- Spec: `../specs/cost-advisor-spec.md` §5.1 §5.4 §6 §9
- Audit: `../specs/cost-advisor-p0-audit-report.md` §8 (scope authority)
- Integration: `../../services/cost_advisor/docs/control-plane-integration.md`
- Prerequisites: `cost-advisor-p0.5-prompt.md` + `cost-advisor-p0.6-prompt.md`
- Follow-on: CA-P1.5 (issue #51) for the other 3 rules

## After P1 completes

The natural next session is CA-P3 (Tier-3 LLM narrative wrapper, spec §9
P3, 4 days) OR CA-P1.5 (the other 3 rules, 5 days). Pick based on whether
first design-partner needs LLM-rendered explanations OR more rule coverage
first.
