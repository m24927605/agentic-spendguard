---
title: "SLO、告警与故障演练 (Phase 5 S23)"
---

本页是生产环境的运行契约:每一项 operator 承诺的数值目标、每一条映射到处置 runbook 的告警、以及每一个故障演练场景。

`deploy/observability/prometheus-rules.yaml` 里的指标在 Prometheus 中强制约束这些 SLO;`deploy/observability/grafana-dashboard.json` 的看板负责把它们渲染出来。

## SLO 总览

| ID  | 名称                          | 目标              | 窗口    | Owner            |
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

上面的数值目标都是初版。各 owning team 每季度复审一次;任何变更都要在 `pricing_overrides_audit` 风格的变更日志里留一条审计记录(TBD:我们会复用现有表,或者新增一张单独的 `slo_changes` 表 —— S23-followup)。

## 必需的指标

来源:`deploy/observability/prometheus-rules.yaml` 引用了这些名字。✓ = 已发布(注明哪个 slice);↻ = S23-followup。

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

标 ↻ 的行还在接线阶段 —— emit 侧代码已经落在对应 service crate 里,但还没暴露到 `/metrics` endpoint。canonical_ingest 的 `/metrics`(S8)是参考实现;照着它复刻 IngestMetrics + http server 那套写法即可。

## 告警规则(示例)

完整规则集在 `deploy/observability/prometheus-rules.yaml`。这里摘录几条,供 on-call playbook 参考。

### A1. Decision latency p99 超标

```
alert: SpendGuardDecisionLatencyHigh
expr: histogram_quantile(0.99, rate(spendguard_decision_latency_seconds_bucket[5m])) > 0.25
for: 10m
labels: { severity: page, slo: L1 }
annotations:
  summary: "Decision p99 > 250ms for 10m"
  runbook: "docs/operations/runbooks/L1-decision-latency.md"
```

Page 条件:持续 10 分钟。

### A2. Decision 不可用

```
alert: SpendGuardDecisionUnavailable
expr: rate(spendguard_decision_total{status="error"}[5m]) / rate(spendguard_decision_total[5m]) > 0.001
for: 5m
labels: { severity: page, slo: L2 }
annotations:
  summary: "Decision error rate > 0.1% for 5m"
  runbook: "docs/operations/runbooks/L2-decision-availability.md"
```

### A3. Ledger commit 失败

```
alert: SpendGuardLedgerCommitFailing
expr: rate(spendguard_ledger_transaction_total{outcome="error"}[5m]) / rate(spendguard_ledger_transaction_total[5m]) > 0.0005
for: 5m
labels: { severity: page, slo: L3 }
annotations:
  summary: "Ledger commit error rate > 0.05% for 5m"
  runbook: "docs/operations/runbooks/L3-ledger-commit.md"
```

### A4. Audit outbox 积压

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

### A6. Pricing snapshot 过期

```
alert: SpendGuardPricingStale
expr: (time() - spendguard_pricing_snapshot_age_seconds) > 86400
for: 30m
labels: { severity: page, slo: L6 }
annotations:
  summary: "Latest pricing_version > 24h old"
  runbook: "docs/operations/runbooks/L6-pricing-stale.md"
```

这条告警必须在 bundle-build 的 fail-closed gate 触发之前先 page 出来。

### A7. Provider reconciliation 积压

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

### A9. Fencing 抢主风暴

```
alert: SpendGuardFencingTakeoverStorm
expr: increase(spendguard_sidecar_fencing_acquire_total{action="promote"}[1h]) > 1
for: 5m
labels: { severity: page, slo: L9 }
annotations:
  summary: "Fencing takeovers > 1 / hour — likely lease flap"
  runbook: "docs/operations/runbooks/L9-fencing-storm.md"
```

## 故障演练场景

按季度轮换演练。演练日志在 `docs/operations/drill-log.md`(S23-followup 模板)记录结果。

### 单个演练的深度 runbook

下面这些全文 runbook(followup #12)会逐项走一遍症状、首查项、缓解、升级,以及每个演练基于 compose 的实战预演 —— 在你担任 primary on-call 之前先读它们:

- [Lease lost mid-batch](drills/lease-lost-mid-batch.md) ——
  验证 outbox-forwarder + ttl-sweeper 里 round-9 的
  `is_leader_now()` gating。
- [Audit chain forwarder backlog](drills/audit-chain-forwarder-backlog.md)
  —— 验证 L4 SLO(audit-outbox forward lag)以及
  forwarder 的幂等性。
- [Strict-signature quarantine spike](drills/strict-signature-quarantine-spike.md)
  —— 覆盖下面 D3 的概要,并给出针对
  `unknown_key` / `invalid_signature` / `key_expired` /
  `key_revoked` 这几种 reason 的完整 triage 树。
- [Approval TTL wave](drills/approval-ttl-wave.md) —— sweeper
  突发处理 + round-9 的原子 TTL guard。

下面的 D1–D4 概要条目作为 executive summary 保留;真正给 on-call 照着读的是上面这些单独的演练文档。

### D1. Ledger failover

步骤:
1. `kubectl delete pod <ledger-pod>`(或模拟 Postgres
   primary failover)。
2. 确认 A3 在 5 分钟内触发。
3. 确认 sidecar fail policy(S22 矩阵)按
   `failPolicy.overrides` 阻断新的涉及金额的 decision。
4. 确认 ledger-replica 完成 promotion,且新的 ledger pod
   成为 leader。
5. 确认恢复后的状态:A3 清除;in-flight 的 reservation
   要么干净 commit,要么经 TTL release。

验收标准:
- failover 期间没有 `audit_outbox_global_keys` UNIQUE 违约。
- `audit_outbox.pending_forward = TRUE` 的计数在恢复后
  10 分钟内回到基线。

### D2. 过期 fencing lease 的处理

步骤:
1. 手动让当前 active sidecar 的 fencing lease 过期
   (在测试环境 UPDATE `fencing_scopes`,或者在被
   kill 的 pod 上等其自然 TTL)。
2. 确认 A9 恰好 +1。
3. 确认抢主后的 sidecar 第一个 decision 用的是
   `fencing_epoch = N+1`。
4. 确认前一个 pod 的 in-flight commit(如果有)从 SP
   拿到 `FENCING_EPOCH_STALE`。

验收标准:
- 出现 `fencing_scope_events.action='promote'` 这一行。
- 没有 `audit_outbox_global_keys` 冲突。

### D3. 签名失败的处理

步骤:
1. 轮换某个 producer 的 Ed25519 key,但**不**更新
   verifier 的信任库(`keys.json`)。
2. 确认 A5 自增,且 canonical_ingest 日志显示
   `key_revoked` / `unknown_key` 这两类 quarantine reason。
3. 确认这些行落进 `audit_signature_quarantine`,且
   claimed_canonical_bytes 正确保留。
4. 更新 verifier 的信任库(滚动重启)。
5. 确认 A5 回到基线。

验收标准:
- 轮换前的行**确实**在 `canonical_events` 里(用旧 key 签的)。
- 轮换中途的行在 `audit_signature_quarantine` 里。
- 轮换后的行**确实**在 `canonical_events` 里(用新 key 签的)。

### D4. Pricing 中断

步骤:
1. 停掉 pricing-sync(把 crontab 清空,或暂停
   pricing-sync worker)。
2. 等 24 小时。
3. 确认 A6 触发。
4. 继续等,直到 `bundle-build` 拒绝再切新 bundle
   (S13-followup 接好这一段)。
5. 重新启用 pricing-sync。
6. 确认 A6 清除;bundle-build 恢复。

验收标准:
- `pricing_sync_attempts.outcome` 日志显示出这段空档。
- freshness 空档期内没有误触发的 budget 强制执行
  (已有 bundle 继续用它们冻结的 pricing tuple)。

## Owner 页(按 spec review standard)

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

上面列出的每个 runbook 都必须在 GA 之前补齐。S23 这份文档先把结构搭好;单条告警的深度展开是下一块工作。
