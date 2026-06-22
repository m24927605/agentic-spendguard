---
title: "演練:audit chain forwarder 積壓"
---

每季演練。端到端驗證 audit chain:當 `outbox-forwarder`
被暫停或變慢時,`audit_outbox` 佇列會成長,但不會掉任何 row;
一旦 forwarder 恢復,佇列就會排空,`canonical_events` 也會追上。

這是 L4 SLO 演練(audit-outbox-forward-lag p99 < 60s,
取 24h 視窗內)。

## 這個演練在驗什麼

- L4 SLO 目標。
- Alert A4 `SpendGuardOutboxLagHigh` 在文件記載的門檻觸發,
  並在恢復後解除。
- S8 的交易不變式:row 絕對不會被無聲丟棄。它們會留在
  `audit_outbox` 並帶著 `pending_forward = TRUE`,直到成功 forward
  (→ `canonical_events`)或被移到 `audit_signature_quarantine`
  (簽章不符 / unknown key)為止。
- forwarder 的冪等性:同一批積壓重跑兩次,對
  `canonical_events.audit_outbox_id` UNIQUE 來說是 no-op。

## 症狀(on-call 會看到什麼)

- Prometheus 上 Alert A4 `SpendGuardOutboxLagHigh` 發出 page。
- Dashboard 面板 `audit_outbox_pending_seconds` 顯示最舊 pending
  的 age 一直成長。
- `SELECT count(*) FROM audit_outbox WHERE pending_forward = TRUE`
  → 數字一直長,沒在排空。
- Dashboard 的 `canonical_events count` 面板持平,或成長速度
  比 `audit_outbox` 總 count 慢。
- 對使用者可見的影響:producer 端完全沒影響(sidecar /
  ledger / webhook 繼續寫 audit row)。消費 audit 的流程
  (下游 BI、compliance 匯出)會看到資料變舊。

## 第一步檢查

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

如果第 4 步顯示 `expired = TRUE` 且沒有 holder,那這次的上層事件
就是「lease 在 batch 中途掉了」——請改看那份演練。

## 緩解(短期解除阻塞)

走哪條路要看第 3 步的結果:

### Forwarder stalled(第 3 步回傳 0)

依升級程度排序的選項:

1. **重啟 forwarder pod**:
   ```bash
   kubectl delete pod <forwarder-pod>
   ```
   新 pod 會透過 leader election 拿到 lease,並從
   `audit_outbox.pending_forward = TRUE` 的 row(以 `recorded_at`
   排序)接著做。forward loop 是冪等的,所以部分重播是安全的。
2. **如果有多個 replica、只有一個卡住**:那個 pod 進入壞狀態
   ——把它砍掉;standby 會接手。
3. **如果所有 replica 都呈現同樣的 stall**:是 canonical_ingest
   端在拒收 → 去查 `audit_signature_quarantine` 有沒有最近的 row。
   「strict-signature-quarantine-spike」演練涵蓋這個情境。

### Forwarder 有在處理但積壓追不上(第 3 步非 0)

1. **暫時調高 forwarder replica 數量**(需 operator ack):
   ```bash
   kubectl scale deployment outbox-forwarder --replicas=2
   ```
   注意:只有 leader 會處理工作——多開的 replica 只是讓接手
   更快。要真正平行化,workload 必須做分區(per-tenant scope_id,
   而不是單一 global lease)。這是已知的 scope 限制。
2. **調整 `OUTBOX_FORWARDER_BATCH_SIZE`**(env var;default 100):
   調高是拿 per-batch latency 換 throughput。恢復期間先 bump
   到 500–1000,lag 清掉後再還原 default。

## 升級(Escalation)

- pending count **持續成長 15 分鐘** → page platform oncall。
- **60 分鐘**仍未恢復 → page engineering manager;
  考慮手動 SQL 排空(極度危險——只有在你真的懂 dedup
  不變式時才做)。
- lag 達 **24 小時** → SLO violation;記錄到
  `docs/site/docs/operations/drill-log.md`。

## 預演(Rehearsal)

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

## 相關連結

- L4 SLO 定義:`docs/site/docs/operations/slos.md` 的 L4 列
- Alert:A4 `SpendGuardOutboxLagHigh`,位於
  `deploy/observability/prometheus-rules.yaml`
- 姊妹演練:`lease-lost-mid-batch.md` —— forwarder 停下來是因為
  它的 lease 過期了,而不是因為它慢
