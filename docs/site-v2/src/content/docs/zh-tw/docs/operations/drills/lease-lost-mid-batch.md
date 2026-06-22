---
title: "演練:批次中途掉 lease"
---

每季演練。用來驗證 round-9 在 `outbox-forwarder` 與 `ttl-sweeper`
裡那套有 expiry 意識的 `is_leader_now()` gating:當某個 worker 的本地
lease 狀態變 stale(renewal task 卡住、過了 `expires_at`),這個
worker 在下一輪 loop 一定要停手,而不是繼續拿 cache 住的 `Leader`
值硬跑。

這個演練是 `services/leases/src/lib.rs::tests::is_leader_now_*` 那些
unit test 的線上對照版。

## 這個演練實際打的路徑

- `services/leases/src/lib.rs::LeaseState::is_leader_now()` —— PR #2
  round 9(commit `8810c14`)加進來、有 expiry 意識的 leader 檢查。
- `services/outbox_forwarder/src/main.rs` 與
  `services/ttl_sweeper/src/main.rs` —— consumer 端的 gate,改成呼叫
  `is_leader_now()`,而不是直接 pattern-match variant。
- worker 那行在 cache 狀態 stale 時會噴的
  `warn!(expires_at = %expires_at, ...)` log。

## 症狀(on-call 會看到什麼)

當 renewal task 卡住(例如 Postgres lease backend 打到慢的
replica),worker pod 不會 crash。取而代之的是:

- worker log 出現這行 warn:`lease expired locally; skip
  batch until renewed`。
- 受影響的 tenant 上,`audit_outbox.pending_forward = TRUE` 的數量
  開始往上爬(只在 outbox-forwarder 是受影響 worker 時才會)。
- `reservations.current_state = 'reserved' AND ttl_expires_at <
  now()` 的數量開始往上爬(只在 ttl-sweeper 是受影響 worker 時才
  會)。
- 如果卡得夠久,A4(`SpendGuardOutboxLagHigh`)或它在 ttl-sweeper
  那邊的對應告警,最後可能會 fire。

## 先做的檢查

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

## 緩解(短期解 block)

如果某個 worker 卡在 warn loop、沒有進展:

1. **從 worker pod 確認 Postgres 連通性**:
   ```bash
   kubectl exec <worker-pod> -- pg_isready -h $LEDGER_PG_HOST -U spendguard
   ```
   如果連不上 → 升級給 platform / oncall(Postgres outage 才是
   上游母 incident)。
2. **重啟受影響的 worker pod**,強制走一次新的
   `try_acquire` cycle:
   ```bash
   kubectl delete pod <worker-pod>
   ```
   standby replica(或同一個 Deployment 補上來的 pod)會在
   `leaderElection.ttlMs` 內接手。
3. **確認接手成功**,看上面那筆 postgres lease row:
   `holder_workload_id` 應該換成新 pod 的 id、
   `expires_at` 應該往前推進。

## 升級

- 持續 **5 分鐘**:page outbox-forwarder / ttl-sweeper 團隊
  primary(依
  `docs/site/docs/operations/slos.md` 的 owner table)。
- 持續 **15 分鐘**仍沒接手:page platform
  oncall —— 代表是 Postgres lease backend 本身壞了,
  不只是單一 worker pod。
- **30 分鐘以上**:升級給 engineering manager,並開始
  考慮用手動 SQL 釋放 lease(要小心 —— 有
  double-leadership 的風險)。

## 預演(compose-based demo)

不碰 prod,拿本地 demo cluster 來驗這個演練:

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

這個預演每季跑一次;輪替 operator,確保每位 on-call 在當 primary
之前,至少都實際跑過一次。

## 相關

- `docs/site/docs/operations/slos.md` —— D2(stale fencing lease)
  講的是 sidecar 端的對應情境:當某個 fencing-scope lease 過期、
  由新的 sidecar pod 帶著 `fencing_epoch = N+1` 接手。
- PR #2 round 9 commit `8810c14` —— `is_leader_now()` 實際的
  實作。
