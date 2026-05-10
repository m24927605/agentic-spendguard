# SLOs, alerts, and incident drills (Phase 5 S23)

This page is the production operating contract: every numeric
target an operator commits to, every alert mapped to a remediation
runbook, every incident-drill scenario.

Metrics in `deploy/observability/prometheus-rules.yaml` enforce
these SLOs in Prometheus; the dashboard at
`deploy/observability/grafana-dashboard.json` renders them.

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

Numeric targets above are first-cut. Each owning team revisits
quarterly; all changes require an audit row in
`pricing_overrides_audit`-style change log (TBD: we'll repurpose
or add a separate `slo_changes` table — S23-followup).

## Required metrics

Source: `deploy/observability/prometheus-rules.yaml` references
these names. ✓ = shipped (which slice); ↻ = S23-followup.

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

The ↻ rows are wiring — emit-side code lives in the relevant
service crate but isn't yet published to a `/metrics`
endpoint. canonical_ingest's `/metrics` (S8) is the reference
implementation; replicate the IngestMetrics + http server pattern.

## Alert rules (sample)

The full set lives in `deploy/observability/prometheus-rules.yaml`.
Excerpts here for the on-call playbook to read.

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

Page condition: 10 minutes sustained.

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

This must page BEFORE the bundle-build fail-closed gate fires.

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

Quarterly drill rotation. The drill log at
`docs/operations/drill-log.md` (S23-followup template) records
results.

### Per-drill deep-dive runbooks

These full-text runbooks (followup #12) walk through symptoms,
first-check, mitigation, escalation, and a compose-based
rehearsal for each drill — read them before being primary
on-call:

- [Lease lost mid-batch](drills/lease-lost-mid-batch.md) —
  validates round-9 `is_leader_now()` gating in
  outbox-forwarder + ttl-sweeper.
- [Audit chain forwarder backlog](drills/audit-chain-forwarder-backlog.md)
  — validates the L4 SLO (audit-outbox forward lag) + the
  forwarder's idempotency.
- [Strict-signature quarantine spike](drills/strict-signature-quarantine-spike.md)
  — covers the high-level D3 below with the full triage tree
  for `unknown_key` / `invalid_signature` / `key_expired` /
  `key_revoked` reasons.
- [Approval TTL wave](drills/approval-ttl-wave.md) — sweeper
  burst handling + round-9 atomic TTL guard.

The high-level D1–D4 entries below stay as the executive
summary; the per-drill docs above are what on-call actually
reads.

### D1. Ledger failover

Steps:
1. `kubectl delete pod <ledger-pod>` (or simulate Postgres
   primary failover).
2. Verify A3 fires within 5 minutes.
3. Verify sidecar fail policy (S22 matrix) blocks new
   monetary decisions per `failPolicy.overrides`.
4. Verify ledger-replica promotion + new ledger pod becomes
   leader.
5. Verify post-recovery: A3 clears; in-flight reservations
   either commit cleanly or release via TTL.

Acceptance:
- No `audit_outbox_global_keys` UNIQUE violations during the
  failover.
- `audit_outbox.pending_forward = TRUE` count returns to
  baseline within 10 minutes of recovery.

### D2. Stale fencing lease handling

Steps:
1. Manually expire the active sidecar's fencing lease (UPDATE
   `fencing_scopes` in test env, or wait for natural TTL on a
   killed pod).
2. Verify A9 increments by exactly 1.
3. Verify the takeover sidecar's first decision uses
   `fencing_epoch = N+1`.
4. Verify the prior pod's in-flight commit (if any) gets
   `FENCING_EPOCH_STALE` from the SP.

Acceptance:
- `fencing_scope_events.action='promote'` row appears.
- No `audit_outbox_global_keys` collisions.

### D3. Signature failure handling

Steps:
1. Rotate one producer's Ed25519 key WITHOUT updating the
   verifier's trust store (`keys.json`).
2. Verify A5 increments + the canonical_ingest log shows
   `key_revoked` / `unknown_key` quarantine reason.
3. Verify the rows land in `audit_signature_quarantine` with
   correct claimed_canonical_bytes preserved.
4. Update verifier's trust store (rolling restart).
5. Verify A5 returns to baseline.

Acceptance:
- The pre-rotation rows ARE in `canonical_events` (signed
  with old key).
- The mid-rotation rows are in `audit_signature_quarantine`.
- The post-rotation rows ARE in `canonical_events` (signed
  with new key).

### D4. Pricing outage

Steps:
1. Disable pricing-sync (set crontab to empty, or pause the
   pricing-sync worker).
2. Wait 24 hours.
3. Verify A6 fires.
4. Continue waiting until `bundle-build` refuses to cut new
   bundles (S13-followup wires this).
5. Re-enable pricing-sync.
6. Verify A6 clears; bundle-build resumes.

Acceptance:
- `pricing_sync_attempts.outcome` log shows the gap.
- No spurious budget enforcement during the freshness gap
  (existing bundles continue using their frozen pricing tuple).

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

Each runbook listed above MUST be filled in before GA. The
S23 doc ships the structure; the per-alert deep dives are
the next chunk.
