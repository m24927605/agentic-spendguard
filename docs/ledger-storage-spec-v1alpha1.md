# Ledger Storage Specification — v1alpha1 (LOCKED)

> 🔒 **Status: LOCKED implementation spec**  
> **Lock date**: 2026-05-07  
> **Lock judgment basis**: Codex round-3 minimal verification — 「v2 已收斂。沒有看到新的 §7 級 high-irreversibility gap。建議 lock with minor v2.1 DDL patch，不需要 v3 / round-4。」  
> **Adoption history**: Round 1/2/3 採納率 100%/100%/100%（3 輪零實質反駁）  
> **Companions**:
> - `agent-runtime-spend-guardrails-complete.md` (v1.3 strategy)
> - `contract-dsl-spec-v1alpha1.md` (LOCKED)
> - `trace-schema-spec-v1alpha1.md` (LOCKED)
> - `sidecar-architecture-spec-v1alpha1.md` (LOCKED)
> 
> **Compatibility policy**: alpha — append-only ledger entries / per-unit balance invariants / partition by recorded_month / sequence allocator immutable / replay-critical dimensions immutable / 7-year audit retention

---

## 0. Lock Status & Prerequisites

### 0.1 範圍

完整的 Ledger Storage Model 設計，可進入 reference implementation POC + first customer design partner。**Phase 1 control plane 所有 4 個 specs 全部 LOCKED**。

### 0.2 POC 前置條件（Codex round-3 規定）

進入 reference implementation POC 前下列必須到位：

1. **套用 v2.1 DDL** patches（§22 詳列）
2. **實作 `post_ledger_transaction()` server-derived insert path**（不信任 caller-supplied denormalized 欄位）
3. **Chaos tests**：per-unit balance / immutability / idempotency pending-retry / multi-shard ReserveSet 2PC
4. **Recorded partition rotation** + shard sequence allocator 自動化
5. **7-year replay golden corpus**：reserve / release / commit / provider_report / invoice / refund / dispute / backfill / RTBF / region failover

### 0.3 GA 前置條件

POC 通過後，GA 路徑前下列必達成：

1. NUMERIC(38,0) vs BIGINT benchmark 在 hot path 性能符合 Contract §14（reserve 20ms p99）
2. Multi-currency reconciliation 端到端驗證
3. 7-year audit replay 驗證（含 RTBF tombstone + CMK rotation + schema migration）
4. Cross-region active-passive failover 驗證
5. Resharding（shard generation）operational drill

### 0.4 何時可能需要 v2 spec

只有以下情況才開啟 v2 spec 修正：
- POC 揭示 architectural 重大缺陷
- 發現新的 §7 級 high-irreversibility gap
- Contract DSL spec / Trace schema spec / Sidecar spec 升級時 ledger 對應 break

正常情況下 v1alpha1 → v1beta1 → v1（GA）為 additive 演進，**無 breaking changes**。

---

## 1. Context（self-contained）

### 1.1 產品

**Agent Runtime Spend Guardrails** — 在 agent step / tool call / reasoning spend 邊界做 budget decision、policy enforcement、approval、rollback、audit 的 runtime 安全層。

### 1.2 已 lock 的 specs（依賴）

- `contract-dsl-spec-v1alpha1.md`：reservationSet、decision_transaction、commit state machine
- `trace-schema-spec-v1alpha1.md`：cost computation timing、audit integration、required Tier 4
- `sidecar-architecture-spec-v1alpha1.md`：region affinity、durability matrix、capability flags、fencing token

### 1.3 v1alpha1 核心哲學

> **Ledger 是 append-only 真相**；mutations 通過 compensating entries，不是 UPDATE。  
> **Truth 與 projection 分離**：ledger_entries 是真相；spending_window_projections / commits 是 derived。  
> **Per-unit balance 是金融不變式**：USD micros 與 token count 不可加總。  
> **Recorded order is truth**：partition by recorded_month；effective_at 僅作 query semantics。  
> **Immutability 必須由 DB 強制**：trigger + role + procedure 三層。  
> **Idempotency 是 Stripe replay**：minimal token 永久 + encrypted full payload 可 RTBF 刪。  
> **Pricing freeze 三層**：pricing_version + price_snapshot_hash + fx_rate_version + unit_conversion_version。

---

## 2. Append-Only Double-Entry Truth Model

```
┌─────────────────────────────────────────────────────────────┐
│                  Append-Only Truth                          │
│                                                              │
│  ledger_units    ledger_accounts    ledger_transactions     │
│       │                │                    │                │
│       └────────────────┴────────────────────┘                │
│                        │                                     │
│                        ▼                                     │
│                ledger_entries                                │
│            (append-only; per-unit balanced)                 │
│                        │                                     │
│                        │ derived projections                 │
│                        ▼                                     │
│  spending_window_projections    reservations    commits     │
│      (rebuildable from ledger; non-truth)                   │
└─────────────────────────────────────────────────────────────┘
```

### 2.1 不變式

```yaml
invariants:
  - balanced_per_(transaction, unit_id) (debits == credits per unit)
  - immutable_after_insert (ledger_entries 不可 UPDATE / DELETE)
  - corrections_via_compensating_entries (反向 entry 取消原 entry)
  - effective_at_distinct_from_recorded_at (帳期語意 vs 入帳時點)
  - recorded_month_partition_truth (partition by arrival, not effective)
```

### 2.2 Account Types

```yaml
account_kinds:
  available_budget: 客戶可用預算
  reserved_hold: 已 reserve 暫扣
  committed_spend: 已 commit 實扣
  debt: 超支記錄
  adjustment: 修正
  refund_credit: provider 退款回沖
  dispute_adjustment: dispute 暫扣
```

---

## 3. Per-Unit Balancing

```yaml
amount_representation:
  type: NUMERIC(38,0)                                 # not BIGINT
  semantics: atomic units (per unit's scale)
  
  unit_definition:
    table: ledger_units
    schema:
      unit_id: UUID v7
      tenant_id: UUID
      unit_kind: enum [monetary, token, credit, non_monetary]
      currency: CHAR(3) (when monetary)
      unit_name: TEXT (when token / credit / non_monetary)
      scale: INT (USD=6, JPY=0, tokens=0)
      rounding_mode: enum [half_even, half_up, truncate, banker]

balance_invariant:
  per_(transaction, unit_id):
    rule: |
      For each (ledger_transaction_id, unit_id):
      sum(debit amounts) == sum(credit amounts)
  
  enforcement:
    primary: stored procedure post_ledger_transaction() statement-level check
    secondary: deferred constraint trigger as backstop
  
  cross_unit_comparison_at_query_time: forbidden
  conversion_only_at_explicit_normalization_step: required
```

---

## 4. Recorded-Order Partitioning

```yaml
partitioning:
  truth_partition_key: (recorded_month, tenant_id, ledger_shard_id)
  rationale: |
    recorded_month 是 immutable arrival 順序；
    late invoice 寫到當前熱 partition；
    effective_month 只作 query index 用
  
  query_index_for_billing:
    secondary_index: (tenant_id, budget_id, effective_month, effective_at)
    purpose: billing / reconciliation 查詢
  
  ordering:
    primary: (ledger_shard_id, ledger_sequence)
    sequence_allocator: per-shard via ledger_sequence_allocators table
    rationale: monotonic per shard；不依賴 timestamp
  
  partition_strategy:
    method: native_postgres_declarative_partitioning
    rotation: monthly range partition + optional hash subpartition by shard
    not_pg_repack_dependency: confirmed
    
    hot_storage:
      retention: 90_days
      backend: postgres
    
    cold_archive:
      retention: 90_days_to_7_years
      backend: object_storage_with_signed_immutability
      query_path: lazy_load_via_archival_query_service
  
  resharding:
    shard_id_no_reuse: required
    new_generation: ledger_shards.shard_generation 自增
    new_sequence: 不繼承舊 shard sequence
    projection_cursor_tracks_active_shard_set: required
```

---

## 5. Complete Schema

### 5.1 Foundation tables

```sql
-- ============================================
-- ledger_units (replay-critical immutable identity)
-- ============================================
CREATE TABLE ledger_units (
  unit_id UUID PRIMARY KEY,
  tenant_id UUID NOT NULL,
  unit_kind TEXT NOT NULL CHECK (unit_kind IN 
    ('monetary', 'token', 'credit', 'non_monetary')),
  currency CHAR(3),
  unit_name TEXT,
  scale INT NOT NULL,
  rounding_mode TEXT NOT NULL CHECK (rounding_mode IN
    ('half_even', 'half_up', 'truncate', 'banker')),
  display_format TEXT,
  effective_from TIMESTAMPTZ NOT NULL DEFAULT now(),
  effective_until TIMESTAMPTZ,
  UNIQUE (tenant_id, unit_kind, currency, unit_name, scale)
);

-- ledger_shards: shard identity & generation
CREATE TABLE ledger_shards (
  ledger_shard_id SMALLINT PRIMARY KEY,
  shard_generation BIGINT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('active', 'draining', 'retired')),
  parent_shard_id SMALLINT,                            -- when split from another
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  retired_at TIMESTAMPTZ
);

-- ledger_sequence_allocators: per-shard monotonic counter
CREATE TABLE ledger_sequence_allocators (
  ledger_shard_id SMALLINT PRIMARY KEY 
    REFERENCES ledger_shards(ledger_shard_id),
  last_sequence BIGINT NOT NULL DEFAULT 0
);

-- budget_window_instances (replay-critical immutable identity)
CREATE TABLE budget_window_instances (
  window_instance_id UUID PRIMARY KEY,
  tenant_id UUID NOT NULL,
  budget_id UUID NOT NULL,
  window_type TEXT NOT NULL CHECK (window_type IN 
    ('calendar_day', 'rolling', 'calendar_month', 'billing_cycle')),
  timezone TEXT,
  tzdb_version TEXT NOT NULL,
  billing_anchor_rule_version TEXT,
  boundary_start TIMESTAMPTZ,
  boundary_end TIMESTAMPTZ,
  rolling_bucket_granularity INTERVAL,
  computed_from_snapshot_at TIMESTAMPTZ NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_budget_window_instances_lookup 
  ON budget_window_instances (tenant_id, budget_id, boundary_start);

-- ledger_accounts
CREATE TABLE ledger_accounts (
  ledger_account_id UUID PRIMARY KEY,
  tenant_id UUID NOT NULL,
  budget_id UUID NOT NULL,
  window_instance_id UUID NOT NULL 
    REFERENCES budget_window_instances(window_instance_id),
  account_kind TEXT NOT NULL CHECK (account_kind IN
    ('available_budget', 'reserved_hold', 'committed_spend', 'debt', 'adjustment',
     'refund_credit', 'dispute_adjustment')),
  unit_id UUID NOT NULL REFERENCES ledger_units(unit_id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, budget_id, window_instance_id, account_kind, unit_id)
);

CREATE INDEX idx_ledger_accounts_lookup 
  ON ledger_accounts (tenant_id, budget_id, window_instance_id);
```

### 5.2 ledger_transactions

```sql
CREATE TABLE ledger_transactions (
  ledger_transaction_id UUID PRIMARY KEY,
  tenant_id UUID NOT NULL,
  
  operation_kind TEXT NOT NULL CHECK (operation_kind IN
    ('reserve', 'release', 
     'commit_estimated', 'provider_report', 'invoice_reconcile',
     'overrun_debt', 'adjustment', 
     'refund_credit', 'dispute_adjustment',
     'compensating')),
  
  -- Posting state machine
  posting_state TEXT NOT NULL DEFAULT 'pending'
    CHECK (posting_state IN ('pending', 'posted', 'voided')),
  posted_at TIMESTAMPTZ,
  
  -- Idempotency replay (privacy split)
  idempotency_key TEXT NOT NULL,
  request_hash BYTEA NOT NULL,
  minimal_replay_response JSONB NOT NULL DEFAULT '{}',
  response_payload_ref TEXT,
  response_payload_hash BYTEA,
  replay_expires_at TIMESTAMPTZ,
  
  -- CMK schema interface (Phase 1 reserved)
  encryption_key_id TEXT,
  encryption_context JSONB,
  
  -- Trace anchors
  trace_event_id UUID,
  audit_decision_event_id UUID,                       -- canonical audit anchor (per Trace §11.1)
  
  -- Time semantics
  effective_at TIMESTAMPTZ NOT NULL,
  recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
  
  -- Lock ordering
  lock_order_token UUID NOT NULL,
  
  -- Fencing
  fencing_scope_id UUID REFERENCES fencing_scopes(fencing_scope_id),
  fencing_epoch_at_post BIGINT,
  
  -- Provider dispute (per round-3 §12 #13)
  provider_dispute_id TEXT,
  case_state TEXT,
  resolved_at TIMESTAMPTZ,
  
  UNIQUE (tenant_id, operation_kind, idempotency_key)
);

CREATE INDEX idx_ledger_transactions_audit 
  ON ledger_transactions (audit_decision_event_id);

CREATE INDEX idx_ledger_transactions_pending 
  ON ledger_transactions (tenant_id, recorded_at)
  WHERE posting_state = 'pending';

CREATE INDEX idx_ledger_transactions_dispute
  ON ledger_transactions (provider_dispute_id, case_state)
  WHERE provider_dispute_id IS NOT NULL;
```

### 5.3 ledger_entries（partition-safe DDL — v2.1 patch）

```sql
CREATE TABLE ledger_entries (
  ledger_entry_id UUID NOT NULL,
  ledger_transaction_id UUID NOT NULL 
    REFERENCES ledger_transactions(ledger_transaction_id),
  ledger_account_id UUID NOT NULL 
    REFERENCES ledger_accounts(ledger_account_id),
  
  -- Per-unit balancing
  tenant_id UUID NOT NULL,
  budget_id UUID NOT NULL,
  window_instance_id UUID,
  unit_id UUID NOT NULL REFERENCES ledger_units(unit_id),
  
  -- Amount with proper precision
  direction TEXT NOT NULL CHECK (direction IN ('debit', 'credit')),
  amount_atomic NUMERIC(38, 0) NOT NULL CHECK (amount_atomic >= 0),
  
  -- Pricing freeze
  pricing_version TEXT NOT NULL,
  price_snapshot_hash BYTEA NOT NULL,
  fx_rate_version TEXT,
  unit_conversion_version TEXT,
  
  -- Cross-references
  reservation_id UUID,
  commit_event_kind TEXT,
  invoice_line_item_ref TEXT,
  
  -- Sequence ordering
  ledger_shard_id SMALLINT NOT NULL 
    REFERENCES ledger_shards(ledger_shard_id),
  ledger_sequence BIGINT NOT NULL,
  
  -- Time semantics
  effective_at TIMESTAMPTZ NOT NULL,
  effective_month DATE NOT NULL,
  recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
  recorded_month DATE NOT NULL,
  
  -- Trace anchor
  ingest_position JSONB,
  created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
  
  -- v2.1 patch: partition-safe PK includes partition key
  PRIMARY KEY (recorded_month, ledger_entry_id)
)
PARTITION BY RANGE (recorded_month);

-- v2.1 patch: partition-safe unique index includes partition key
CREATE UNIQUE INDEX ledger_entries_partition_sequence_uq
  ON ledger_entries (recorded_month, ledger_shard_id, ledger_sequence);

-- Effective query index
CREATE INDEX ledger_entries_effective_query_idx
  ON ledger_entries (tenant_id, budget_id, effective_month, effective_at);

-- Account/transaction indexes
CREATE INDEX idx_ledger_entries_account 
  ON ledger_entries (ledger_account_id, effective_at);

CREATE INDEX idx_ledger_entries_transaction 
  ON ledger_entries (ledger_transaction_id);

CREATE INDEX idx_ledger_entries_reservation 
  ON ledger_entries (reservation_id) 
  WHERE reservation_id IS NOT NULL;

-- Auto-create monthly partitions (via pg_partman or migration runner)
CREATE TABLE ledger_entries_2026_05 
  PARTITION OF ledger_entries 
  FOR VALUES FROM ('2026-05-01') TO ('2026-06-01');
-- ... rotation ongoing
```

### 5.4 fencing_scopes + history

```sql
CREATE TABLE fencing_scopes (
  fencing_scope_id UUID PRIMARY KEY,
  scope_type TEXT NOT NULL CHECK (scope_type IN 
    ('reservation', 'budget_window')),
  tenant_id UUID NOT NULL,
  budget_id UUID NOT NULL,
  reservation_id UUID,
  window_instance_id UUID,
  current_epoch BIGINT NOT NULL DEFAULT 0,
  active_owner_instance_id TEXT,
  ttl_expires_at TIMESTAMPTZ,
  epoch_source_authority TEXT NOT NULL DEFAULT 'ledger_lease',
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  
  CHECK (
    (scope_type = 'reservation' AND reservation_id IS NOT NULL AND window_instance_id IS NULL) OR
    (scope_type = 'budget_window' AND window_instance_id IS NOT NULL AND reservation_id IS NULL)
  )
);

CREATE UNIQUE INDEX fencing_scope_reservation_uq
  ON fencing_scopes (tenant_id, budget_id, reservation_id)
  WHERE scope_type = 'reservation';

CREATE UNIQUE INDEX fencing_scope_budget_window_uq
  ON fencing_scopes (tenant_id, budget_id, window_instance_id)
  WHERE scope_type = 'budget_window';

CREATE INDEX idx_fencing_active_lookup
  ON fencing_scopes (scope_type, tenant_id, budget_id, ttl_expires_at);

-- v2.1 patch: fencing history projection
CREATE TABLE fencing_scope_events (
  fencing_event_id UUID PRIMARY KEY,
  fencing_scope_id UUID NOT NULL REFERENCES fencing_scopes(fencing_scope_id),
  old_epoch BIGINT NOT NULL,
  new_epoch BIGINT NOT NULL,
  owner_instance_id TEXT NOT NULL,
  action TEXT NOT NULL CHECK (action IN 
    ('acquire', 'renew', 'revoke', 'promote', 'recover')),
  audit_event_id UUID NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE INDEX idx_fencing_scope_events_history
  ON fencing_scope_events (fencing_scope_id, created_at);
```

### 5.5 Projections & support tables

```sql
-- spending_window_projections
CREATE TABLE spending_window_projections (
  tenant_id UUID NOT NULL,
  budget_id UUID NOT NULL,
  window_instance_id UUID NOT NULL 
    REFERENCES budget_window_instances(window_instance_id),
  unit_id UUID NOT NULL REFERENCES ledger_units(unit_id),
  
  available_atomic NUMERIC(38,0) NOT NULL,
  reserved_hold_atomic NUMERIC(38,0) NOT NULL DEFAULT 0,
  committed_spend_atomic NUMERIC(38,0) NOT NULL DEFAULT 0,
  debt_atomic NUMERIC(38,0) NOT NULL DEFAULT 0,
  adjustment_atomic NUMERIC(38,0) NOT NULL DEFAULT 0,
  refund_credit_atomic NUMERIC(38,0) NOT NULL DEFAULT 0,
  
  reservation_count BIGINT NOT NULL DEFAULT 0,
  commit_count BIGINT NOT NULL DEFAULT 0,
  
  -- Sequence-based cursor
  projection_lag_shard_id SMALLINT,
  projection_lag_sequence BIGINT,
  
  derived_from_append_only_ledger BOOLEAN NOT NULL DEFAULT true,
  rebuildable_from_entries BOOLEAN NOT NULL DEFAULT true,
  
  version BIGINT NOT NULL DEFAULT 0,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  
  PRIMARY KEY (tenant_id, budget_id, window_instance_id, unit_id)
);

CREATE INDEX idx_projection_cursor 
  ON spending_window_projections (projection_lag_shard_id, projection_lag_sequence);

-- reservations & commits projections (latest state only)
CREATE TABLE reservations (
  reservation_id UUID PRIMARY KEY,
  tenant_id UUID NOT NULL,
  budget_id UUID NOT NULL,
  window_instance_id UUID NOT NULL,
  current_state TEXT NOT NULL CHECK (current_state IN 
    ('reserved', 'committed', 'released', 'overrun_debt')),
  trace_run_id UUID,
  trace_step_id UUID,
  trace_llm_call_id UUID,
  source_ledger_transaction_id UUID NOT NULL 
    REFERENCES ledger_transactions(ledger_transaction_id),
  ttl_expires_at TIMESTAMPTZ NOT NULL,
  idempotency_key TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX reservations_idempotency_scoped
  ON reservations (tenant_id, budget_id, idempotency_key);

CREATE INDEX idx_reservations_active 
  ON reservations (tenant_id, budget_id, window_instance_id)
  WHERE current_state = 'reserved';

CREATE INDEX idx_reservations_ttl 
  ON reservations (ttl_expires_at)
  WHERE current_state = 'reserved';

CREATE TABLE commits (
  commit_id UUID PRIMARY KEY,
  reservation_id UUID NOT NULL,
  tenant_id UUID NOT NULL,
  budget_id UUID NOT NULL,
  unit_id UUID NOT NULL REFERENCES ledger_units(unit_id),
  latest_state TEXT NOT NULL CHECK (latest_state IN 
    ('unknown', 'estimated', 'provider_reported', 'invoice_reconciled')),
  estimated_amount_atomic NUMERIC(38,0),
  provider_reported_amount_atomic NUMERIC(38,0),
  invoice_reconciled_amount_atomic NUMERIC(38,0),
  delta_to_reserved_atomic NUMERIC(38,0),
  pricing_version TEXT NOT NULL,
  price_snapshot_hash BYTEA NOT NULL,
  estimated_at TIMESTAMPTZ,
  provider_reported_at TIMESTAMPTZ,
  invoice_reconciled_at TIMESTAMPTZ,
  latest_projection_only BOOLEAN NOT NULL DEFAULT true,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_commits_reservation ON commits (reservation_id);
CREATE INDEX idx_commits_state ON commits (latest_state, updated_at);
```

### 5.6 Pricing / FX / unit conversion versions

```sql
CREATE TABLE pricing_versions (
  pricing_version TEXT PRIMARY KEY,
  price_snapshot_hash BYTEA NOT NULL,
  fx_rate_version TEXT NOT NULL,
  unit_conversion_version TEXT NOT NULL,
  effective_from TIMESTAMPTZ NOT NULL,
  effective_until TIMESTAMPTZ,
  schema JSONB NOT NULL,
  signature BYTEA NOT NULL,
  signing_key_id TEXT NOT NULL,
  encryption_key_id TEXT,
  encryption_context JSONB,
  payload_ref TEXT,
  imported_from TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE fx_rate_versions (
  fx_rate_version TEXT PRIMARY KEY,
  effective_from TIMESTAMPTZ NOT NULL,
  effective_until TIMESTAMPTZ,
  rates JSONB NOT NULL,
  signature BYTEA NOT NULL,
  immutable_after_publish BOOLEAN NOT NULL DEFAULT true
);

CREATE TABLE unit_conversion_versions (
  unit_conversion_version TEXT PRIMARY KEY,
  conversions JSONB NOT NULL,
  signature BYTEA NOT NULL
);
```

---

## 6. Immutability Enforcement（v2.1 三層強制 + replay-critical immutability）

### 6.1 Database Triggers

```sql
-- ledger_entries: complete immutability (no UPDATE / DELETE)
CREATE OR REPLACE FUNCTION reject_immutable_ledger_entry_mutation()
RETURNS TRIGGER AS $$
BEGIN
  RAISE EXCEPTION 'ledger_entries are immutable; use compensating entry'
    USING ERRCODE = '42P10';
  RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER ledger_entries_no_update_delete
  BEFORE UPDATE OR DELETE ON ledger_entries
  FOR EACH ROW EXECUTE FUNCTION reject_immutable_ledger_entry_mutation();

-- v2.1 patch: ledger_units identity columns immutable
CREATE OR REPLACE FUNCTION reject_replay_identity_mutation()
RETURNS TRIGGER AS $$
BEGIN
  RAISE EXCEPTION 'replay-critical identity columns are immutable'
    USING ERRCODE = '42P10';
  RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER ledger_units_no_identity_update
  BEFORE UPDATE OF unit_kind, currency, unit_name, scale, rounding_mode
  ON ledger_units
  FOR EACH ROW EXECUTE FUNCTION reject_replay_identity_mutation();

-- v2.1 patch: budget_window_instances completely immutable
CREATE OR REPLACE FUNCTION reject_immutable_reference_mutation()
RETURNS TRIGGER AS $$
BEGIN
  RAISE EXCEPTION 'replay-critical reference is immutable'
    USING ERRCODE = '42P10';
  RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER budget_window_instances_no_update_delete
  BEFORE UPDATE OR DELETE ON budget_window_instances
  FOR EACH ROW EXECUTE FUNCTION reject_immutable_reference_mutation();

-- v2.1 patch: pricing_versions completely immutable
CREATE TRIGGER pricing_versions_no_update_delete
  BEFORE UPDATE OR DELETE ON pricing_versions
  FOR EACH ROW EXECUTE FUNCTION reject_immutable_reference_mutation();

-- v2.1 patch: fx_rate_versions and unit_conversion_versions also immutable
CREATE TRIGGER fx_rate_versions_no_update_delete
  BEFORE UPDATE OR DELETE ON fx_rate_versions
  FOR EACH ROW EXECUTE FUNCTION reject_immutable_reference_mutation();

CREATE TRIGGER unit_conversion_versions_no_update_delete
  BEFORE UPDATE OR DELETE ON unit_conversion_versions
  FOR EACH ROW EXECUTE FUNCTION reject_immutable_reference_mutation();
```

### 6.2 Restricted Writer Role

```sql
CREATE ROLE ledger_posting_role NOINHERIT;
GRANT EXECUTE ON FUNCTION post_ledger_transaction(...) TO ledger_posting_role;

REVOKE INSERT, UPDATE, DELETE ON ledger_entries FROM PUBLIC;
REVOKE INSERT, UPDATE, DELETE ON ledger_entries FROM ledger_posting_role;

CREATE ROLE ledger_application_role;
GRANT EXECUTE ON FUNCTION post_ledger_transaction(...) TO ledger_application_role;

CREATE ROLE ledger_reader_role;
GRANT SELECT ON ledger_entries, ledger_transactions, ledger_accounts 
  TO ledger_reader_role;
```

### 6.3 Stored Procedure Posting Path（v2.1 server-side derivation）

```sql
CREATE OR REPLACE FUNCTION post_ledger_transaction(
  p_transaction_id UUID,
  p_entries JSONB                                       -- caller supplies ledger_account_id + direction + amount only
) RETURNS UUID AS $$
DECLARE
  v_unit_balances JSONB;
BEGIN
  -- v2.1 patch: server-side derive tenant/budget/window/unit from ledger_account_id
  -- Don't trust caller's denormalized fields
  
  -- Step 1: Validate transaction is pending
  PERFORM 1 FROM ledger_transactions
    WHERE ledger_transaction_id = p_transaction_id
      AND posting_state = 'pending'
    FOR UPDATE;
  
  IF NOT FOUND THEN
    RAISE EXCEPTION 'Transaction not pending or not found';
  END IF;
  
  -- Step 2: Insert entries with server-derived fields
  INSERT INTO ledger_entries (
    ledger_entry_id, ledger_transaction_id, ledger_account_id,
    tenant_id, budget_id, window_instance_id, unit_id,         -- ← server-derived
    direction, amount_atomic,
    pricing_version, price_snapshot_hash, fx_rate_version, unit_conversion_version,
    reservation_id, commit_event_kind,
    ledger_shard_id, ledger_sequence,                          -- ← server-allocated
    effective_at, effective_month, recorded_at, recorded_month
  )
  SELECT 
    (entry->>'ledger_entry_id')::UUID,
    p_transaction_id,
    la.ledger_account_id,
    la.tenant_id,                                              -- ← from ledger_accounts
    la.budget_id,
    la.window_instance_id,
    la.unit_id,
    entry->>'direction',
    (entry->>'amount_atomic')::NUMERIC(38,0),
    entry->>'pricing_version',
    decode(entry->>'price_snapshot_hash', 'hex'),
    entry->>'fx_rate_version',
    entry->>'unit_conversion_version',
    (entry->>'reservation_id')::UUID,
    entry->>'commit_event_kind',
    (entry->>'ledger_shard_id')::SMALLINT,
    -- v2.1: sequence allocator
    (UPDATE ledger_sequence_allocators 
       SET last_sequence = last_sequence + 1 
       WHERE ledger_shard_id = (entry->>'ledger_shard_id')::SMALLINT
       RETURNING last_sequence),
    (entry->>'effective_at')::TIMESTAMPTZ,
    date_trunc('month', (entry->>'effective_at')::TIMESTAMPTZ)::DATE,
    clock_timestamp(),
    date_trunc('month', clock_timestamp())::DATE
  FROM jsonb_array_elements(p_entries) AS entry
  JOIN ledger_accounts la 
    ON la.ledger_account_id = (entry->>'ledger_account_id')::UUID;
  
  -- Step 3: Verify per-(tx, unit) balance (statement-level check)
  SELECT jsonb_object_agg(
    unit_id::TEXT, 
    SUM(CASE WHEN direction = 'debit' THEN amount_atomic ELSE -amount_atomic END)
  ) INTO v_unit_balances
  FROM ledger_entries
  WHERE ledger_transaction_id = p_transaction_id
  GROUP BY unit_id;
  
  IF EXISTS (
    SELECT 1 FROM jsonb_each(v_unit_balances) 
    WHERE value::TEXT::NUMERIC != 0
  ) THEN
    RAISE EXCEPTION 'Per-unit balance violation: %', v_unit_balances;
  END IF;
  
  -- Step 4: Mark transaction as posted
  UPDATE ledger_transactions
  SET posting_state = 'posted',
      posted_at = clock_timestamp()
  WHERE ledger_transaction_id = p_transaction_id;
  
  RETURN p_transaction_id;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;
```

### 6.4 Constraint Trigger as Backstop

```sql
CREATE CONSTRAINT TRIGGER ledger_transaction_balanced_per_unit
  AFTER INSERT OR UPDATE ON ledger_entries
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION assert_ledger_transaction_balanced_per_unit();
```

---

## 7. Idempotency Privacy Split

```yaml
idempotency_replay:
  scope: (tenant_id, operation_kind, idempotency_key)
  
  retention_split:
    minimal_replay_response:
      retention: 7_years (audit aligned)
      storage: ledger_transactions.minimal_replay_response (JSONB)
      schema: versioned per operation_kind
      contains:
        - reservation_id (if applicable)
        - ledger_transaction_id
        - status code
        - timestamp
      not_contains: PII / full prompts / encryption keys
    
    full_response_payload:
      retention: tenant_policy
      default: 24_hours
      maximum_phase_1: 30_days
      storage: encrypted blob via response_payload_ref
      encryption: at_rest with tenant DEK
      rtbf_deletable: required
  
  retry_semantics:
    on_first_request:
      execute + store_minimal + store_full_encrypted
    on_retry_within_replay_window:
      verify_request_hash: required
      if_match: return full payload
      if_mismatch: reject_with_idempotency_key_reused_error
    on_retry_after_full_expires:
      return: minimal_replay_response only
      caller_must_query_resource_state: required
```

---

## 8. Window Semantics with `snapshot_at`

```yaml
window_query:
  rolling_query:
    snapshot_at: required parameter (no now())
    rationale: query with now() = unreproducible audit replay
    
    example: |
      "spend in last 1 hour as of $snapshot_at" =
        sum(ledger_entries 
          WHERE effective_at > $snapshot_at - interval '1 hour'
            AND effective_at <= $snapshot_at
            AND ledger_account_id = X)
  
  bucket_strategy_for_rolling:
    purpose: bound query cost
    bucket_granularity: contract_defined from platform allowed set
    platform_default: 1_minute
    platform_allowed: [10_seconds, 1_minute, 5_minutes, 1_hour]
    materialized_buckets: pre-computed sums per bucket
  
  budget_window_instance:
    immutable_after_creation: required (per §6.1 trigger)
    contains_freeze_fields: tzdb_version, billing_anchor_rule_version, computed_from_snapshot_at
```

---

## 9. Fencing Scopes

```yaml
fencing:
  scope_options:
    reservation:
      identity: reservation_id
      use_case: per-reservation lifecycle
    budget_window:
      identity: (budget_id, window_instance_id)
      use_case: budget-level coordination
  
  authority: ledger_lease (per Sidecar §9)
  monotonic_epoch: BIGINT per scope
  CAS_on_recover: required
  
  history_table: fencing_scope_events (v2.1)
  audit_trail: all acquire/renew/revoke/promote/recover events recorded
  
  cross_region_failover:
    protocol: signed promotion + old writer revoked
    fail_closed_until_revocation_acked: required
    sla:
      revocation_ack_required_for_promotion: yes
      sidecar_critical_revocation_window: 5_minutes (per Sidecar §7)
```

---

## 10. Refund / Dispute Operation Kinds

```yaml
operation_kinds_extended:
  refund_credit:
    description: provider issues credit for prior charge
    direction: credits adjustment account
    impact: increases available_budget
  
  dispute_adjustment:
    description: customer disputes charge; pending resolution
    direction: temporary debit on adjustment until resolved
    fields:
      provider_dispute_id: TEXT (e.g., Stripe dispute_id)
      case_state: enum [open, under_review, resolved_in_favor, resolved_against, withdrawn]
      resolved_at: TIMESTAMPTZ
    
    resolution_flow:
      resolved_in_favor: emit refund_credit operation
      resolved_against: emit compensating reverse
      withdrawn: emit compensating reverse
```

---

## 11. CMK Schema Interface（Phase 1 reserve, Phase 2 active）

```yaml
cmk_schema_interface:
  phase_1:
    columns_present_in_schema:
      - encryption_key_id (TEXT)
      - encryption_context (JSONB)
      - payload_ref (TEXT)
    columns_populated: optional (NULL allowed)
    encryption: platform_managed_key
    
    migration_tool_dry_run_manifest:
      schema_reserved: yes
      command: spendguard.migrate.cmk-dry-run
      output: re-encrypt manifest preview
  
  phase_2_activation:
    customer_managed_key:
      key_id: customer's KMS / GCP CMEK / Azure KeyVault
      encryption_context: tenant_id + tenant_metadata
      payload_ref: encrypted blob in customer-controlled bucket
    
    migration_from_pmk_to_cmk:
      method: re_encrypt_with_dual_key_period
      duration: 12_months
      tool: spendguard.migrate.cmk
      audit: required canonical events
```

---

## 12. STONITH Cross-Region Region Fencing

```yaml
cross_region_fencing:
  promotion_protocol:
    step_1_revoke_old_region:
      endpoint_catalog: mark old region endpoints REVOKED + signed
      sidecar_behavior: see immediately fail-closed
    
    step_2_network_acl_or_security_group:
      action: cut old region ledger writer's network access
    
    step_3_signing_key_revocation:
      action: revoke old region producer signing key
    
    step_4_promote_new_region:
      action: new region acquires next fencing epoch
    
    step_5_audit_event:
      type: spendguard.region_failover_promoted
      signed: required
  
  sla:
    revocation_ack_required_before_promotion: yes
    hard_enforcement_during_failover: fail_closed
    sidecar_critical_revocation_max_stale: 5_minutes (per Sidecar §7)
  
  not_dns_alone: confirmed
  minimum_3_layer_revocation: required
```

---

## 13. Multi-Currency Conversion Freezing

```yaml
multi_currency_handling:
  three_layer_freeze:
    pricing_version: 哪個 pricing schema
    fx_rate_version: 哪份 FX rates
    unit_conversion_version: 哪份 token/credit 換算表
    price_snapshot_hash: 整體不可變雜湊
  
  cross_unit_comparison_at_query_time: forbidden
  conversion_only_at_explicit_normalization_step: required
  
  tables:
    fx_rate_versions: immutable_after_publish (per §6.1 trigger)
    unit_conversion_versions: immutable
    pricing_versions: immutable
```

---

## 14. Partitioning / Archival / Schema Migration

```yaml
partitioning:
  ledger_entries:
    partition_strategy: monthly_range_by_recorded_month
    optional_subpartition: hash_by_ledger_shard_id
    not_84_partitions_per_tenant: confirmed (use shared monthly partitions across tenants)
  
  hot_storage:
    retention: 90_days
  
  warm_storage:
    retention: 90_days_to_2_years
  
  cold_archive:
    retention: 2_years_to_7_years
    backend: WORM_bucket_with_immutability + Sigstore_signature_provenance
  
  schema_migration:
    additive_columns: online via default
    type_changes: forbidden after lock
    column_rename: forbidden (use alias view)
    column_removal: tombstone for 1 year
    
    breaking_migration:
      requires: dual_write_period_with_decoder_compat
      duration: minimum 12 months
    
    online_migration_tool: spendguard.migrate (not pg_repack-bound)
    
    ddl_audit_event:
      type: spendguard.ledger.schema_migration
      captures: before_schema_hash + after_schema_hash + migration_runner_id
      runner_only_path: required
```

---

## 15. Encryption at Rest

```yaml
encryption:
  per_tenant_key_hierarchy:
    master_key (KMS-managed, platform-owned)
      ↓ derives
    tenant_data_encryption_key (per tenant)
      ↓ encrypts
    ledger rows + projections + audit
  
  rotation:
    master_key: yearly
    tenant_dek: yearly with dual-key period
    old_data_remains_decryptable: required (envelope re-encryption)
  
  customer_managed_key_option: phase_2+ (schema reserved Phase 1)
  
  field_level_encryption:
    sensitive_fields:
      - request_hash
      - response_payload_ref content
      - invoice_line_item_ref
    encryption_at_application_layer: required
    plus_tablespace_encryption: defense_in_depth
  
  rtbf_compatibility:
    aligned_with_trace_§10.2_storage_classes:
      ledger_entries: canonical_raw_log (hash-only ok)
      ledger_transactions.full_response: profile_payload_blob (RTBF deletable)
```

---

## 16. DR / PITR / Reconciliation Source Order

```yaml
disaster_recovery:
  rpo: minutes
  rto: minutes_to_hours
  audit_retention_loss_unacceptable: required
  
  pitr:
    granularity: seconds
    retention: 35_days_minimum_(7_years_for_audit)
    method: postgres_wal_archive_to_object_storage
  
  reconciliation_source_order:
    truth_priority:
      1: ledger_entries (canonical, append-only)
      2: provider_invoice (external truth)
      3: spending_window_projections (derived, rebuildable)
    
    when_disagreement:
      provider_invoice_vs_ledger_entries:
        small_tolerance: auto-compensate via compensating entry
        over_tolerance: approval_workflow + compensating entry
        all_via_compensating_entry: required (no UPDATE)
      
      projection_vs_ledger_entries:
        action: rebuild_projection from ledger
        not_overwrite_ledger: required
  
  ledger_self_audit_during_dr:
    track: who triggered DR + recovered + compensating entries
    immutable_audit_log: required
```

---

## 17. Ledger Self-Audit

```yaml
ledger_self_audit:
  audited_events:
    - pricing_version_changes
    - reconciliation_runs
    - adjustment_entries
    - fencing_lease_acquisitions
    - schema_migrations (DDL via runner only)
    - dr_invocations
  
  storage:
    aligned_with_trace_§10.2: immutable_audit_log
    canonical_event_envelope: cloudevents (per Trace §7.5)
    event_types:
      - spendguard.ledger.pricing_version_change
      - spendguard.ledger.reconciliation_run
      - spendguard.ledger.adjustment_entry
      - spendguard.ledger.fencing_lease_acquired
      - spendguard.ledger.schema_migration
      - spendguard.ledger.dr_invoked
  
  signing: per Trace §13 producer signing
```

---

## 18. Capability Flags

```yaml
capability_flags:
  strong_global:
    region_scope: multi_region_linearizable
    hard_enforcement_allowed: true
    read_staleness_bound: 0_ms
    fencing_authority: globally_serialized_lease
    phase: phase_3_plus
  
  single_writer_per_budget:
    region_scope: single_region
    hard_enforcement_allowed: true
    read_staleness_bound: replica_lag_ms (typical < 100ms)
    fencing_authority: per_budget_lease
    phase: phase_1_through_phase_2
  
  eventual:
    region_scope: any
    hard_enforcement_allowed: false
    read_staleness_bound: unbounded
    fencing_authority: not_applicable
    use_case: observability_only

phase_1_constraint:
  ledger_advertises: single_writer_per_budget (only)
  cannot_advertise_strong_global: confirmed
```

---

## 19. Phase 1 Postgres Constraints

```yaml
phase_1:
  isolation_level: SERIALIZABLE
  alternative: explicit row locks with documented invariants
  capability_advertised: single_writer_per_budget (same region)
  cannot_advertise: strong_global
  
  read_replicas:
    use_for: query / dashboard / analytics
    not_for: reservation / commit / decision snapshot
```

---

## 20. Companion Spec Integration

### 20.1 Contract §3.2 ReserveSet

ReserveSet 跨 budget 必須 per-unit balanced；同 ledger_transaction_id；lock_order_token 強制 lexicographic order。Phase 2 cross-shard 2PC（不 punt）。

### 20.2 Contract §6 audit_decision Anchoring

`ledger_transactions.audit_decision_event_id` 為 audit replay anchor（對齊 Trace §11.1）；posting_state 確保 audit 在 publish 前 posted。

### 20.3 Trace §10.4 三 amount

每 unit 獨立 entries：`commit_estimated` / `provider_report` / `invoice_reconcile`。`finalized_amount` derived at query time。

### 20.4 Sidecar §6.2 durability matrix

ledger_transactions.posting_state = `posted` 提供 audit_decision durable anchor。

### 20.5 Sidecar §9 fencing

`fencing_scopes` 表是 ledger lease authority；CAS via UPDATE WHERE current_epoch = X；`fencing_scope_events` 記歷史。

### 20.6 Sidecar §12.5 capability flags

Phase 1 advertise `single_writer_per_budget` 同 region only。

---

## 21. Reference Implementation POC Plan

### 21.1 必達成的實作項

1. 套用 v2.1 DDL（§22）
2. 實作 `post_ledger_transaction()` server-derived insert path
3. Recorded partition rotation + shard sequence allocator 自動化
4. fencing_scope_events history projection
5. Per-unit balance / immutability / idempotency pending-retry chaos tests

### 21.2 7-Year Replay Golden Corpus

```yaml
golden_corpus:
  scenarios:
    - reserve_full_lifecycle
    - release_via_ttl
    - release_via_explicit
    - commit_estimated_only
    - commit_estimated_then_provider_reported
    - commit_estimated_then_invoice_reconciled
    - overrun_debt_with_compensating
    - refund_credit
    - dispute_adjustment_with_resolution_in_favor
    - dispute_adjustment_with_resolution_against
    - backfill_late_invoice_with_recorded_partition
    - rtbf_with_lookup_edge_deletion
    - region_failover_with_signed_promotion
    - multi_unit_balance_validation
    - cross_shard_reserveSet_2pc
```

### 21.3 Chaos Tests

```yaml
chaos_tests:
  - per_unit_balance_violation_attempt (must reject)
  - immutability_attempt_via_direct_update (must reject)
  - immutability_attempt_via_role_bypass (must reject)
  - idempotency_pending_retry_with_partial_post (verify replay)
  - sequence_allocator_race_condition (verify monotonic)
  - cross_region_fencing_split_brain (verify fail_closed)
  - replay_critical_dimension_update_attempt (verify trigger fires)
  - large_transaction_batch_limit (verify enforced)
  - rolling_window_query_without_snapshot_at (verify rejected)
```

---

## 22. v2.1 Patch Detail（記錄用）

進入 spec lock 前納入的 minor clarifications：

| Patch | 位置 | 內容 |
|---|---|---|
| Partitioned table PK | §5.3 | `PRIMARY KEY (recorded_month, ledger_entry_id)`（PostgreSQL partition-safe） |
| Partition-safe unique index | §5.3 | `UNIQUE INDEX (recorded_month, ledger_shard_id, ledger_sequence)` |
| ledger_shards 表 | §5.1 | shard identity + generation + status + parent |
| ledger_sequence_allocators 表 | §5.1 | per-shard monotonic counter（取代 nextval sequence） |
| ledger_units identity immutable | §6.1 | trigger blocks UPDATE OF unit_kind, currency, unit_name, scale, rounding_mode |
| budget_window_instances immutable | §6.1 | trigger blocks UPDATE / DELETE |
| pricing_versions immutable | §6.1 | trigger blocks UPDATE / DELETE |
| fx_rate / unit_conversion immutable | §6.1 | same triggers |
| Stored procedure server-derive | §6.3 | tenant_id / budget_id / window_instance_id / unit_id 從 ledger_account_id JOIN，不信 caller |
| fencing_scope_events 表 | §5.4 | history projection of acquire/renew/revoke/promote/recover |
| provider_dispute_id + case_state + resolved_at | §5.2 | dispute lifecycle tracking |

---

## 23. Companion Compatibility Policy（alpha）

| 承諾 | 細節 |
|---|---|
| **Append-only ledger entries** | 永不 UPDATE / DELETE；corrections via compensating entries |
| **Per-unit balance invariants** | 跨 schema 升級保留 |
| **Partition by recorded_month** | 不可改為 effective_month |
| **Sequence allocator immutable** | shard_id 不重用；新 shard 用新 generation |
| **Replay-critical dimensions immutable** | ledger_units identity / budget_window_instances / pricing_versions 不可改 |
| **7-year audit retention** | 由 records 自身控制；schema migration 不影響歷史 |
| **Idempotency replay window** | minimal_replay 7 年；full payload tenant policy |
| **Postgres SERIALIZABLE Phase 1** | 升 Phase 2+ 仍保留 strong consistency 承諾 |
| **Capability flag stability** | Phase 1 advertise single_writer_per_budget；不會降至 eventual |
| **Alpha SLA** | Postgres 99.95% availability; backup RPO minutes |

---

## 24. Adoption History

| Round | Codex 反饋 | 採納率 | 主要產出 |
|---|---|---|---|
| Round 1 | 致命架構錯誤（mutable 1:1 vs append-only double-entry）+ 7 partial reject | 100% | Append-only ledger truth；scoped idempotency；pricing freeze 三層；capability flags scope；fencing scope；§8-§13 升 6 項 high-irreversibility |
| Round 2 | 2 個新 high-irreversibility gap（per-unit balancing + recorded-order partitioning）+ 5 implementation-grade | 100% | NUMERIC(38,0) + ledger_units；partition by recorded_month + sequence ordering；immutability 三層；idempotency privacy split；fencing_scopes DDL 修正；budget_window_instances DDL；refund/dispute kinds；CMK schema interface |
| Round 3 | Minimal verification | 100% | v2.1 patch（partition-safe PK；ledger_shards + sequence_allocators；replay-critical immutability triggers；server-side derivation；fencing_scope_events history）→ **LOCK** |

---

## 25. Lock 後的下一步

🎉 **Phase 1 control plane 4 個 specs 全部 LOCKED**：

```
✅ Stage 1A — Contract DSL spec v1alpha1
✅ Stage 1B — Trace Canonical Schema spec v1alpha1
✅ Stage 1C — Sidecar Architecture spec v1alpha1
✅ Stage 1D — Ledger Storage spec v1alpha1
```

下一步：

1. **Reference impl POC 全力開工**（4 specs 平行）
2. **Stage 1A-1D 整合 review**（跨 spec 一致性最終 audit）
3. **First customer design partner onboarding**（K8s SaaS-managed 模式）
4. **Phase 1 service implementations**：
   - Bundle registry service
   - Endpoint catalog service
   - Decision journal service
   - Ledger service (this spec)
   - Canonical ingest service
5. **POC chaos test suite**（4 個 spec 的 chaos tests 整合執行）

---

*Document version: ledger-storage-spec-v1alpha1 (LOCKED) | Generated: 2026-05-07 | Adoption: 100% across 3 Codex rounds | POC prerequisites listed §0.2 | GA prerequisites listed §0.3 | Companion: agent-runtime-spend-guardrails-complete.md (v1.3) + contract-dsl-spec-v1alpha1.md (LOCKED) + trace-schema-spec-v1alpha1.md (LOCKED) + sidecar-architecture-spec-v1alpha1.md (LOCKED)*
