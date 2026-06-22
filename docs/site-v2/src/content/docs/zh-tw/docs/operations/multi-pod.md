---
title: "Multi-pod 部署 runbook (Phase 5 S5)"
---

這份 runbook 說明在 S1（lease primitive）、S2（per-pod producer instance id）、S3（Ledger fencing RPC）、S4（sidecar fencing-lease 生命週期）都已上線之後,如何安全地把 sidecar、outbox-forwarder、ttl-sweeper 這三個服務做水平 / 多節點部署。

**TL;DR**:outbox-forwarder 與 ttl-sweeper 可以安全擴到 N 個 pod,用 leader election 協調。sidecar 是 DaemonSet — 每個節點一個 pod — 同一時間只有一個 sidecar pod 能持有設定好的 fencing scope;其餘的會在啟動時 fail-closed,等著接手。這是 **active/standby**,不是水平擴展。

## 各元件的擴展模型

### outbox-forwarder

- **Type**:Deployment。
- **Multi-pod 模型**:leader election。只有 leader 會輪詢 `audit_outbox` table 並轉送到 canonical-ingest;standby replica 會持續對 lease 送 heartbeat,具備接手資格。
- **Helm gate**:當 `leaderElection.mode = disabled` 時,`outboxForwarder.replicas > 1` 會被拒絕(S1 gate;這是預期行為)。
- **要設什麼**:
  ```yaml
  outboxForwarder:
    replicas: 2  # or 3 for region-spread
  leaderElection:
    mode: postgres   # or k8s after S5+S7
    region: us-west-2
    ttlMs: 15000
    renewIntervalMs: 5000
  ```
- **Failover 行為**:當 active 的 leader pod 掛掉,Postgres lease 的 TTL 會在 `ttlMs`(預設 15s)之後過期。某個 standby 呼叫 `acquire_lease` SP 並勝出,轉送即恢復。不會產生重複的 canonical event,因為 `audit_outbox.pending_forward` 是 durable cursor。

### ttl-sweeper

- **Type**:Deployment。
- **Multi-pod 模型**:與 outbox-forwarder 完全一致(leader election;只有 leader 輪詢 + 釋放過期的 reservation)。
- **Helm gate**:`leaderElection.mode=disabled` 時,`ttlSweeper.replicas > 1` 會被拒絕(S1)。
- **建議值**:`replicas: 2` 做 HA。再往上加沒有額外吞吐量(只有一個 pod 在 sweep)。

### sidecar

- **Type**:DaemonSet(設計上每個節點一個 pod — 與掛載 UDS adapter socket 的 workload pod 共置)。
- **Multi-pod 模型**:每個 pod 透過 downward API 由 `metadata.name` 推導出唯一的 `workload_instance_id`(S2)。啟動時每個 pod 呼叫 `Ledger.AcquireFencingLease`(S4);Ledger SP 用 `FOR UPDATE` 序列化,並把 lease 只授予其中一個 pod。其餘 pod 在啟動時 fail-closed,帶 `S4: acquire fencing lease at startup`,停在 CrashLoopBackOff 或 Pending。
- **這不是水平擴展**。任一時刻,每個 fencing scope 只有一個 active、負責決策服務的 sidecar。
- **那為什麼要用 DaemonSet?** 為了共置:每個節點都有一個同節點上 app pod 能連到的 UDS socket。fencing scope 是 per-tenant(或 per-tenant×region);只有一個節點的 sidecar 持有它。
- **Helm gate**:必須設 `sidecar.acknowledgeMultiPod=true`,以明確表達 operator 已知曉 active/standby 語意。啟用 multi-pod 時,`workloadInstanceIdOverride` 不可設定(override 代表單一 pod 身分)。

## Failover 與 takeover

### Sidecar fencing takeover

當 active 的 sidecar pod 掛掉(OOM、eviction、節點故障):

1. 該 pod 的 `AcquireFencingLease` lease 在 `SPENDGUARD_SIDECAR_FENCING_TTL_SECONDS`(預設 30s)之後逾時。
2. 其他節點上、啟動時就 crash 的 standby sidecar 會由 kubelet 重啟。重啟後它們會再次呼叫 `Ledger.AcquireFencingLease`。
3. Ledger SP 看到前一個 lease 已過期,於是授予新 pod 一個 `takeover` action,帶 `epoch_increment = 1`。新 pod 的稽核 row 現在會以 `fencing_epoch = N+1` 簽章。
4. 舊 pod 任何 in-flight 的決策若試圖以 `fencing_epoch = N` commit,會被 Ledger 的 CAS 檢查擋掉(`FENCING_EPOCH_STALE` error)。稽核不變式(「沒有有效 epoch 就不生效」)維持成立。

Operator dashboard 會呈現:
- `spendguard_sidecar_fencing_epoch` gauge(per pod)
- `spendguard_sidecar_fencing_acquire_action_total{action}` counter(acquire / renew / takeover)— takeover 突增代表發生了 failover。

### Outbox-forwarder leader change

- `coordination_lease_history` table 是稽核日誌:每次 takeover 都會寫一筆 row,帶 `event_type = 'taken_over'` 與 `transition_count + 1`。
- Operator 監看 `spendguard_outbox_forwarder_leader_age_seconds` histogram 與 `coordination_lease_history` 的 row。

## 回退到單一 pod

三個服務的回退都只是:

```yaml
sidecar:
  acknowledgeMultiPod: false  # if you set it
outboxForwarder:
  replicas: 1
ttlSweeper:
  replicas: 1
```

不需要動 DB。lease/fencing 狀態存在 Postgres,由當下還活著的那個 pod 來 renew / take over。

## Chaos drill 檢查清單

S5 的驗收標準要求一個「kind test:兩個 sidecar、兩個 forwarder、兩個 sweeper,全部 healthy」。在那個自動化測試落地之前(延後到 S5-followup),operator 應手動驗證:

1. 以 `outboxForwarder.replicas=2`、`ttlSweeper.replicas=2`、sidecar DaemonSet 部署在 2-node cluster。
2. 確認 `coordination_leases` 顯示每個 `lease_name`(`outbox-forwarder`、`ttl-sweeper`)正好一個 leader。
3. 確認 `fencing_scopes` 顯示每個 scope 正好一個 `current_holder_instance_id`。
4. `kubectl delete pod <leader>`。等 `ttlMs + grace`(預設約 30s)。
5. 確認 `coordination_lease_history` 多了一筆 `taken_over` row。
6. 確認 ledger / canonical-ingest 沒看到重複的稽核 row(`audit_outbox_global_keys` 對 `(tenant, workload_instance_id, producer_sequence)` 的 UNIQUE 會擋掉重複)。
7. 對 sidecar 重複一次:`kubectl delete pod <active-sidecar>` — 另一節點上的 standby sidecar 以 epoch+1 接手。

## Observability 不變式

每個具備 S1+S4 意識的部署都應對以下情況告警:

- `coordination_lease_history` 中 `event_type='taken_over'` 的 row,每小時每個 lease 超過 1 次 — 很可能是 lease-flap(TTL 太短或網路分區)。
- `fencing_scope_events` 中 `action='promote'` 每小時超過 1 次 — sidecar takeover storm。
- sidecar pod 處於 `CrashLoopBackOff`、log 內帶 `acquire fencing lease at startup` 超過 5 分鐘 — 通常代表 seeded 的 scope row 不見了,或 workload identity 撞號。

## 已知限制 (S5-followup)

1. **Per-pod fencing scope** 尚未支援。所有節點上的所有 sidecar pod 共用設定好的 `sidecar.fencingScopeId`。真正的水平擴展需要 per-pod 的 scope 指派;以 S5-followup 追蹤。
2. 上述 chaos drill 的 **自動化 kind test** 延後。
3. takeover 期間的 **sidecar pre-stop drain** 已就位(S4),但 takeover SP 目前還不會撤銷前持有者的 lease — 它只是讓 TTL 自然過期。要更快 takeover 會需要一個明確的 revoke RPC。
