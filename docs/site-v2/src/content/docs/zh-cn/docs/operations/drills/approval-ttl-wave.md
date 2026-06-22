---
title: "演练:approval TTL 大批量过期"
---

季度演练。验证 approval 状态机 + sweeper 在突发负载下的表现:当大量 approval 同时过期
(例如某租户配了一条短 TTL 的 contract 规则,而审批团队整夜不在线),TTL sweeper 必须
正确处理这波突发,且不能出现以下情况:

- 漏处理或重复过期某些行。
- 绕过 round-9 的原子 TTL guard
  (`resolve_approval_request` SP:用户对一条已过 TTL 的 pending 行做 approve/deny
  → 409 CONFLICT)。
- TTL 到期后还有 approval 卡在 `pending`。
- 漏掉 round-5 的 system-actor 注入(最终行必须带上
  `resolved_by_subject = 'system:ttl-sweeper'` /
  `resolved_by_issuer = 'system:spendguard'`)。

## 本演练覆盖的内容

- Migration 0030 (round 5) —— TTL sweeper 的 system-actor 注入。
- Migration 0033 (round 9) —— resolve_approval_request SP 内部的原子 TTL guard。
- Migration 0035 (followup #4) —— `expired` transition 的 notification outbox 写入。
- 负载下的 S14 sweeper helper `expire_pending_approvals_due()`。

## 症状(on-call 看到的现象)

- 告警 A8 `SpendGuardApprovalLatencyHigh`(或其 expired-tail 变体)触发。
- `approval_requests` 查询:大量行处于 `state='pending' AND ttl_expires_at < now()`。
- `approval_events` 查询:短时间窗口内出现一批 `to_state='expired'` 事件。
- `approval_notifications` 查询:对应的一批 `transition_kind='expired'` 行待派发。
- 用户侧影响:在 ResumeAfterApproval 上等待的 adapter 收到表示 approval 已失效的
  typed error → 调用方抛出 typed exception。

## 第一步检查

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

## 缓解(短期解封)

### Sweeper 停摆(第 3 步显示上次动作距今 > 1 分钟)

TTL sweeper 没在跑。两种可能:

1. **Pod down**:`kubectl get pods -l app.kubernetes.io/component=ttl-sweeper`
   ,若不是 Running 就重启。
2. **Lease 丢失**:参照 `lease-lost-mid-batch.md` 演练 —— sweeper 日志会出现
   "lease expired locally" 告警。处理方式相同:重启该 pod。

### Sweeper 在跑,但这波突发太大

sweeper 通过 `expire_pending_approvals_due()` 批量处理,在一次 SP 调用里处理掉所有
过期行。如果这波突发很大(成千上万行),这个 SP 在单个事务里可能跑很久。

1. **检查 sweeper 日志**里 SP 完成那行:
   `expired N approvals via sweeper`。
2. **如果 SP 看起来卡住了**:到 Postgres 查长事务:
   ```sql
   SELECT pid, now() - xact_start AS duration, query
     FROM pg_stat_activity
    WHERE state <> 'idle'
      AND xact_start IS NOT NULL
    ORDER BY xact_start;
   ```
   sweeper 持有这把锁,会挡住新 approval 的创建/resolve(它们会卡在 row-level lock 上)。
3. **operator 监督下的批量上限**(S14-followup):未来会有一个 migration 给 SP 的
   单次调用批量大小加上限;目前只能要么等 sweeper 跑完,要么 operator 硬等。

### 大批量过期本就是预期内的(例如 test tenant)

无需处理。这些行已正确过期,带上了 system actor,notification 也已入队。

## 升级

- stuck-pending 计数持续增长 **5 分钟** → page approver oncall(设置短 TTL 的那条
  contract 由其负责)。
- sweeper **15 分钟** 没有进展 → page platform oncall;TTL sweeper 服务可能需要重启。
- 有 adapter 等待已 **超过 30 分钟** → page engineering manager。adapter 抛出的
  typed "approval lapsed" exception 意味着已经有用户影响。

## 排练

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

## 相关

- L8 SLO 定义:`docs/site/docs/operations/slos.md` 第 L8 行
- PR #2 round 5 commit `c084a26` —— TTL sweeper SP 修复(system actor 注入)
- PR #2 round 9 commit `8810c14` —— 原子 TTL guard
- PR #14 commit `6f8d4d5` (followup #4) —— notification outbox 写入
- 姊妹演练:`lease-lost-mid-batch.md` 覆盖 sweeper pod 丢失 lease 的场景(上游
  事故模式)
