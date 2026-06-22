---
title: "演練:approval TTL 大量到期"
---

每季演練。驗證 approval 狀態機 + sweeper 在突發負載下的行為:當大量 approval 同時到期
(例如某租戶設了短 TTL 的 contract rule,而 approver team 整晚離線),TTL sweeper 必須
正確處理這波突發,且不能發生以下任一狀況:

- 漏掉或重複到期某些 row。
- 繞過 round-9 的 atomic TTL guard
  (`resolve_approval_request` SP:使用者對一個已 TTL 到期的 pending row 做 approve/deny
  → 409 CONFLICT)。
- 任何 approval 在 TTL 之後仍卡在 `pending`。
- 漏掉 round-5 的 system-actor 注入(row 結束時必須帶
  `resolved_by_subject = 'system:ttl-sweeper'` /
  `resolved_by_issuer = 'system:spendguard'`)。

## 這個演練在測什麼

- Migration 0030 (round 5) — TTL sweeper 的 system-actor 注入。
- Migration 0033 (round 9) — resolve_approval_request SP 內的 atomic TTL guard。
- Migration 0035 (followup #4) — `expired` transition 的 notification outbox 寫入。
- S14 的 sweeper helper `expire_pending_approvals_due()` 在負載下的表現。

## 症狀(on-call 會看到什麼)

- Alert A8 `SpendGuardApprovalLatencyHigh`(或其 expired-tail 變體)觸發。
- `approval_requests` 查詢:大量 row 處於 `state='pending' AND ttl_expires_at < now()`。
- `approval_events` 查詢:短時間內出現一波 `to_state='expired'` 事件。
- `approval_notifications` 查詢:對應出現一波 `transition_kind='expired'` 的 row 等待派送。
- 使用者可見的影響:等在 ResumeAfterApproval 的 adapter 收到表示 approval 已失效的
  typed error → caller 拋出 typed exception。

## 第一步檢查

```bash
# 1. Snapshot pending approvals past TTL.
psql -h $LEDGER_PG_HOST -U spendguard -d spendguard_ledger -c "
  SELECT count(*) AS stuck_pending,
         min(ttl_expires_at) AS oldest_overdue
    FROM approval_requests
   WHERE state = 'pending'
     AND ttl_expires_at < now();
"
# Non-zero stuck count means sweeper isn't keeping up.

# 2. Recent expire-event rate.
psql -h $LEDGER_PG_HOST -U spendguard -d spendguard_ledger -c "
  SELECT count(*) AS expired_in_5min
    FROM approval_events
   WHERE to_state = 'expired'
     AND occurred_at > now() - interval '5 minutes';
"

# 3. Sweeper liveness — when did it last act?
psql -h $LEDGER_PG_HOST -U spendguard -d spendguard_ledger -c "
  SELECT max(occurred_at) AS last_sweep_action
    FROM approval_events
   WHERE actor_subject = 'system:ttl-sweeper';
"

# 4. Notification outbox backlog from the burst.
psql -h $LEDGER_PG_HOST -U spendguard -d spendguard_ledger -c "
  SELECT count(*) AS pending_dispatch
    FROM approval_notifications
   WHERE transition_kind = 'expired'
     AND pending_dispatch = TRUE;
"
```

## 緩解(短期解除卡住)

### Sweeper 停擺(第 3 步顯示上次動作距今 > 1 分鐘)

TTL sweeper 沒在跑。兩種可能:

1. **Pod 掛了**:`kubectl get pods -l app.kubernetes.io/component=ttl-sweeper`
   若不是 Running 就重啟。
2. **Lease 掉了**:依照 `lease-lost-mid-batch.md` 演練 —
   sweeper log 會出現 "lease expired locally" 的 warn。處置相同:重啟該 pod。

### Sweeper 有在跑,但這波突發太大

Sweeper 透過 `expire_pending_approvals_due()` 批次處理,一次 SP 呼叫就把所有 overdue row
處理掉。若這波突發很大(數千 row),這個 SP 會在單一 transaction 內跑很久。

1. **檢查 sweeper log** 找 SP 完成那行:
   `expired N approvals via sweeper`。
2. **若 SP 看起來卡住**:到 Postgres 查 long-running transaction:
   ```sql
   SELECT pid, now() - xact_start AS duration, query
     FROM pg_stat_activity
    WHERE state <> 'idle'
      AND xact_start IS NOT NULL
    ORDER BY xact_start;
   ```
   Sweeper 握著 lock 會讓新的 approval 無法建立/resolve(它們會卡在 row-level lock 上等)。
3. **Operator 監督下的批次上限**(S14-followup):未來會有一個 migration 替 SP 加上
   per-call 批次大小上限;目前 sweeper 要嘛跑完、要嘛 operator 等它跑完。

### 這波大量到期本來就是預期的(例如 test tenant)

不用處置。Row 已正確以 system actor 到期 + notification 已入列。

## 升級(Escalation)

- stuck-pending 數量持續成長 **5 分鐘** → 呼叫 approver oncall
  (負責那個設了短 TTL 的 contract)。
- **15 分鐘** sweeper 毫無進度 → 呼叫 platform oncall;
  TTL sweeper service 可能需要重啟。
- **30 分鐘以上** 仍有 adapter 在等 → 呼叫 engineering manager。
  Adapter 拋出的 typed "approval lapsed" exception 代表已有使用者受影響。

## 預演(Rehearsal)

```bash
# 1. Bring up demo with TTL=5s so we can create a burst quickly.
SIDECAR_TTL_SECONDS=5 make demo-up DEMO_MODE=ttl_sweep
# (The PR #6 ttl_sweep mode wires this end-to-end.)

# 2. Generate a burst of pending approvals via direct SQL
# (workaround: real adapter-driven approvals in burst would
# require a test contract with REQUIRE_APPROVAL rules).
docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger -c "
  INSERT INTO approval_requests
    (approval_id, tenant_id, decision_id, audit_decision_event_id,
     state, ttl_expires_at, approver_policy, requested_effect,
     decision_context)
  SELECT
    gen_random_uuid(),
    '00000000-0000-4000-8000-000000000001',
    gen_random_uuid(),
    gen_random_uuid(),
    'pending',
    clock_timestamp() + interval '500 ms',
    '{}'::jsonb, '{}'::jsonb, '{}'::jsonb
  FROM generate_series(1, 50);
"

# 3. Wait past TTL.
sleep 2

# 4. Trigger the sweeper SP directly (it would normally be
# called by the ttl-sweeper service):
docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger -c "
  SELECT expire_pending_approvals_due() AS expired_count;
"
# Expected: 50.

# 5. Verify all rows are now expired with system actor.
docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger -c "
  SELECT state, resolved_by_subject, count(*)
    FROM approval_requests
   WHERE state IN ('expired', 'pending')
   GROUP BY state, resolved_by_subject;
"
# Expected: state=expired, resolved_by_subject=system:ttl-sweeper, count=50.

# 6. Verify notification rows landed (followup #4).
docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger -c "
  SELECT transition_kind, count(*)
    FROM approval_notifications
   GROUP BY transition_kind;
"
# Expected: transition_kind=expired with N rows for the tenant
# IFF that tenant has a row in tenant_notification_config; 0
# rows if no config (followup #4 default behavior).

make demo-down
```

## 相關連結

- L8 SLO 定義:`docs/site/docs/operations/slos.md` row L8
- PR #2 round 5 commit `c084a26` — TTL sweeper SP 修正(system actor 注入)
- PR #2 round 9 commit `8810c14` — atomic TTL guard
- PR #14 commit `6f8d4d5` (followup #4) — notification outbox 寫入
- 姊妹演練:`lease-lost-mid-batch.md` 涵蓋 sweeper pod 掉 lease 的情境(本案的母事件型態)
