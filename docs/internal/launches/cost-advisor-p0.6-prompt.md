# Session Prompt — Cost Advisor P0.6 — Ledger ttl_status View

> Self-contained prompt for a fresh Claude Code session.
> **Goal**: ship `reservations_with_ttl_status_v1` view in `spendguard_ledger` so cost_advisor rules can detect `ttl_expired` reservations.
> **Estimated**: 2 days.
> **Tracker**: GitHub issue #49.
> **Status going in**: CA-P0 GREEN-merged on main (commit `42cb787`). Codex r5 revealed the view is required before any rule can fire.

---

## Prompt to paste into a fresh session

```
任務上下文
=========
你正在 agentic-spendguard 專案實作 Cost Advisor 的 P0.6（ledger projection
view），這是 codex r5 adversarial review 揭露的新 workstream。

CA-P0 audit (docs/specs/cost-advisor-p0-audit-report.md §8.2) 證實 spec
§5.1 第一條 rule `idle_reservation_rate_v1` 不能 fire — 因為 ledger
.reservations.current_state 沒有 'ttl_expired' state，TTL 過期是寫在
audit_outbox 的 release event reason='TTL_EXPIRED'。

P0.6 補一個 view 把這個資訊 derive 出來，給 cost_advisor 規則用。

工作目錄：/Users/michael.chen/products/agentic-spendguard
GitHub：https://github.com/m24927605/agentic-spendguard
Issue：#49

關鍵戰略決定（不要再質疑）
=========================
1. 走 VIEW 路線，不改 reservations 表（reservations 已是 Phase 2B locked
   schema 不重新設計）
2. 不需要 materialized view — 規則查詢頻率低（nightly），實時 view 夠用
3. JOIN 路徑：reservations.source_ledger_transaction_id → ledger_transactions
   (找對應的 release tx) → audit_outbox (找 release event payload)
4. 不要動 cost_advisor crate；P1 才會接這個 view 進 rule SQL

關鍵檔案
========
- docs/specs/cost-advisor-spec.md §5.1 + §11.5 A2（規則需求）
- docs/specs/cost-advisor-p0-audit-report.md §8.1 + §8.2（為什麼 P0.6 存在）
- services/ledger/migrations/0010_projections.sql（reservations + commits 表結構）
- services/ledger/migrations/0015_post_release_transaction.sql（release SP, 含 reason 寫法）
- services/ledger/migrations/0019_release_ttl_sweeper_extensions.sql
  （TTL_EXPIRED reason 怎麼寫）
- services/ttl_sweeper/src/（worker 怎麼觸發 release）
- services/ledger/migrations/0009_audit_outbox.sql（audit_outbox 表結構）

啟動程序
========
1. cd /Users/michael.chen/products/agentic-spendguard
2. git pull origin main
3. cat docs/specs/cost-advisor-p0-audit-report.md（特別看 §8）
4. git switch -c feat/cost-advisor-p0.6

P0.6 實作流程
=============

### Step 1：JOIN 路徑驗證（半天）

走 schema 一遍確認 JOIN 路徑可行：

```sql
-- 抓最近 7 天的 TTL_EXPIRED release event 看 payload shape
SELECT
    o.audit_outbox_id,
    o.decision_id,
    o.cloudevent_payload->'data' AS data_field,
    o.recorded_at
  FROM audit_outbox o
 WHERE (o.cloudevent_payload->>'type') = 'spendguard.audit.outcome'
   AND o.recorded_at > NOW() - INTERVAL '7 days'
 LIMIT 5;
```

注意：payload_json.data 是 base64 encoded（codex r5 P2-a），需要
decode 才看得到 reason 欄位。確認 release event 真的有 reason='TTL_EXPIRED'。

接下來：
```sql
-- reservations -> ledger_transactions 哪一個欄位 join?
\d reservations
\d ledger_transactions
```

reservations.source_ledger_transaction_id 是「reserve tx」。release 是
另一個 tx，但 decision_id 相同。所以 JOIN 用 decision_id。

**產出**：把 JOIN SQL 寫出來在 services/ledger/migrations/draft 內，跑
docker-compose 確認 1+ TTL_EXPIRED row 真的能被抓到。

### Step 2：寫 view migration（半天）

`services/ledger/migrations/0039_reservations_with_ttl_status_view.sql`

```sql
CREATE OR REPLACE VIEW reservations_with_ttl_status_v1 AS
SELECT
    r.reservation_id,
    r.tenant_id,
    r.budget_id,
    r.current_state,
    CASE
        WHEN r.current_state = 'released'
             AND release_evt.reason = 'TTL_EXPIRED'
            THEN 'ttl_expired'
        ELSE r.current_state
    END AS derived_state,
    EXTRACT(EPOCH FROM (r.ttl_expires_at - r.created_at))::INT AS ttl_seconds,
    r.created_at,
    r.ttl_expires_at,
    release_evt.reason AS release_reason,
    release_evt.recorded_at AS released_at
  FROM reservations r
  LEFT JOIN LATERAL (
      SELECT
          convert_from(decode(o.cloudevent_payload->>'data_b64', 'base64'),
                       'UTF8')::jsonb->>'reason' AS reason,
          o.recorded_at
        FROM audit_outbox o
        JOIN ledger_transactions lt
          ON lt.audit_decision_event_id = (o.cloudevent_payload->>'id')::uuid
       WHERE lt.decision_id = (SELECT decision_id FROM ledger_transactions
                               WHERE ledger_transaction_id = r.source_ledger_transaction_id)
         AND (o.cloudevent_payload->>'type') = 'spendguard.audit.outcome'
         AND (convert_from(decode(o.cloudevent_payload->>'data_b64', 'base64'),
                          'UTF8')::jsonb->>'kind') = 'release'
       ORDER BY o.recorded_at DESC
       LIMIT 1
  ) release_evt ON TRUE;

COMMENT ON VIEW reservations_with_ttl_status_v1 IS
  'Cost Advisor P0.6: derived view exposing ttl_expired state + ttl_seconds for cost_advisor rules. Joins reservations + audit_outbox via decision_id.';
```

注意 codex r5 P2-a：payload_json.data 是 base64 encoded，所以要 decode。
這個 JOIN 比 audit-report draft 範例多一步 ledger_transactions join 才
能從 reserve tx 走到 release tx（同 decision_id 兩個 tx）。

**產出**：migration 檔 + 跑過 docker-compose apply OK。

### Step 3：Performance + unit test（半天）

加 index 給 view 用：
```sql
-- 已存在 idx_reservations_active / idx_reservations_ttl 涵蓋 ttl_expires_at
-- audit_outbox 已有 decision_id + recorded_at 索引（檢查）
-- 若 view 慢，加 EXPLAIN ANALYZE 看 plan
```

unit test fixtures（services/ledger/tests/ttl_status_view_test.rs 或
就直接 SQL test）：
1. fixture：reserve(TTL=5s) → 等過期 → ttl_sweeper release → 查 view
   應該返回 derived_state='ttl_expired'
2. fixture：reserve → commit_estimated → 查 view 應該返回
   derived_state='committed'
3. fixture：reserve → 主動 release(reason='RUN_ABORTED') → 查 view
   應該返回 derived_state='released'（不是 ttl_expired）

**產出**：3 個 fixture 都 pass + EXPLAIN ANALYZE plan 文件化（在
README 或 migration comment 中）。

### Step 4：Demo mode（半天）

擴 deploy/demo/Makefile：DEMO_MODE=ttl_sweep 已存在，加 verify SQL 確認
新 view 返回 expected derived_state='ttl_expired' row：

```bash
# services/ledger/migrations/verify_p0.6_view.sql
SELECT reservation_id, derived_state, ttl_seconds, release_reason
  FROM reservations_with_ttl_status_v1
 WHERE current_state = 'released' AND release_reason = 'TTL_EXPIRED';
```

加 Makefile target：
```makefile
demo-verify-p0.6:
    @docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger \
        -f /var/spendguard/ledger-migrations/verify_p0.6_view.sql
```

**產出**：`DEMO_MODE=ttl_sweep make demo-up && make demo-verify-p0.6` 真跑 PASS。

執行慣例
========
- **Branch**: `feat/cost-advisor-p0.6`
- **Commit message**: `[CA-P0.6]` prefix，例如：
  `feat(ledger): reservations_with_ttl_status_v1 view + ttl_sweep verify [CA-P0.6]`
- **Codex challenge**: 完成後跑一輪（migration touches hot ledger schema，
  風險高）
- **更新 issue**: 標進度於 #49

完成標準
========
- [ ] JOIN 路徑驗證通過（Step 1）
- [ ] migration 0039 apply 乾淨（Step 2）
- [ ] 3 個 unit fixture pass（Step 3）
- [ ] demo-verify-p0.6 PASS（Step 4）
- [ ] codex challenge 跑 1 輪以上達到 GREEN
- [ ] issue #49 標 closed

不要做
======
- 不要動 reservations 表 schema（locked，加 column 就是規格重做）
- 不要碰 ttl_sweeper code（它已經寫對 reason='TTL_EXPIRED'，只是 view
  之前沒接到）
- 不要在這個 issue 內接 cost_advisor crate（那是 #50 的事）
- 不要寫 materialized view（先用 view + EXPLAIN ANALYZE 驗證夠用）

開始
====
請執行 Step 1 JOIN 路徑驗證並回報 PASS/FAIL，再進 Step 2。
```

---

## Notes for the human running this prompt

- **Pre-req**: docker-compose 跑 + DEMO_MODE=ttl_sweep 確認 baseline 可運行
- **可能 surprise**: ledger_transactions 表中 reserve / release 同 decision_id 兩個 tx 的 JOIN — 如果 schema 設計不允許這個 JOIN，可能需要走另一條路（e.g. cloudevent_payload.decision_id 直接 JOIN）

## Related artifacts

- Spec: `../specs/cost-advisor-spec.md` §5.1 + §11.5 A2
- Audit: `../specs/cost-advisor-p0-audit-report.md` §8.1 + §8.2 (authoritative)
- Sibling: `cost-advisor-p0.5-prompt.md` (parallel workstream)
- Downstream: `cost-advisor-p1-prompt.md` (gated on this + P0.5)

## After P0.6 completes

If P0.5 also done → start CA-P1 (issue #50). The runtime + rule SQL consumes
this view.
