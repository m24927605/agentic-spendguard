# Stage 2 POC Topology Specification — v1alpha1 (LOCKED)

> 🔒 **Status: LOCKED implementation spec**  
> **Lock date**: 2026-05-07  
> **Lock judgment basis**: Codex round 3 minimal verification — 「No new §0.2 critical decisions; cross-spec invariants hold; minor v2.1 patch (audit_outbox decision_id partition-safe partial unique + topology diagram arrow disambiguation) applied. Ready to lock.」  
> **Adoption history**: Round 1/2/3 採納率 100%/100%/100%（3 輪零實質反駁；round 1 = 37 findings；round 2 = 22 patches；round 3 = 2 minor patches）  
> **Companions**:
> - `agent-runtime-spend-guardrails-complete.md` (v1.3 strategy)
> - `contract-dsl-spec-v1alpha1.md` (LOCKED)
> - `trace-schema-spec-v1alpha1.md` (LOCKED)
> - `sidecar-architecture-spec-v1alpha1.md` (LOCKED)
> - `ledger-storage-spec-v1alpha1.md` (LOCKED)
> 
> **Compatibility policy**: alpha — service decomposition stable / wire protocol gRPC additive only / Postgres durability boundary commitment / pricing build-time freeze / audit path uniformity (no direct CI from non-ledger producers) / 22 round-2 + 2 round-3 patches integrated

---

## 0. Lock Status & POC Scope

### 0.1 範圍
覆蓋 Phase 1 first customer (K8s SaaS-managed) reference implementation POC topology。**不**覆蓋 Lambda / Cloud Run / air-gapped / multi-region failover / SPIRE / customer-managed key（Phase 2+）。

### 0.2 不可逆決策清單（v2 重新整理）
| # | 決策 | §位置 | vs v1 |
|---|---|---|---|
| 🔒 D1 | 5 services + 3 POC tiers | §3 | unchanged |
| 🔒 D2 | Audit durability via ledger transactional outbox + **synchronous_commit + sync replica quorum** | §4 | 強化 durability boundary |
| 🔒 D3 | Bundle Registry = 1 service 3 namespaces | §5 | unchanged |
| 🔒 D4 | Stage Output Persistence path: sidecar → ledger atomic commit (sync ack) → outbox forwarder → CI → immutable_audit_log | §6 | unchanged |
| 🔒 D5 | Sidecar binary 用 Rust（含 §7.4 production-debug gates） | §7 | unchanged |
| 🔒 D6 | Wire protocol baseline | §8 | NormalizeCost 移除為 hot-path；BudgetClaim + MinimalReplayResponse 具體 proto |
| 🔒 D7 | Trust bootstrap via Helm root CA bundle + cert-manager external issuer | §12.1 | unchanged |
| 🔒 D8 | Sidecar injection via explicit Helm/Kustomize | §9.1 | unchanged |
| 🔒 D9 | Provider Webhook Receiver 為 control plane 唯一 provider event 入口；**audit 統一走 ledger.audit_outbox** | §11 | 修：webhook 不直接 emit CI |
| 🔒 D10 | **Pricing snapshot freezed at contract bundle build time**（不在 hot path 動態查 Platform Pricing DB） | §9.4, §10 | 重大變更（per Contract §12 + Codex C-2.3, C-2.5） |
| 🔒 D11 | Ledger fencing 為唯一 lease authority；audit_outbox append 攜帶 ledger_fencing_epoch | §4.4 | unchanged |
| 🔒 D12 | **CI per-decision_id sequence enforcement**：`audit.outcome` 必須 strictly after `audit.decision` | §4.8 | NEW per Codex A5 |

### 0.3 Lock 條件（已達成）
1. ✅ Codex round 1 反饋 100% 採納（37 findings）→ v1
2. ✅ Codex round 2 反饋 100% 採納（22 patches）→ v2
3. ✅ Codex round 3 minimal verification — 5 lock recommendation = ⚠️ MINOR PATCH BEFORE LOCK；2 minor patches (v2.1) 已 in-place applied → 升 v1alpha1 LOCKED
4. v0 + v1 RFC 已刪除；轉移歷史至 §22 adoption history

### 0.4 Sidecar §6 物理映射 note（per Codex round 2 §3 Sidecar ⚠️）
- Sidecar §6.2 spec 列舉 3 個 abstract durability options（canonical_ingest_ack / remote_decision_journal_ack / persistent_local_wal_with_replay_guarantee）
- 對 K8s SaaS-managed mode，Sidecar §6.2 偏好 `remote_decision_journal_ack`
- **本 RFC 不修 Sidecar spec**；只標明物理實作：K8s SaaS-managed 模式下 `remote_decision_journal_ack` 的物理 ack source = **ledger.audit_outbox commit ack**（per Ledger §20.4 ledger durable anchor + 本 RFC §4）
- 客戶 dashboard 顯示：`durability_mode: remote_decision_journal_ack` + `ack_source: ledger_audit_outbox_commit`

---

## 1. Context（self-contained）

### 1.1 為什麼需要 v2
v1 的 round 2 review 揭露 5 個新 critical decisions + 10 partial adoptions + 7 self-consistency 矛盾。最關鍵：

1. **NormalizeCost 違反 Contract §12 hot-path 禁令**：v1 設 NormalizeCost 為 ReserveSet 前置 hot-path RPC，每筆 decision 查 Platform Pricing DB read replica。Contract §12 明示「不可在 hot path 動態查詢」。
2. **audit_outbox DDL invalid**：v1 同時宣告 `audit_outbox_id UUID PRIMARY KEY`（column-level）與 `PRIMARY KEY (recorded_month, audit_outbox_id)`（table-level），兩個 PRIMARY KEY 衝突。
3. **Postgres durability boundary 不具體**：v1 沒明示 `synchronous_commit=on` + sync replica quorum，async replica 失敗時 audit 可能遺失（重新引入 v0 所修的問題）。
4. **Provider Webhook audit path 自相矛盾**：§11 說 receiver 直接 emit CI；§19 說走 audit_outbox。直接走 CI 違反 audit-before-effect。
5. **Pricing consistency 三種說法矛盾**：tenant DB read replica vs FK to platform vs no cache。

**v2 解法**：
- Pricing freezed at contract bundle build time（per Ledger §13 三層 freeze）；contract bundle 含 pricing_version + price_snapshot_hash；sidecar 在 bundle pull 時 cache schema；ReserveSet 接收 **pre-normalized atomic amounts**（in tenant default unit）。
- Platform Pricing Authority DB 只在 contract bundle build pipeline + monthly reconciliation 用（cold path）。
- audit_outbox DDL 修正單一 PRIMARY KEY + partition-safe UNIQUE constraints。
- Postgres synchronous_commit + 至少 1 sync replica 強制。
- Provider Webhook receiver 永遠通過 Ledger gRPC；不直接 emit CI。
- 全部 v1 ⚠️ partial 加到 ✅ hardened。

---

## 2. Service Topology（v2 重點：pricing 走 build-time bundle）

```
┌────────────────────────────────────────────────────────────────────┐
│                      Customer K8s Cluster                           │
│  (sidecars cache contract_bundle including frozen pricing schema)  │
│                                                                      │
│  ┌─────────────────────────┐    ┌─────────────────────────┐       │
│  │  App Pod (replica 1)    │    │  App Pod (replica N)    │       │
│  │ ┌────────┐ ┌──────────┐ │    │ ┌────────┐ ┌──────────┐ │       │
│  │ │  App   │◄┤ Sidecar  │ │    │ │  App   │◄┤ Sidecar  │ │       │
│  │ │ (Py/   │ │ (Rust)   │ │    │ │ (Py/   │ │ (Rust)   │ │       │
│  │ └────────┘ └────┬─────┘ │    │ └────────┘ └────┬─────┘ │       │
│  └──────────────────┼──────┘    └──────────────────┼───────┘       │
│  Customer cert-manager (external issuer to Spendguard CA)          │
│  Helm root CA bundle pinned at install                             │
└─────────────────────┼──────────────────────────────┼────────────────┘
                      │ mTLS / gRPC                 │
                      │ + OCI bundle pull (multi)   │
                      │ + HTTPS catalog manifest    │
┌─────────────────────▼──────────────────────────────▼────────────────┐
│            Spendguard Control Plane (us-west-2)                     │
│                                                                      │
│  ┌─────────────────────────────────────────────────────────────┐  │
│  │   Ledger Service (Tier 1)                                     │  │
│  │   Postgres 16 SERIALIZABLE primary + 2 SYNC replicas (multi-AZ)│  │
│  │   - synchronous_commit=on; synchronous_standby_names='ANY 1'  │  │
│  │   - ledger_transactions / ledger_entries / audit_outbox       │  │
│  │   - fencing_scopes (single lease authority)                   │  │
│  │   - cached pricing schema (read-only snapshot from Platform)  │  │
│  │                                                                │  │
│  │   Outbox forwarder process (in-cluster, async to CI)          │  │
│  │                                                                │  │
│  │   gRPC: ReserveSet / Release / Commit* / Refund* / Dispute*   │  │
│  │         / Compensate / QueryBudgetState                       │  │
│  │         / ReplayAuditFromCursor / QueryDecisionOutcome        │  │
│  │         (NormalizeCost moved out of hot path; see §10)        │  │
│  └─────────────────────────────────────────────────────────────┘  │
│                                                                      │
│  ┌──────────────┐                                                  │
│  │  Provider    │                                                  │
│  │  Webhook     │                                                  │
│  │  Receiver    │   gRPC to Ledger only (audit goes via            │
│  │  (Tier 1)    │   ledger.audit_outbox; NOT direct to CI)         │
│  │              │   ───────────────────► (Ledger box above)        │
│  └──────────────┘                                                  │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────┐    │
│  │  Canonical Ingest Service (Tier 2)                         │    │
│  │  3 storage classes; best_effort_with_backpressure          │    │
│  │  Schema bundle validation                                  │    │
│  │                                                             │    │
│  │  per-decision_id sequence enforcement:                     │    │
│  │  audit.outcome strictly after audit.decision (quarantine   │    │
│  │  + 30s release / orphan_outcome)                           │    │
│  │                                                             │    │
│  │  gRPC: AppendEvents / VerifySchemaBundle                  │    │
│  │                                                             │    │
│  │  ◄─── single inbound from Ledger outbox forwarder ────     │    │
│  └──────────────────────────────────────────────────────────┘    │
│                                                                      │
│  ┌────────────────────────┐    ┌────────────────────────────────┐  │
│  │  Endpoint Catalog      │    │  Bundle Registry                │  │
│  │  (atomic update)       │    │  3 namespaces (multi-path)      │  │
│  └────────────────────────┘    └────────────────────────────────┘  │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────┐   │
│  │  Platform Pricing Authority DB (BUILD-TIME, NOT HOT PATH)  │   │
│  │  - Used by contract bundle build pipeline (cold path)      │   │
│  │  - Used by monthly reconciliation runs (cold path)         │   │
│  │  - NOT queried during decision hot path                    │   │
│  │  - Per-tenant Ledger has snapshot copy (read-only,         │   │
│  │    refreshed on bundle deploy)                             │   │
│  └────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────┘

Cold paths (build/reconciliation):
  ┌────────────────────────────────────────────────────────────┐
  │  Contract Bundle Build Pipeline                            │
  │  Reads Platform Pricing DB → freezes pricing_version +     │
  │  price_snapshot_hash into contract_bundle artifact         │
  │  → signs + publishes to Bundle Registry                    │
  └────────────────────────────────────────────────────────────┘
```

---

## 3. 🔒 D1: 5 Services + 3 POC Tiers（v2 unchanged from v1）

### 3.1 Service inventory（v2 = v1）
| Service | POC tier |
|---|---|
| Ledger (含 audit_outbox + outbox forwarder) | T1 |
| Provider Webhook Receiver | T1 |
| Canonical Ingest | T2 |
| Endpoint Catalog | T3 |
| Bundle Registry | T3 |

Platform Pricing Authority DB 不算「service」(沒有 customer-facing API)；是 internal data store for contract bundle build pipeline。

---

## 4. 🔒 D2: Audit Durability via Ledger Transactional Outbox（v2 強化 durability boundary）

### 4.1 Architecture（unchanged from v1，補 sync replica）

```
sidecar gRPC ReserveSet 至 Ledger Postgres primary:
  BEGIN TRANSACTION (SERIALIZABLE)
    1. Acquire ledger_fencing_scope row lock with epoch check (CAS)
    2. INSERT INTO ledger_transactions (operation_kind='reserve', ...)
    3. INSERT INTO ledger_entries (multiple rows for per-unit balance)
    4. INSERT INTO audit_outbox (audit_decision_event_payload, ...)
  COMMIT (with synchronous_commit=on)
       ↓
  Postgres waits for WAL flush + at least 1 sync replica ack
       ↓
  Only after sync replica ack: response returned to sidecar
       ↓
  Sidecar receives ReserveSetResponse
       ↓
  Sidecar publishes effect (Contract §6 stage 6)

→ Audit invariant satisfied: WAL durable + sync replica replicated BEFORE publish_effect
```

### 4.2 🔒 v2 NEW: Postgres durability config

```yaml
postgres_durability_phase_1:
  primary:
    region: us-west-2
    az: us-west-2a (primary)
  
  sync_replicas:
    count: 2
    az_distribution: [us-west-2b, us-west-2c]
    streaming_replication: enabled
  
  config:
    synchronous_commit: "on"
    synchronous_standby_names: "ANY 1 (replica_b, replica_c)"
    # Tx commit waits for: WAL on primary + ack from at least 1 sync replica
    # Survives single-AZ failure without audit loss
    
    wal_keep_size: "16GB"
    archive_mode: "on"
    archive_command: "wal-g wal-push %p"
    
    backup_strategy:
      base_backup: hourly
      wal_archive: continuous to S3
      pitr_retention: 35_days
  
  failure_modes_handled:
    primary_az_down: failover to sync replica (manual Phase 1; auto Phase 2)
    sync_replica_az_down: degraded mode (warns; ReserveSet may slow); 
                         if BOTH sync replicas down → ReserveSet REJECTS until restored
    primary_disk_failure: PITR restore from S3 wal archive
  
  audit_durability_proof:
    invariant: "ReserveSet ack only returned after WAL durable + ≥1 sync replica ack"
    failure_resilience: 1 AZ failure does not lose audit; 2 AZ failure causes service degrade (fail_closed) but no data loss before degrade
```

### 4.3 audit_outbox table DDL（v2 修正 per Codex C-2.1 + Ledger immutability tightening）

```sql
-- ============================================
-- audit_outbox table (v2 corrected DDL)
-- ============================================
CREATE TABLE audit_outbox (
  audit_outbox_id UUID NOT NULL,                      -- UUID v7
  audit_decision_event_id UUID NOT NULL,              -- per Trace §11.1
  decision_id UUID NOT NULL,                          -- Contract §6 idempotency key
  tenant_id UUID NOT NULL,
  
  -- Source ledger transaction (FK)
  ledger_transaction_id UUID NOT NULL 
    REFERENCES ledger_transactions(ledger_transaction_id),
  
  -- Audit event payload (CloudEvents 1.0 envelope, per Trace §7.5)
  event_type TEXT NOT NULL CHECK (event_type IN 
    ('spendguard.audit.decision', 'spendguard.audit.outcome')),
  cloudevent_payload JSONB NOT NULL,
  cloudevent_payload_signature BYTEA NOT NULL,        -- ed25519 by ledger or sidecar
  
  -- Fencing
  ledger_fencing_epoch BIGINT NOT NULL,
  workload_instance_id TEXT NOT NULL,
  
  -- Forwarding state (only fields allowed to UPDATE)
  pending_forward BOOLEAN NOT NULL DEFAULT TRUE,
  forwarded_at TIMESTAMPTZ,
  forward_attempts INT NOT NULL DEFAULT 0,
  last_forward_error TEXT,
  
  -- Time
  recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
  recorded_month DATE NOT NULL,
  
  -- Replay support (per Codex A4)
  producer_sequence BIGINT NOT NULL,
  idempotency_key TEXT NOT NULL,
  
  -- v2 fix: SINGLE primary key (partition-safe)
  PRIMARY KEY (recorded_month, audit_outbox_id)
)
PARTITION BY RANGE (recorded_month);

-- v2 fix: partition-safe UNIQUE constraints (composite with partition key)
CREATE UNIQUE INDEX audit_outbox_decision_event_uq
  ON audit_outbox (recorded_month, audit_decision_event_id);

CREATE UNIQUE INDEX audit_outbox_idempotency_uq
  ON audit_outbox (recorded_month, tenant_id, idempotency_key);

CREATE UNIQUE INDEX audit_outbox_producer_seq_uq
  ON audit_outbox (recorded_month, tenant_id, workload_instance_id, producer_sequence);

-- v2.1 patch: per-decision uniqueness（partition-safe partial unique）
-- 每個 (tenant, decision_id) 只能有 1 筆 audit.decision event
CREATE UNIQUE INDEX audit_outbox_decision_per_decision_uq
  ON audit_outbox (recorded_month, tenant_id, decision_id)
  WHERE event_type = 'spendguard.audit.decision';

-- 每個 (tenant, decision_id) 只能有 1 筆 audit.outcome event
CREATE UNIQUE INDEX audit_outbox_outcome_per_decision_uq
  ON audit_outbox (recorded_month, tenant_id, decision_id)
  WHERE event_type = 'spendguard.audit.outcome';

-- Indexes
CREATE INDEX audit_outbox_pending_forwarder_idx
  ON audit_outbox (recorded_month, pending_forward, recorded_at)
  WHERE pending_forward = TRUE;

CREATE INDEX audit_outbox_replay_cursor_idx
  ON audit_outbox (tenant_id, workload_instance_id, producer_sequence);

CREATE INDEX audit_outbox_decision_id_idx
  ON audit_outbox (tenant_id, decision_id);

-- v2 fix: tightened immutability (per Codex round 2 §3 Ledger ❌)
CREATE OR REPLACE FUNCTION reject_audit_outbox_immutable_columns()
RETURNS TRIGGER AS $$
BEGIN
  IF (OLD.audit_outbox_id, OLD.audit_decision_event_id, OLD.decision_id, 
      OLD.tenant_id, OLD.ledger_transaction_id, OLD.event_type,
      OLD.cloudevent_payload, OLD.cloudevent_payload_signature,
      OLD.ledger_fencing_epoch, OLD.workload_instance_id,
      OLD.recorded_at, OLD.recorded_month,
      OLD.producer_sequence, OLD.idempotency_key)
     IS DISTINCT FROM
     (NEW.audit_outbox_id, NEW.audit_decision_event_id, NEW.decision_id, 
      NEW.tenant_id, NEW.ledger_transaction_id, NEW.event_type,
      NEW.cloudevent_payload, NEW.cloudevent_payload_signature,
      NEW.ledger_fencing_epoch, NEW.workload_instance_id,
      NEW.recorded_at, NEW.recorded_month,
      NEW.producer_sequence, NEW.idempotency_key) THEN
    RAISE EXCEPTION 'audit_outbox immutable columns cannot be changed'
      USING ERRCODE = '42P10';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_outbox_immutability
  BEFORE UPDATE ON audit_outbox
  FOR EACH ROW EXECUTE FUNCTION reject_audit_outbox_immutable_columns();

-- v2 fix: DELETE protection
CREATE TRIGGER audit_outbox_no_delete
  BEFORE DELETE ON audit_outbox
  FOR EACH ROW EXECUTE FUNCTION reject_immutable_ledger_entry_mutation();
```

### 4.4 Audit invariant 證明（v2 強化）

**「無 audit 則無 effect」 + sync replica quorum**：
1. Sidecar 呼叫 ReserveSet RPC → Ledger Postgres primary
2. Postgres BEGIN tx；INSERT 三表（atomic intention）
3. COMMIT 觸發 WAL flush + sync replica streaming
4. **synchronous_commit=on**：等 WAL durable in primary AND ≥1 sync replica acks
5. **synchronous_standby_names='ANY 1 (replica_b, replica_c)'**：至少 1 of 2 replicas confirms
6. ack 回 sidecar；sidecar 才 publish_effect

**1-AZ failure scenario**：
- 若 us-west-2a (primary) 失效：sync replicas in 2b+2c 各保有 audit data；failover 後 audit 完整
- 若 us-west-2b 失效：primary + 2c 仍同步；服務正常
- 若 BOTH replicas 失效：primary 仍接收 writes 但 commit 等不到 sync ack → ReserveSet timeout / fail_closed；sidecar 收 fail_closed → 不 publish

### 4.5 Ledger fencing 為唯一 lease authority（v2 unchanged from v1）

ReserveSet stored procedure server-side fencing CAS：
```sql
-- 內 post_ledger_transaction stored procedure
SELECT current_epoch INTO v_current_epoch FROM fencing_scopes
  WHERE fencing_scope_id = $tenant_budget_scope
  FOR UPDATE;

IF v_current_epoch != p_caller_fencing_epoch THEN
  RAISE EXCEPTION 'FENCING_EPOCH_STALE: caller epoch % != current %', 
    p_caller_fencing_epoch, v_current_epoch
    USING ERRCODE = '40P02';  -- abort transaction
END IF;
```

### 4.6 Recovery state machine（v2 unchanged from v1 §4.5）

### 4.7 Adapter publish API idempotency（v2 unchanged）

### 4.8 🔒 D12: CI per-decision_id sequence enforcement（v2 NEW per Codex A5 hard）

Canonical Ingest 對每筆 (tenant_id, decision_id) 強制：
- 第 1 個 event 必為 `spendguard.audit.decision`
- 後續 events 可為 `spendguard.audit.outcome` 或無
- 若 `audit.outcome` 在 `audit.decision` 前抵達（e.g., out-of-order forwarder）：
  - quarantine to `awaiting_decision_event` queue
  - 等 `audit.decision` 抵達後 release（≤ 30s window）
  - 若 30s 後仍無 decision event → audit_event 標記 `orphan_outcome` + alert

```protobuf
// CI server-side check
service CanonicalIngest {
  rpc AppendEvents(AppendEventsRequest) returns (AppendEventsResponse) {
    // For each event in request:
    //   if event.type == "spendguard.audit.outcome":
    //     check existing decision event for same (tenant_id, decision_id)
    //     if missing → quarantine + return AWAITING_DECISION
    //   if event.type == "spendguard.audit.decision":
    //     dedupe by event_id
    //     unblock matching outcomes from quarantine
  }
}
```

### 4.9 Replay cursor scope（v2 unchanged）

---

## 5. 🔒 D3: Bundle Registry — 1 Service 3 Namespaces（v2 unchanged from v1）

### 5.1-5.4 unchanged from v1

---

## 6. 🔒 D4: Stage Output Persistence Path（v2 unchanged）

---

## 7. 🔒 D5: Sidecar Binary in Rust（v2 unchanged from v1）

---

## 8. 🔒 D6: Wire Protocol Baseline（v2 重大調整 per Codex 多項）

### 8.1 Transports（v2 = v1）

### 8.2 gRPC Service / Methods

#### 8.2.1 Ledger（v2 重大調整 per Codex B5, B7, B9, A8）

```protobuf
syntax = "proto3";
package spendguard.ledger.v1;
import "google/protobuf/timestamp.proto";

service Ledger {
  // === REMOVED in v2: NormalizeCost (moved to cold path; see §10) ===
  
  // Reservation lifecycle
  rpc ReserveSet(ReserveSetRequest) returns (ReserveSetResponse);
  rpc Release(ReleaseRequest) returns (ReleaseResponse);
  rpc CommitEstimated(CommitEstimatedRequest) returns (CommitEstimatedResponse);
  rpc ProviderReport(ProviderReportRequest) returns (ProviderReportResponse);
  rpc InvoiceReconcile(InvoiceReconcileRequest) returns (InvoiceReconcileResponse);
  rpc RefundCredit(RefundCreditRequest) returns (RefundCreditResponse);
  rpc DisputeAdjustment(DisputeAdjustmentRequest) returns (DisputeAdjustmentResponse);
  rpc Compensate(CompensateRequest) returns (CompensateResponse);
  rpc QueryBudgetState(QueryBudgetStateRequest) returns (QueryBudgetStateResponse);
  
  // Recovery
  rpc ReplayAuditFromCursor(ReplayAuditFromCursorRequest) 
      returns (stream ReplayAuditEvent);
  rpc QueryDecisionOutcome(QueryDecisionOutcomeRequest) 
      returns (QueryDecisionOutcomeResponse);
}

// === v2 NEW: BudgetClaim concrete proto (per Codex B9) ===
message BudgetClaim {
  string budget_id = 1;
  string unit_id = 2;                       // FK to ledger_units.unit_id
  string amount_atomic = 3;                  // string for NUMERIC(38,0) precision
  enum Direction {
    DIRECTION_UNSPECIFIED = 0;
    DIRECTION_DEBIT = 1;
    DIRECTION_CREDIT = 2;
  }
  Direction direction = 4;
  string window_instance_id = 5;             // FK to budget_window_instances
}

// === ReserveSet (v2 with concrete BudgetClaim, lock_order_token, audit context) ===
message ReserveSetRequest {
  string tenant_id = 1;
  string decision_id = 2;
  string audit_decision_event_id = 3;
  uint64 producer_sequence = 4;
  string idempotency_key = 5;
  
  // Fencing
  uint64 ledger_fencing_epoch = 6;
  string workload_instance_id = 7;
  
  // Multi-budget claims (v2: server canonicalizes lock order)
  repeated BudgetClaim claims = 8;
  
  // v2 NEW: lock_order_token (per Codex A8 hard)
  // Server derives if omitted; validates if provided
  optional string lock_order_token = 9;
  
  // Audit payload (CloudEvents 1.0 envelope, signed)
  bytes cloudevent_payload = 10;
  bytes cloudevent_payload_signature = 11;
  
  // Pricing context (v2: pricing already frozen in contract bundle)
  string pricing_version = 12;               // from contract bundle
  bytes price_snapshot_hash = 13;            // from contract bundle (immutable)
}

message ReserveSetResponse {
  oneof outcome {
    ReserveSetSuccess success = 1;
    Replay replay = 2;                       // common minimal replay
    Error error = 3;
  }
}

message ReserveSetSuccess {
  string ledger_transaction_id = 1;
  string reservation_set_id = 2;
  repeated Reservation reservations = 3;
  string audit_decision_event_id = 4;
  uint64 producer_sequence = 5;
  string lock_order_token = 6;               // v2: server-derived value
  
  // Full payload (only on first success; subject to retention policy)
  optional FullResponsePayload full = 7;
}

// === v2 NEW: Common MinimalReplayResponse (per Codex B5 hard) ===
message Replay {
  string ledger_transaction_id = 1;
  string reservation_set_id_or_op_id = 2;     // depends on op_kind
  string audit_decision_event_id = 3;
  google.protobuf.Timestamp recorded_at = 4;
  // 不含 PII / full prompts / encryption keys (per Ledger §7)
}

// All Ledger response types use Replay for replays:
//   ReleaseResponse / CommitEstimatedResponse / ProviderReportResponse /
//   InvoiceReconcileResponse / RefundCreditResponse / DisputeAdjustmentResponse /
//   CompensateResponse  
// 都遵循 oneof { Success | Replay | Error } 模式。

message Error {
  enum Code {
    CODE_UNSPECIFIED = 0;
    FENCING_EPOCH_STALE = 1;
    LOCK_ORDER_TOKEN_MISMATCH = 2;
    PRICING_VERSION_UNKNOWN = 3;
    UNIT_NORMALIZATION_REQUIRED = 4;          // v2: signal that caller should rebuild bundle
    BUDGET_EXHAUSTED = 5;
    DEADLOCK_TIMEOUT = 6;
    SYNC_REPLICA_UNAVAILABLE = 7;             // v2: synchronous_commit failure
    TENANT_DISABLED = 8;
  }
  Code code = 1;
  string message = 2;
  map<string, string> details = 3;
}
```

#### 8.2.1.1 lock_order_token derivation（v2 NEW per Codex A8 hard）

```rust
fn derive_lock_order_token(claims: &[BudgetClaim]) -> String {
    let mut sorted: Vec<&BudgetClaim> = claims.iter().collect();
    sorted.sort_by(|a, b| {
        (a.budget_id.as_str(), a.unit_id.as_str())
            .cmp(&(b.budget_id.as_str(), b.unit_id.as_str()))
    });
    let canonical = sorted.iter()
        .map(|c| format!("{}:{}", c.budget_id, c.unit_id))
        .collect::<Vec<_>>()
        .join(",");
    let hash = sha256(canonical.as_bytes());
    format!("v1:{}", hex::encode(hash))
}
```

Server validation logic in `post_ledger_transaction`:
```sql
-- if caller provided lock_order_token, verify match
IF p_caller_lock_order_token IS NOT NULL THEN
  v_derived_token := derive_lock_order_token_via_extension(p_claims);
  IF v_derived_token != p_caller_lock_order_token THEN
    RAISE EXCEPTION 'LOCK_ORDER_TOKEN_MISMATCH' 
      USING ERRCODE = '40P03';
  END IF;
END IF;

-- in either case, server uses derived token for actual locking order
v_lock_order_token := COALESCE(v_derived_token, derive_lock_order_token_via_extension(p_claims));

-- acquire row locks in lock_order_token order
PERFORM 1 FROM ledger_accounts
  WHERE ledger_account_id = ANY(p_account_ids)
  ORDER BY (budget_id::TEXT, unit_id::TEXT)
  FOR UPDATE;
```

#### 8.2.2 Canonical Ingest（v2 強化 per-decision sequence enforcement per A5 hard）

```protobuf
service CanonicalIngest {
  rpc AppendEvents(AppendEventsRequest) returns (AppendEventsResponse);
  rpc VerifySchemaBundle(VerifySchemaBundleRequest) returns (VerifySchemaBundleResponse);
  rpc QueryAuditChain(QueryAuditChainRequest) returns (stream AuditChainEvent);
}

message AppendEventsRequest {
  repeated CanonicalEvent events = 1;
  string producer_id = 2;
  bytes producer_signature = 3;              // ed25519 over events
}

message AppendEventsResponse {
  repeated EventResult results = 1;
}

message EventResult {
  string event_id = 1;
  enum Status {
    STATUS_UNSPECIFIED = 0;
    APPENDED = 1;
    DEDUPED = 2;
    AWAITING_PRECEDING_DECISION = 3;          // v2: outcome before decision
    QUARANTINED = 4;                          // schema validation failure
    ORPHAN_OUTCOME = 5;                       // 30s timeout no decision
  }
  Status status = 2;
  optional string error = 3;
}
```

CI server logic：
1. For `audit.outcome` events: lookup existing `audit.decision` for same `(tenant_id, decision_id)`
2. If missing: quarantine; status = AWAITING_PRECEDING_DECISION
3. Background reaper unblocks when matching decision arrives
4. After 30s without decision: mark ORPHAN_OUTCOME; alert ops
5. For `audit.decision`: dedupe by event_id; unblock matching outcomes

#### 8.2.3 Provider Webhook Receiver（v2 修正 per Codex C-2.4）

```
HTTP POST /v1/webhook/{provider}
  → verify signature
  → dedupe by provider event_id (Redis SETNX, 24h TTL)
  → call Ledger gRPC (ProviderReport / RefundCredit / DisputeAdjustment / etc.)
    - Audit goes via Ledger.audit_outbox (NOT direct CI emit)
  → return 200 only after Ledger commits (sync replica acked)
```

**v2 fix (per Codex C-2.4)**：webhook receiver **不**直接 emit audit canonical event 至 CI。所有 audit 必經 ledger.audit_outbox + outbox forwarder → CI 路徑。理由：
- 統一 audit durability boundary (Postgres ACID)
- 不引入 audit-before-effect 風險
- Provider events 與 sidecar events 用同一 audit chain pattern

#### 8.2.4 Endpoint Catalog（v2 SSE specifics per Codex C5 hard）

**SSE reconnection policy**:
```yaml
sse_client_config:
  initial_connect_timeout: 10s
  
  reconnect:
    strategy: jittered_exponential_backoff
    base_delay: 1s
    max_delay: 30s
    jitter_factor: 0.3                       # ±30%
    max_attempts: unlimited                   # 永遠重試
  
  heartbeat:
    expected_interval: 60s                    # server emits keepalive every 60s
    client_timeout: 30s                       # if no event for 30s → reconnect
    
  fail_closed_gate:
    rule: "last_verified_critical_version_age > 5min → fail_closed for enforcement routes"
    source: manifest pull endpoint (NOT socket state)
    rationale: SSE 是 hint；correctness 看 pull-based manifest verification age
```

#### 8.2.5 Bundle Registry（v2 unchanged）

### 8.3 Protobuf canonical event encoding（v2 unchanged from v1）

### 8.4 mTLS 信任鏈（v2 unchanged from v1 §8.4）

---

## 9. Deployment Topology（v2 with Postgres durability config）

### 9.1 客戶側（v2 unchanged）

### 9.2 Spendguard control plane（v2 with sync replicas）

| Service | 部署 | v2 vs v1 |
|---|---|---|
| Ledger | Postgres 16 SERIALIZABLE primary (us-west-2a) + 2 sync replicas (us-west-2b/2c); synchronous_commit=on; synchronous_standby_names='ANY 1' | **v2: sync replicas required** |
| Provider Webhook Receiver | Deployment + autoscale; Redis | unchanged |
| Canonical Ingest | Deployment + autoscale; S3 + Postgres-as-blob | unchanged + per-decision_id sequence enforcement |
| Endpoint Catalog | Deployment + S3 + signed manifest | unchanged |
| Bundle Registry | OCI multi-path | unchanged |
| Platform Pricing Authority DB | **Cold-path only**; queried by build pipeline + reconciliation | **v2: not queried in hot path** |

### 9.3 Per-tenant Ledger ops 限制（v2 unchanged from v1 §9.3）

### 9.4 🔒 D10: Platform Pricing Authority DB（v2 重新設計 per Codex C-2.3, C-2.5, B7 hard）

**v1 錯誤模型**：每筆 ReserveSet 前置 NormalizeCost RPC 查 Platform Pricing DB read replica。違反 Contract §12 hot-path 禁令。

**v2 正確模型**：

```
═══════════════════════════════════════════════════════════════════
COLD PATH (build time, not in decision hot path)
═══════════════════════════════════════════════════════════════════

Platform Pricing Authority DB (central, immutable additive)
  - pricing_versions (FOCUS v1.2 + provider snapshots)
  - fx_rate_versions (currency)
  - unit_conversion_versions (token / credit / etc.)
  ↓
Contract Bundle Build Pipeline
  - Reads Platform Pricing DB at build time
  - Generates contract_bundle artifact containing:
    * pricing_version (TEXT)
    * price_snapshot_hash (BYTEA)         ← immutable hash
    * fx_rate_version (TEXT)
    * unit_conversion_version (TEXT)
    * normalized_pricing_schema (JSONB)   ← embedded into bundle
  - Signs bundle with ed25519
  - Publishes to Bundle Registry
       ↓
Tenant Ledger DB (replica of contract_bundle.normalized_pricing_schema)
  - On contract bundle deployment, ledger DB updates internal cache table
  - This update happens at deployment time (cold path)
  - Not synchronized at decision time

═══════════════════════════════════════════════════════════════════
HOT PATH (decision time)
═══════════════════════════════════════════════════════════════════

Sidecar receives decision boundary trigger
  ↓
Sidecar reads contract_bundle from local cache (per Contract §14 prewarm)
  ↓
Sidecar normalizes amounts using cached pricing schema (in-process, < 1ms)
  ↓
Sidecar issues ReserveSet with:
  - BudgetClaim {amount_atomic in canonical unit (USD micros)}
  - pricing_version = bundle.pricing_version
  - price_snapshot_hash = bundle.price_snapshot_hash
  ↓
Ledger validates pricing_version + price_snapshot_hash against ledger DB cache
  - If mismatch (bundle deployment lag): return PRICING_VERSION_UNKNOWN
  - Sidecar refreshes bundle and retries
  ↓
Ledger writes ledger_entries with normalized atomic amounts (already in canonical unit)
```

**Why this 不違反 Contract §12**:
- Contract §12 forbids "cross-unit comparison via dynamic query in hot path"
- v2: cross-unit conversion happens at **bundle build time** (cold path)
- Sidecar 在 hot path 只讀 cached schema（in-process）；不查任何 DB
- Ledger 只 validate version IDs match cached snapshot；不做 conversion

**Bundle deployment lag 處理**：
- Contract bundle 從 Bundle Registry pull 至 sidecar 需時（normal 數秒）
- Sidecar refresh 與 ledger DB cache update 之間可能有 lag
- 若 sidecar 用較新 pricing_version；ledger 用較舊 cache → ledger return PRICING_VERSION_UNKNOWN
- Sidecar 重試後 ledger DB cache 更新；之後成功
- **不會引入 audit gap**（ReserveSet 失敗 = 無 reservation = 無 publish = 無 audit-before-effect 違反）

**v2 cross-DB consistency model**:

```yaml
platform_pricing_db:
  authority: central control plane
  immutability: enforced (per Ledger §6.1 trigger pattern)
  schema_change: additive only (new versions); existing rows never modified
  replication: NOT replicated to tenant ledger DBs at runtime
  
  consumers:
    - contract_bundle_build_pipeline: reads at build time
    - reconciliation_runs: reads monthly
    - operator_dashboard: read for visibility

tenant_ledger_db:
  pricing_cache:
    table: ledger.pricing_snapshots
    populated_when: contract_bundle deployed (event-driven, not polling)
    contents: 
      - {pricing_version, price_snapshot_hash, normalized_pricing_schema_json}
    immutability: same trigger as audit_outbox
    deletion: never (kept for 7yr audit replay)
  
  reference_in_ledger_entries:
    pricing_version: TEXT (FK soft-link to ledger.pricing_snapshots)
    price_snapshot_hash: BYTEA (frozen at insert)
  
  consistency_at_decision_time:
    sidecar_cached_bundle_version → ReserveSet pricing_version
    ledger validates against ledger.pricing_snapshots cache
    if cache miss: return PRICING_VERSION_UNKNOWN (sidecar refresh + retry)
```

---

## 10. POC Tier deliverables（v2 更新）

### 10.1 Tier 1: Ledger
- v2.1 DDL（Ledger §22）+ audit_outbox v2 fixed DDL（§4.3）
- post_ledger_transaction stored procedure v2（含 audit_outbox 寫入路徑、lock_order_token 驗證、fencing CAS）
- **Sync replication setup**: 2 sync replicas + synchronous_commit=on
- Outbox forwarder process（Rust，in-cluster）
- **lock_order_token derivation extension**（Postgres function 或 sidecar 一致實作）
- ReplayAuditFromCursor + QueryDecisionOutcome
- 11 chaos tests（Ledger §21.3 + 5 audit_outbox + 1 sync replica failure）
- Replay golden corpus 18 scenarios（含 v2 NEW: dj_collapse + outbox forwarder + sync_replica_quorum_loss）

### 10.2 Tier 1: Provider Webhook Receiver
- HTTPS POST endpoint per provider
- Signature verification per provider
- Redis idempotency dedupe (24h TTL)
- gRPC client to Ledger (audit goes via ledger.audit_outbox)
- 4 chaos tests

### 10.3 Tier 2: Canonical Ingest（v2 加 per-decision sequence enforcement）
- gRPC AppendEvents + sequence enforcement logic
- best_effort_with_backpressure
- 三 storage classes
- Schema bundle validation
- 5 golden corpus scenarios

### 10.4 Tier 3: Endpoint Catalog（v2 加 jittered SSE reconnect）
- Versioned immutable catalog + signed manifest
- SSE with jittered exp backoff base 1s max 30s
- Heartbeat 60s server / 30s client timeout
- Fail-closed gate based on `last_verified_critical_version_age`
- 5 chaos tests

### 10.5 Tier 3: Bundle Registry（v2 unchanged）

### 10.6 Cold Path: Contract Bundle Build Pipeline（v2 NEW per §9.4）
- GitHub Actions / GitLab CI pipeline
- Reads Platform Pricing DB at build time
- Embeds pricing_version + price_snapshot_hash + fx_rate_version + unit_conversion_version + normalized_pricing_schema into contract_bundle.json
- ed25519 signs bundle
- Publishes to Bundle Registry (3 namespace contract_bundle/)
- Triggers tenant ledger DB cache update (event-driven via control-plane internal API)

---

## 11. Provider Webhook Receiver（v2 修正 per Codex C-2.4）

### 11.1 服務職責（v2）

| Job | 細節 |
|---|---|
| Receive provider webhooks | HTTPS POST per provider |
| Signature verification | per-provider key |
| Idempotency dedupe | Redis SETNX, 24h TTL |
| Map to Ledger ops | provider event → Ledger gRPC |
| **Audit goes via Ledger** | **不直接 emit CI**（per Codex C-2.4） |

### 11.2 Provider event mapping（v2 unchanged from v1）

### 11.3 Audit path uniformity（v2 emphasis）

所有 audit canonical events（無論來源是 sidecar 還是 webhook receiver）路徑：

```
source → Ledger gRPC (sync replica acked) → ledger.audit_outbox row → 
        outbox forwarder → CI (per-decision_id sequence enforced) → 
        immutable_audit_log (7yr retention)
```

**Webhook receiver 不直接 POST CI**。理由：
- 統一 durability boundary (Postgres ACID + sync replica)
- 不引入 audit-before-effect 風險（v0 中 DJ 的問題）
- Provider events 與 sidecar events 同 audit chain pattern；replay 與 reconciliation 一致

---

## 12. Cross-cutting Concerns（v2 更新）

### 12.1 mTLS / Trust Bootstrap（v2 unchanged from v1）

### 12.2 Encryption at Rest（unchanged）

### 12.3 Region Affinity（unchanged）

### 12.4 Throughput Assumption + Benchmark Methodology（v2 unchanged from v1）

### 12.5 Observability — smoke metrics（v2 unchanged）

---

## 13. Cross-Service Chaos Tests（v2 更新 with sync replica failure tests）

### 13.1 audit_outbox & sync replica chaos（v2 expanded）

```yaml
audit_durability_chaos_tests:
  - dj_collapse_correctness (v1 from §13.1; unchanged)
  
  - sync_replica_quorum_loss_during_reserveset
    description: BOTH sync replicas 同時 down 時 ReserveSet
    expected: synchronous_commit timeout → ReserveSet returns SYNC_REPLICA_UNAVAILABLE
              → sidecar fail_closed → no publish_effect
              → audit invariant preserved (no orphan effects)
  
  - sync_replica_async_promotion
    description: Phase 1 single primary; manually promote 1 sync replica → primary
    expected: short downtime; in-flight ReserveSet either commit or rollback;
              promoted replica has all committed audit_outbox rows;
              no audit gap
  
  - audit_outbox_forwarder_lag_30s
    description: 故意暫停 forwarder 30s
    expected: audit_outbox.pending_forward 累積；CI 收到 events；recovery 後追上
  
  - audit_outbox_partition_rotation (v1 unchanged)
  
  - dj_ack_then_pod_failure_before_publish_effect (v1 unchanged)
  
  - fencing_epoch_stale_during_reserve (v1 unchanged)
  
  - postgres_primary_disk_failure_pitr_recovery
    description: primary disk 故障；從 S3 WAL archive PITR
    expected: data 無 loss（synchronous_commit 已將 WAL 推至 sync replica + S3）；
              promotion + replay 後恢復；audit invariant 維持
```

### 13.2 CI per-decision_id sequence chaos（v2 NEW）

```yaml
ci_sequence_enforcement_chaos:
  - audit_outcome_arrives_before_decision
    description: outbox forwarder 因 partition lag 而 outcome 先 forward
    expected: CI 把 outcome 放 quarantine (AWAITING_PRECEDING_DECISION)；
              decision 抵達後 release；30s 內處理完
  
  - orphan_outcome_30s_timeout
    description: decision event 永久遺失（極端情境）
    expected: outcome 標記 ORPHAN_OUTCOME；alert；不阻擋其他 events
```

### 13.3 lock_order_token chaos（v2 NEW per A8 hard）

```yaml
lock_order_chaos:
  - caller_provides_wrong_lock_order_token
    expected: ledger return LOCK_ORDER_TOKEN_MISMATCH; no DB writes
  
  - caller_omits_lock_order_token
    expected: server derives + uses; response includes derived token
  
  - parallel_reservesets_with_overlapping_budgets
    description: 並發 ReserveSet 覆蓋同一 budget 子集
    expected: lexicographic lock ordering；無 deadlock；其中一個成功一個 retry
```

### 13.4 Provider Webhook chaos（v1 unchanged）

### 13.5 v0/v1 retained chaos tests
（保留 v1 §13.4 列表）

---

## 14. Open Questions / 隱含假設（v2 更新）

### 14.1 已解決
- ✅ Per-tenant vs shared Postgres → ceiling at 5
- ✅ DJ storage backend → 取消（合進 ledger）
- ✅ Forwarder reliability → at-least-once + dedupe by event_id
- ✅ Endpoint Catalog SSE → invalidation hint，pull 是 correctness
- ✅ Postgres durability boundary → synchronous_commit + sync replica quorum
- ✅ Pricing hot-path → moved to bundle build time
- ✅ Provider Webhook audit path → ledger.audit_outbox only
- ✅ audit_outbox DDL → single PRIMARY KEY, partition-safe UNIQUE
- ✅ lock_order_token derivation → sha256(sorted_tuple)
- ✅ CI per-decision_id sequence → enforced
- ✅ Privacy split → MinimalReplayResponse common type
- ✅ BudgetClaim → concrete proto

### 14.2 Round 3 待 verify
- v2 sync_standby_names 配置是否需 `FIRST 1` vs `ANY 1` (前者更嚴格但 latency 較高)
- Bundle deployment lag 期間 ledger DB cache 更新策略 (event-driven push vs polling)
- 跨 partition forwarder 處理是否正確處理 month boundary

### 14.3 待後續 RFC
- Adapter implementation RFC
- Phase 2 self-hosted control plane

---

## 15. Considered & Rejected Alternatives（v2 更新）

### 15.1 v0/v1 中已 rejected（保留）

### 15.2 v2 NEW rejected
- **NormalizeCost 為 hot-path RPC** (v1)：rejected per Contract §12 + Codex B7 hard。改為 build-time + reconciliation only.
- **Provider Webhook Receiver 直接 emit CI** (v1)：rejected per Codex C-2.4。改為走 ledger.audit_outbox 統一路徑。
- **audit_outbox dual PRIMARY KEY** (v1)：rejected DDL invalid。修為單一 (recorded_month, audit_outbox_id) 主鍵 + 多個 partition-safe UNIQUE indexes。
- **Postgres async replication only** (v1 implicit)：rejected per Codex A1, B2 hard。改為 synchronous_commit + sync replica quorum.

---

## 16. 🔒 Day 0 / Day 1 / Day 7 Onboarding Milestones（v2 unchanged from v1 §16）

---

## 17. 🔒 Topology Sequence Diagrams（v2 column names corrected per self-consistency #7）

### 17.1 Happy path (v2 corrected)

```
Sidecar      Ledger        Sync Replicas     Adapter        Outbox Forwarder    CI
  |             |                |                |                |                |
  |--ReserveSet>|                |                |                |                |
  |             |--BEGIN tx----->|                |                |                |
  |             |--INSERT 3 tables                |                |                |
  |             |--COMMIT---wal->|                |                |                |
  |             |                |--ack(at least 1)|                |                |
  |<--success---|<---------------|                |                |                |
  |--apply(effect_hash)--------->|                |                |                |
  |             |                |                |--executes------|                |
  |<--applied---|----------------|                |                |                |
  |             |                |                |                |--read pending->|
  |             |                |                |                |--POST batch--->|
  |             |                |                |                |<--ack----------|
  |             |                |                |                |--UPDATE        |
  |             |                |                |                |   pending_forward=false,
  |             |                |                |                |   forwarded_at=now()
```

(v2 fix: 用 actual column names `pending_forward`, `forwarded_at` instead of v1's incorrect `forwarded=true`)

### 17.2-17.4 (v1 內容保留 + column 名稱修正)

---

## 18. Adapter Implementation Note (POC)（unchanged from v1）

---

## 19. Reconciliation Flow（v2 unchanged from v1 §19）

---

## 20. Path to Lock — Codex Round 3 Minimal Verification

### 20.1 Round 3 任務範圍（terse）

| Verify | 目的 |
|---|---|
| 22 round-2 patches actually addressed in v2 text | 不只 promise，要看 patch 真的入 |
| No new §0.2 critical decisions in v2 itself | Confirm no 致命錯誤 missed |
| Cross-spec invariants 仍 hold | 特別 Contract §12 (hot path) + Sidecar §6.2 (durability matrix) + Ledger §6.1 (immutability) |
| Self-consistency: counts, column names, contradictions all resolved | check 逐項 round-2 self-consistency list |

### 20.2 採納流程
1. Codex round 3 minimal verify → 若無新 critical → lock
2. Lock 後 v2 升 `stage2-poc-topology-spec-v1alpha1.md`
3. v0 + v1 + round 1/2/3 review files 全部刪除（per 規則 4）
4. Lock 文件含 §22 adoption history 紀錄 3 輪 reviews

---

## 21. Lock 後的下一步（v2 unchanged）

---

## 22. Adoption History

| Round | Findings | Adoption | 主要產出 |
|---|---|---|---|
| **Round 1** (v0 → v1) | 37 findings: 12 must + 13 should + 12 clarification/partial reject | 100% | v1：DJ collapse 進 ledger transactional outbox；Provider Webhook Receiver 新增；Helm trust bootstrap (Day-0)；explicit Helm injection (no mutating webhook)；Day 0/1/7 onboarding milestones；多 services 從 5 重組；Rust sidecar production-debug gates；OCI multi-path bundle distribution |
| **Round 2** (v1 → v2) | 22 patches: 5 new critical (audit_outbox DDL invalid / Postgres durability boundary missing / Pricing DB consistency contradictions / Provider Webhook audit path inconsistent / NormalizeCost violates Contract §12 hot-path) + 10 round-1 partial hard-fix + 7 self-consistency | 100% | v2：audit_outbox DDL 單 PRIMARY KEY；synchronous_commit + sync replica quorum；pricing build-time freeze (NormalizeCost 移出 hot path)；Provider Webhook 統一走 ledger.audit_outbox；CI per-decision_id sequence enforcement；BudgetClaim + MinimalReplayResponse + lock_order_token concrete proto；audit_outbox immutability tightening |
| **Round 3** (v2 → v1alpha1 LOCKED) | 2 minor patches: audit_outbox per-decision partition-safe partial unique + topology diagram arrow disambiguation | 100% | v2.1 in-place patches；no §0.2 critical remaining；cross-spec invariants verified across 4 LOCKED specs (Contract §12 hot-path / Sidecar §6.2 durability / Ledger §6.1 immutability / Contract §6 stage ordering / Ledger §13 pricing freeze) → **LOCK** |

**Total**: 3 rounds, 61 findings/patches, 100% adoption rate across all rounds. No round 4 needed.

---

## 23. Lock 結論

🎉 **Stage 2 POC topology spec v1alpha1 LOCKED — 進入 reference implementation 並行展開階段。**

✅ **Phase 1 control plane 5 個 specs 全部 LOCKED**：
```
✅ Stage 1A — Contract DSL spec v1alpha1
✅ Stage 1B — Trace Canonical Schema spec v1alpha1
✅ Stage 1C — Sidecar Architecture spec v1alpha1
✅ Stage 1D — Ledger Storage spec v1alpha1
✅ Stage 2 — POC Topology spec v1alpha1（本文件）
```

下一步：
1. **Reference implementation 並行展開**（per §21）
2. **每個 service 進開發**：寫 code 前先 Codex review 對應 spec 段落（per 工作原則）；寫完後 Codex challenge mode + chaos tests
3. **First customer design partner onboarding 預備**：Day 0 / Day 1 / Day 7 rehearsal in staging（per §16）

---

*Document version: stage2-poc-topology-spec-v1alpha1 (LOCKED) | Generated: 2026-05-07 | Lock judgment basis: Codex round 3 minimal verification + v2.1 in-place patches | 100% adoption: 37 (round 1) + 22 (round 2) + 2 (round 3) | Companion: 4 Stage 1 LOCKED specs + Stage 1 Integration Audit*
