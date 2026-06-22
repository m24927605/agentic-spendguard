---
title: "演练:批处理中途丢失 lease"
---

季度演练。用来验证 round-9 在 `outbox-forwarder` 和 `ttl-sweeper`
里加的 expiry-aware `is_leader_now()` 门控:一旦 worker 本地的
lease 状态过期(续约任务卡住、超过 `expires_at`),worker 在下一轮
loop 必须立刻停止处理,而不是继续吃缓存里的 `Leader` 值干活。

这个演练是 `services/leases/src/lib.rs::tests::is_leader_now_*`
单元测试的线上对照版本。

## 这个演练覆盖什么

- `services/leases/src/lib.rs::LeaseState::is_leader_now()` ——
  PR #2 round 9(commit `8810c14`)加的 expiry-aware leader 检查。
- `services/outbox_forwarder/src/main.rs` 和
  `services/ttl_sweeper/src/main.rs` —— 消费侧门控,调用
  `is_leader_now()`,而不是直接 pattern-match 那个 variant。
- worker 在缓存状态过期时打的那行
  `warn!(expires_at = %expires_at, ...)` 日志。

## 现象(on-call 看到什么)

续约任务卡住时(比如 Postgres lease 后端命中了慢副本),worker
pod 不会 crash。取而代之的是:

- worker 日志出现 warn 行:`lease expired locally; skip
  batch until renewed`。
- 受影响 tenant 的 `audit_outbox.pending_forward = TRUE` 计数开始
  爬升(仅当受影响 worker 是 outbox-forwarder 时)。
- `reservations.current_state = 'reserved' AND ttl_expires_at <
  now()` 计数开始爬升(仅当受影响 worker 是 ttl-sweeper 时)。
- 卡顿够久的话,A4(`SpendGuardOutboxLagHigh`)或它的 ttl-sweeper
  对应告警最终可能触发。

## 先查这些

```bash
# Identify the worker pod and check its log for the local-expire warn:
kubectl logs -l app.kubernetes.io/component=outbox-forwarder --tail=200 \
  | grep -F "lease expired locally"

# Confirm the lease row in postgres:
psql -h $LEDGER_PG_HOST -U $LEDGER_PG_USER -d spendguard_ledger -c "
  SELECT lease_name, holder_workload_id, expires_at, expires_at < clock_timestamp() AS already_expired
    FROM coordination_leases
   WHERE lease_name IN ('outbox-forwarder', 'ttl-sweeper');
"

# If `already_expired = TRUE` AND another worker hasn't taken over,
# the renewal path is broken (not just stalled).
```

## 缓解(短期解阻塞)

如果某个 worker 卡在 warn loop 里、不再有进展:

1. **从 worker pod 检查 Postgres 连通性**:
   ```bash
   kubectl exec <worker-pod> -- pg_isready -h $LEDGER_PG_HOST -U spendguard
   ```
   连不上 → 升级给 platform/oncall(Postgres 故障是上游父事件)。
2. **重启受影响的 worker pod**,强制走一轮全新的
   `try_acquire`:
   ```bash
   kubectl delete pod <worker-pod>
   ```
   备用 replicas(或同一个 Deployment 拉起的替代 pod)会在
   `leaderElection.ttlMs` 之内接管。
3. **确认接管**,看上面那条 postgres lease 行:
   `holder_workload_id` 应该变成新 pod 的 id,且
   `expires_at` 应该往前推进。

## 升级路径

- 持续 **5 分钟**:page outbox-forwarder / ttl-sweeper
  team primary(按
  `docs/site/docs/operations/slos.md` 的 owner 表)。
- 持续 **15 分钟**且未接管:page platform
  oncall —— 说明是 Postgres lease 后端本身坏了,不只是单个
  worker pod 的问题。
- **30 分钟以上**:升级给 engineering manager,并开始考虑手动用
  SQL 释放该 lease(谨慎操作 —— 有 double-leadership 的风险)。

## 彩排(基于 compose 的 demo)

不碰 prod,在本地 demo 集群上验证这个演练:

```bash
# 1. Bring up the demo with both workers running.
make demo-up DEMO_MODE=invoice
docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger \
  -c "SELECT lease_name, holder_workload_id, expires_at FROM coordination_leases ORDER BY lease_name;"

# 2. Simulate renewal stall: pause the worker so its renewer can't
# run. The local lease state stays `Leader` but expires_at goes
# stale.
docker pause spendguard-outbox-forwarder

# 3. Wait past leaderElection.ttlMs (compose default: ~10s).
sleep 15

# 4. Unpause. The next poll iteration should hit is_leader_now() =
# false and emit the warn line BEFORE attempting forward_batch.
docker unpause spendguard-outbox-forwarder
sleep 3
docker logs spendguard-outbox-forwarder 2>&1 | tail -20 \
  | grep -E "lease expired locally|lease state = LEADER"

# Expected output: at least one "lease expired locally" line BEFORE
# the next "lease state = LEADER" (renewed).

# 5. Cleanup.
make demo-down
```

这个彩排每季度跑一次;轮换 operator,确保每个 on-call 在当 primary
之前,至少亲手执行过一次。

## 相关

- `docs/site/docs/operations/slos.md` —— D2(stale fencing lease)
  覆盖 sidecar 侧的类比场景:fencing-scope lease 老化过期,新的
  sidecar pod 以 `fencing_epoch = N+1` 接管。
- PR #2 round 9 commit `8810c14` —— `is_leader_now()` 的实际
  实现。
