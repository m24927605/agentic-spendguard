---
title: "SLO、告警與事故演練 (Phase 5 S23)"
---

這頁是 production 的營運契約:operator 承諾的每一個數值目標、每一條對應到復原 runbook 的告警、每一個事故演練情境,全部寫在這裡。

`deploy/observability/prometheus-rules.yaml` 裡的 metrics 在 Prometheus 端落實這些 SLO;`deploy/observability/grafana-dashboard.json` 的 dashboard 負責呈現。

## SLO summary

| ID  | Name                          | Target            | Window  | Owner            |
|-----|-------------------------------|-------------------|---------|------------------|
| L1  | Decision latency (p99)        | < 250 ms          | 30 days | sidecar team     |
| L2  | Decision availability         | ≥ 99.9%           | 30 days | sidecar team     |
| L3  | Ledger commit success         | ≥ 99.95%          | 30 days | ledger team      |
| L4  | Audit outbox forward lag (p99)| < 60 s            | 24 h    | platform team    |
| L5  | Canonical ingest reject rate  | < 0.5%            | 24 h    | platform team    |
| L6  | Pricing snapshot age (p99)    | < 24 h            | 24 h    | pricing team     |
| L7  | Provider reconciliation lag   | < 4 h             | 24 h    | platform team    |
| L8  | Approval latency (p99)        | < 5 min business  | 30 days | approver oncall  |
| L9  | Fencing lease takeover rate   | < 1 / pod / hour  | 24 h    | sidecar team     |

上面的數值目標都是初版。各 owning team 每季重新檢視一次;任何變更都要在 `pricing_overrides_audit` 風格的 change log 留一筆稽核 row(TBD:我們會沿用既有表,或另開一張 `slo_changes` 表 — S23-followup)。

## Required metrics

來源:`deploy/observability/prometheus-rules.yaml` 引用這些名稱。✓ = 已 ship(對應的 slice 見 Source slice 欄);↻ = S23-followup。

| Metric                                                   | Source slice   | Status |
|----------------------------------------------------------|----------------|--------|
| `spendguard_decision_latency_seconds`                     | S23            | ↻      |
| `spendguard_decision_total{status}`                       | S23            | ↻      |
| `spendguard_ledger_transaction_total{outcome,code}`       | S23            | ↻      |
| `spendguard_ledger_lease_age_seconds{lease_name}`         | S1             | ✓      |
| `spendguard_outbox_pending_seconds{tenant}`               | S23            | ↻      |
| `spendguard_ingest_events_quarantined_total{reason}`      | S8             | ✓      |
| `spendguard_ingest_events_accepted_total{route}`          | S8             | ✓      |
| `spendguard_ingest_events_rejected_invalid_signature_total{route}` | S8    | ✓      |
| `spendguard_pricing_snapshot_age_seconds{provider}`       | S13            | ↻      |
| `spendguard_provider_reconciliation_lag_seconds{provider}`| S10            | ↻      |
| `spendguard_approval_latency_seconds{outcome}`            | S20            | ↻      |
| `spendguard_sidecar_fencing_acquire_total{action}`        | S4             | ↻      |

標 ↻ 的 row 還在 wiring 階段 — emit 端的程式碼已經在對應 service crate 裡,但還沒 publish 到 `/metrics` endpoint。canonical_ingest 的 `/metrics`(S8)是參考實作;照著它的 IngestMetrics + http server pattern 複製即可。

## Alert rules (sample)

完整一套放在 `deploy/observability/prometheus-rules.yaml`。這裡只節錄,給 on-call playbook 翻閱用。

### A1. Decision latency p99 above target

```
alert: SpendGuardDecisionLatencyHigh
expr: histogram_quantile(0.99, rate(spendguard_decision_latency_seconds_bucket[5m])) > 0.25
for: 10m
labels: { severity: page, slo: L1 }
annotations:
  summary: "Decision p99 > 250ms for 10m"
  runbook: "docs/operations/runbooks/L1-decision-latency.md"
```

Page 條件:持續 10 分鐘。

### A2. Decision unavailable

```
alert: SpendGuardDecisionUnavailable
expr: rate(spendguard_decision_total{status="error"}[5m]) / rate(spendguard_decision_total[5m]) > 0.001
for: 5m
labels: { severity: page, slo: L2 }
annotations:
  summary: "Decision error rate > 0.1% for 5m"
  runbook: "docs/operations/runbooks/L2-decision-availability.md"
```

### A3. Ledger commit failures

```
alert: SpendGuardLedgerCommitFailing
expr: rate(spendguard_ledger_transaction_total{outcome="error"}[5m]) / rate(spendguard_ledger_transaction_total[5m]) > 0.0005
for: 5m
labels: { severity: page, slo: L3 }
annotations:
  summary: "Ledger commit error rate > 0.05% for 5m"
  runbook: "docs/operations/runbooks/L3-ledger-commit.md"
```

### A4. Audit outbox lag

```
alert: SpendGuardOutboxLag
expr: histogram_quantile(0.99, rate(spendguard_outbox_pending_seconds_bucket[15m])) > 60
for: 15m
labels: { severity: page, slo: L4 }
annotations:
  summary: "Audit outbox p99 lag > 60s for 15m"
  runbook: "docs/operations/runbooks/L4-outbox-lag.md"
```

### A5. Canonical ingest reject rate

```
alert: SpendGuardCanonicalIngestRejecting
expr: rate(spendguard_ingest_events_rejected_invalid_signature_total[10m]) > 0.5
for: 10m
labels: { severity: page, slo: L5 }
annotations:
  summary: "Canonical ingest rejecting > 0.5 events/sec for 10m"
  runbook: "docs/operations/runbooks/L5-canonical-rejects.md"
```

### A6. Pricing snapshot stale

```
alert: SpendGuardPricingStale
expr: (time() - spendguard_pricing_snapshot_age_seconds) > 86400
for: 30m
labels: { severity: page, slo: L6 }
annotations:
  summary: "Latest pricing_version > 24h old"
  runbook: "docs/operations/runbooks/L6-pricing-stale.md"
```

這條一定要在 bundle-build 的 fail-closed gate 觸發「之前」就 page。

### A7. Provider reconciliation lag

```
alert: SpendGuardProviderReconciliationLag
expr: spendguard_provider_reconciliation_lag_seconds > 14400
for: 1h
labels: { severity: warn, slo: L7 }
annotations:
  summary: "Provider reconciliation > 4h behind for 1h"
  runbook: "docs/operations/runbooks/L7-recon-lag.md"
```

### A8. Approval latency

```
alert: SpendGuardApprovalLatency
expr: histogram_quantile(0.99, rate(spendguard_approval_latency_seconds_bucket[1h])) > 300
for: 30m
labels: { severity: warn, slo: L8 }
annotations:
  summary: "Approval p99 > 5m for 30m"
  runbook: "docs/operations/runbooks/L8-approval-latency.md"
```

### A9. Fencing takeover storm

```
alert: SpendGuardFencingTakeoverStorm
expr: increase(spendguard_sidecar_fencing_acquire_total{action="promote"}[1h]) > 1
for: 5m
labels: { severity: page, slo: L9 }
annotations:
  summary: "Fencing takeovers > 1 / hour — likely lease flap"
  runbook: "docs/operations/runbooks/L9-fencing-storm.md"
```

## Incident drill scenarios

每季輪一次演練。演練結果記在 `docs/operations/drill-log.md`(S23-followup 的範本)。

### Per-drill deep-dive runbooks

下面這幾份全文 runbook(followup #12)逐步帶過症狀、首要檢查、緩解、升級,以及每個演練用 compose 跑的預演 — 接 primary on-call 之前先讀過:

- [Lease lost mid-batch](drills/lease-lost-mid-batch.md) —
  驗證 outbox-forwarder + ttl-sweeper 裡 round-9 的 `is_leader_now()` gating。
- [Audit chain forwarder backlog](drills/audit-chain-forwarder-backlog.md)
  — 驗證 L4 SLO(audit-outbox forward lag)以及 forwarder 的 idempotency。
- [Strict-signature quarantine spike](drills/strict-signature-quarantine-spike.md)
  — 涵蓋下面高階的 D3,附上 `unknown_key` / `invalid_signature` / `key_expired` /
  `key_revoked` 各 reason 的完整 triage tree。
- [Approval TTL wave](drills/approval-ttl-wave.md) — sweeper
  突發處理 + round-9 atomic TTL guard。

下面高階的 D1–D4 條目留作 executive summary;on-call 實際照著做的是上面那幾份 per-drill 文件。

### D1. Ledger failover

Steps:
1. `kubectl delete pod <ledger-pod>`(或模擬 Postgres primary failover)。
2. 確認 A3 在 5 分鐘內觸發。
3. 確認 sidecar 的 fail policy(S22 matrix)依 `failPolicy.overrides`
   擋下新的金額相關 decision。
4. 確認 ledger-replica 升主 + 新的 ledger pod 成為 leader。
5. 確認復原後:A3 清除;in-flight 的 reservation 要嘛乾淨 commit,
   要嘛透過 TTL release。

Acceptance:
- failover 期間沒有任何 `audit_outbox_global_keys` UNIQUE violation。
- `audit_outbox.pending_forward = TRUE` 的數量在復原後 10 分鐘內回到 baseline。

### D2. Stale fencing lease handling

Steps:
1. 手動讓 active sidecar 的 fencing lease 過期(在 test env 直接 UPDATE
   `fencing_scopes`,或對已 kill 的 pod 等 TTL 自然到期)。
2. 確認 A9 剛好遞增 1。
3. 確認接手的 sidecar 第一個 decision 用的是 `fencing_epoch = N+1`。
4. 確認原 pod 的 in-flight commit(若有)從 SP 拿到 `FENCING_EPOCH_STALE`。

Acceptance:
- 出現一筆 `fencing_scope_events.action='promote'` 的 row。
- 沒有任何 `audit_outbox_global_keys` 撞鍵。

### D3. Signature failure handling

Steps:
1. 輪換某個 producer 的 Ed25519 key,但「不要」更新 verifier 的 trust store
   (`keys.json`)。
2. 確認 A5 遞增 + canonical_ingest 的 log 顯示 `key_revoked` / `unknown_key`
   的 quarantine reason。
3. 確認這些 row 落進 `audit_signature_quarantine`,且 claimed_canonical_bytes
   原樣保留。
4. 更新 verifier 的 trust store(rolling restart)。
5. 確認 A5 回到 baseline。

Acceptance:
- 輪換「前」的 row 在 `canonical_events` 裡(用舊 key 簽的)。
- 輪換「中」的 row 在 `audit_signature_quarantine` 裡。
- 輪換「後」的 row 在 `canonical_events` 裡(用新 key 簽的)。

### D4. Pricing outage

Steps:
1. 停掉 pricing-sync(把 crontab 清空,或暫停 pricing-sync worker)。
2. 等 24 小時。
3. 確認 A6 觸發。
4. 繼續等到 `bundle-build` 拒絕切新 bundle(S13-followup 會接這條 wiring)。
5. 重新啟用 pricing-sync。
6. 確認 A6 清除;bundle-build 恢復。

Acceptance:
- `pricing_sync_attempts.outcome` 的 log 顯示出這段空窗。
- 新鮮度空窗期間沒有出現假性的 budget enforcement
  (既有 bundle 繼續沿用自己凍結的 pricing tuple)。

## Owner page (per spec review standard)

| Component         | Page owner           | Backup           |
|-------------------|----------------------|------------------|
| Sidecar           | sidecar oncall       | platform oncall  |
| Ledger            | ledger oncall        | platform oncall  |
| Canonical Ingest  | platform oncall      | sidecar oncall   |
| Outbox forwarder  | platform oncall      | platform oncall  |
| TTL sweeper       | platform oncall      | platform oncall  |
| Webhook receiver  | platform oncall      | provider oncall  |
| Control Plane     | platform oncall      | sre              |
| Dashboard         | platform oncall      | sre              |

上面列出的每一份 runbook 在 GA 之前都「必須」補齊。S23 這份文件先把結構 ship 出來;per-alert 的 deep dive 是下一塊要補的。
