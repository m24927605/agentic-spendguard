---
title: "演练:audit chain forwarder 积压"
---

季度演练。端到端验证 audit chain:当 `outbox-forwarder` 暂停或变慢时,
`audit_outbox` 队列会增长,但不会丢失任何行;一旦 forwarder 恢复,队列排空,
`canonical_events` 追平。

这是 L4 SLO 演练(audit-outbox-forward-lag p99 < 60s,以 24h 窗口计)。

## 这个演练验证什么

- L4 SLO 目标。
- 告警 A4 `SpendGuardOutboxLagHigh` 在文档规定的阈值上触发,恢复后清除。
- 来自 S8 的事务不变式:行绝不会被静默丢弃。它们以 `pending_forward =
  TRUE` 的状态留在 `audit_outbox`,直到要么转发成功(→
  `canonical_events`),要么移入 `audit_signature_quarantine`(签名不匹配 /
  未知 key)。
- forwarder 的幂等性:把同一批积压重跑两次,对 `canonical_events.audit_outbox_id`
  UNIQUE 约束而言是个 no-op。

## 现象(on-call 看到的)

- 告警 A4 `SpendGuardOutboxLagHigh` 在 Prometheus 上 page。
- 仪表盘面板 `audit_outbox_pending_seconds` 显示最老 pending 行的滞留时长在增长。
- `SELECT count(*) FROM audit_outbox WHERE pending_forward = TRUE`
  → 数字在涨,不在排空。
- 仪表盘的 `canonical_events count` 面板持平,或增长慢于 `audit_outbox` 的总行数。
- 用户可见影响:生产者侧无影响(sidecar / ledger / webhook 继续写审计行)。
  消费审计的流程(下游 BI、合规导出)会看到数据陈旧。

## 第一步检查

```bash
# 1. Confirm the forwarder pod is still running.
kubectl get pods -l app.kubernetes.io/component=outbox-forwarder -o wide
# All Running? Continue. Any CrashLoopBackOff? Tail logs:
kubectl logs <forwarder-pod> --tail=200

# 2. Pending count + oldest age (single SQL query):
psql -h $LEDGER_PG_HOST -U spendguard -d spendguard_ledger -c "
  SELECT count(*) AS pending,
         max(now() - recorded_at) AS oldest_age,
         min(recorded_at) AS oldest_recorded_at
    FROM audit_outbox
   WHERE pending_forward = TRUE;
"

# 3. Are forwards still landing? Compare canonical_events count
# now vs 1 minute ago:
psql -h $CANONICAL_PG_HOST -U spendguard -d spendguard_canonical -c "
  SELECT count(*) FROM canonical_events
   WHERE recorded_at > now() - interval '1 minute';
"
# 0 = forwarder is stalled. Non-zero = forwarder is processing,
# just behind on backlog.

# 4. Check the forwarder's lease state (validates "lease lost
# mid-batch" isn't actually the parent incident):
psql -h $LEDGER_PG_HOST -U spendguard -d spendguard_ledger -c "
  SELECT holder_workload_id, expires_at, expires_at < now() AS expired
    FROM coordination_leases
   WHERE lease_name = 'outbox-forwarder';
"
```

如果第 4 步显示 `expired = TRUE` 且没有 holder,那父事件就是 "lease lost mid-batch"
—— 改去看那个演练。

## 缓解(短期解除阻塞)

走哪条路取决于第 3 步的结果:

### Forwarder 卡死(第 3 步返回 0)

按升级顺序的选项:

1. **重启 forwarder pod**:
   ```bash
   kubectl delete pod <forwarder-pod>
   ```
   新 pod 通过 leader election 拿到 lease,从 `audit_outbox.pending_forward = TRUE`
   的行按 `recorded_at` 排序续跑。转发 loop 是幂等的,所以部分重放是安全的。
2. **如果有多个 replica 而只有一个卡住**:那个 pod 处于异常状态 —— kill 掉它,
   standby 接管。
3. **如果所有 replica 都是同样的卡死**:那是 canonical_ingest 侧在拒绝 →
   去看 `audit_signature_quarantine` 里最近的行。"strict-signature-quarantine-spike"
   演练覆盖那个场景。

### Forwarder 在处理但积压了(第 3 步非零)

1. **临时调高 forwarder replica 数**(需要 operator ack):
   ```bash
   kubectl scale deployment outbox-forwarder --replicas=2
   ```
   注意:只有 leader 真正处理任务 —— 多出来的 replica 只是让接管更快。要真正并行,
   工作负载必须分片(按 per-tenant scope_id,而不是单个全局 lease)。这是一个已知的
   scope 限制。
2. **调 `OUTBOX_FORWARDER_BATCH_SIZE`**(环境变量;默认 100):值越高,
   用每批延迟换吞吐。恢复期间提到 500–1000,lag 清除后恢复默认值。

## 升级

- pending 数持续增长 **15 分钟** → page platform oncall。
- **60 分钟**仍未恢复 → page engineering manager;
  考虑手工 SQL 排空(非常危险 —— 仅在你理解 dedup 不变式时才做)。
- lag 达 **24 小时** → SLO 违规;记录到
  `docs/site/docs/operations/drill-log.md`。

## 排练

```bash
# 1. Bring up demo with full chain.
make demo-up DEMO_MODE=invoice

# 2. Pause the forwarder.
docker pause spendguard-outbox-forwarder

# 3. Generate audit traffic by re-running the demo a few times.
for i in 1 2 3; do
  make demo-up DEMO_MODE=decision
done

# 4. Confirm pending count grows.
docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger \
  -c "SELECT count(*) AS pending FROM audit_outbox WHERE pending_forward = TRUE;"
# Expected: > 5 rows pending.

# 5. Resume the forwarder.
docker unpause spendguard-outbox-forwarder
sleep 10

# 6. Confirm drain.
docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger \
  -c "SELECT count(*) AS pending FROM audit_outbox WHERE pending_forward = TRUE;"
# Expected: pending count back to baseline (0 or close).

# 7. Confirm canonical_events caught up.
docker exec spendguard-postgres psql -U spendguard -d spendguard_canonical \
  -c "SELECT count(*) FROM canonical_events;"
# Expected: count matches the total audit rows generated.

make demo-down
```

## 相关

- L4 SLO 定义:`docs/site/docs/operations/slos.md` 第 L4 行
- 告警:`deploy/observability/prometheus-rules.yaml` 中的 A4 `SpendGuardOutboxLagHigh`
- 姊妹演练:`lease-lost-mid-batch.md` —— forwarder 停下是因为 lease 过期失效,
  而不是因为它慢
