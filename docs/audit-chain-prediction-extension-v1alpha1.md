# Audit Chain Prediction Extension Specification — v1alpha1 (DRAFT)

> 📝 **Status: DRAFT** (writing in design phase on branch `design/predictor-upgrade`)
> **DRAFT → LOCKED criteria**: locks together with the predictor-upgrade spec set per `predictor-architecture-spec-v1alpha1.md` §0.2; additionally requires `verify-chain` regression suite green on (a) pre-existing demo audit rows with NULL prediction fields and (b) freshly-written rows with all 18 fields populated, across all 8+ demo modes.
> **Companion specs (this set)**: `predictor-architecture-spec-v1alpha1.md` (umbrella), `tokenizer-service-spec-v1alpha1.md`, `output-predictor-service-spec-v1alpha1.md`, `output-predictor-plugin-contract-v1alpha1.md`, `run-cost-projector-spec-v1alpha1.md`, `cold-start-baseline-spec-v1alpha1.md`, `contract-dsl-spec-v1alpha2.md`, `stats-aggregator-spec-v1alpha1.md`, `calibration-report-spec-v1alpha1.md`.
> **Pre-existing LOCKED dependencies that this spec extends**:
> - `services/ledger/migrations/0009_audit_outbox.sql` — base schema
> - `services/ledger/migrations/0011_immutability_triggers.sql` — `reject_audit_outbox_immutable_columns` function (this spec extends the OLD/NEW column comparison list)
> - `proto/spendguard/common/v1/common.proto` — CloudEvent envelope (this spec adds extension attribute fields at tag 300+)
> - `services/signing/src/lib.rs` — Signer trait + canonical_bytes contract (no behavioral change)
> - `services/canonical_ingest/src/verifier.rs` — `canonical_bytes_proto` + `canonical_bytes_json` (no behavioral change; proto3 additive evolution carries new fields automatically)
> - `trace-schema-spec-v1alpha1.md` §10.2 — storage classes (new columns land in `immutable_audit_log` class, no new class needed)
> - `contract-dsl-spec-v1alpha1.md` §13 — DecisionAuditEvent schema (this spec adds the prediction subset)
>
> **Compatibility policy**: alpha — strictly additive over existing `audit_outbox` schema and CloudEvent proto. Old verifiers continue to verify new rows (proto3 default values for unknown fields). New verifiers continue to verify old rows (all new columns nullable; NULL = field not populated = signed as proto3 default).

---

## §0. Lock status & prerequisites

### 0.1 範圍

本 spec 是**整套 predictor upgrade 對 audit chain 的全部影響的單一 source of truth**。涵蓋：

1. `audit_outbox` schema 新增 18 個 columns（11 decision-side prediction + 3 run-level + 4 commit-side；§2.4 reviewer-flagged 把 `cold_start_layer_used` 從 metadata 升為 first-class column → decision-side 從 10 → 11，總計 17 → 18）
2. `CloudEvent` proto 同步新增 18 個 extension attribute fields（mirror，為了讓 producer_signature 自然覆蓋新欄位 —— 詳見 §3 與 §6）
3. `reject_audit_outbox_immutable_columns` PostgreSQL trigger 函式更新（防止新欄位被 forwarder UPDATE path 違反 audit immutability —— 此風險已在 HANDOFF Step 4 discrepancy #4 識別）
4. Canonical bytes derivation 不需改 code（proto3 additive evolution 自動覆蓋新 field）
5. verify_cloudevent + verify-chain CLI 的 backward / forward 相容性論證
6. Storage class、outbox_forwarder、canonical_ingest replication 全部不變

不在本 spec 範圍：

- 新欄位的 *值* 如何計算（推給 tokenizer / output_predictor / run_cost_projector / stats_aggregator 各自的 spec）
- Calibration report 如何讀新欄位（推給 `calibration-report-spec-v1alpha1.md`）
- 新欄位的 decision code consumers（推給 `contract-dsl-spec-v1alpha2.md` 與 `run-cost-projector-spec-v1alpha1.md`）

### 0.2 DRAFT → LOCKED criteria

進入 LOCKED 之前下列 6 項必達成：

1. SLICE_01 migration（`0044_audit_outbox_prediction_columns.sql` 暫名）run 通過 fresh Postgres + 既有 demo Postgres，無 error
2. `reject_audit_outbox_immutable_columns` trigger update 在 18 個新欄位上 UPDATE attempt 全部 raise `42P10`
3. `verify-chain` regression 在 (a) NULL 新欄位的既有 rows + (b) 全填新欄位的新 rows 全綠
4. 8+ 個 demo modes（`make demo-up DEMO_MODE={proxy,decision,deny,approval,ttl_sweep,agent_real,...}`）audit chain 仍 verify 通過
5. CloudEvent proto bump 通過 prost / tonic codegen 無 break
6. canonical_ingest 對 mix old-and-new CloudEvent 的 dedupe / append / replay 全部正確

### 0.3 GA prerequisites

於 `predictor-architecture-spec-v1alpha1.md` §0.3 列出。本 spec 額外要求：

1. `audit_outbox` 在 5 production tenants × 30 日 × 平均寫入量驗證 column additions 對 INSERT throughput 影響 < 5%（partition + multi-index 互動 sanity）
2. `verify-chain` 對 1M+ rows 大量 replay 全綠

### 0.4 何時可能需要 v2

只有以下情況開啟 v2：

- 出現第 19 個 audit chain 必擴欄位且該欄位語意上不適合 nested 進 cloudevent_payload data（罕見）
- Signing canonical bytes derivation 需要結構性改變（例如改 hash 演算法或加 Merkle tree）
- Contract DSL 升 v1beta1 時 audit schema 需 break

---

## §1. Context (self-contained)

### 1.1 為什麼有這份 spec

整套 predictor upgrade 跨 6 個 service（tokenizer / output_predictor / stats_aggregator / run_cost_projector / customer plugin / calibration_report），但**所有預測值都必須進 audit chain 才算數**。Audit chain 是 calibration evidence 的根；如果 prediction 不被簽章、不被 replicated、不能被 `verify-chain` replay，那「calibration-grade audit」這個產品承諾就垮。

具體說：

- 若 `predicted_b_tokens` 只存在記憶體 / log file → operator 可以聲稱「我們預測準確」但無 cryptographic 證據
- 若 `predicted_b_tokens` 存到 audit_outbox 但沒有 signature 覆蓋 → 可以 SQL UPDATE 後重新洗數據（即便 `verify-chain` 對 reserve/commit core fields 仍綠）
- 若 `predicted_b_tokens` 有 signature 但 trigger 容許 UPDATE → DBA / 滲透者可改寫並讓既有簽章「對」（只要欄位不在 trigger 鎖定清單內）

本 spec 把這三條 attack surface 全部關掉。

### 1.2 在 T → L → C → D → E → P 中的位置

```
T (Trace) → L (Ledger) → C (Contract DSL) → D (Decision) → E (Evidence) → P (Proof)
                              ↑           ↑              ↑
                          contract       hot            audit
                          rule eval      path           chain
                          (沒新增)        新增 18 cols    新增 18 mirror
                                          + 簽章覆蓋     fields in CE proto
```

本 spec 涉及 **D (Decision)** 與 **E (Evidence)** 兩階段：D 階段 sidecar 寫新欄位 + 對應 CloudEvent fields；E 階段 outbox_forwarder + canonical_ingest 把 mirrored CloudEvent 推到 storage class 並驗證簽章。

### 1.3 核心哲學

> **Audit chain 的 invariant 一個都不能變**：「無 audit 則無 effect」、append-only、signed、immutability triggers。
>
> **新欄位走 additive evolution**：proto3 additive field tags + Postgres ADD COLUMN nullable + 不引入新 storage class。
>
> **新欄位必須被 signature 覆蓋**：透過 mirror 到 CloudEvent extension attrs 達成，不修改 canonical_bytes derivation logic（mirror approach 比 derivation 改造低風險）。
>
> **Cross-storage consistency 透過 `verify-chain` 強制**：column value 必須等於 CloudEvent field value，否則 verify 失敗。

---

## §2. The 18 new columns inventory

所有新欄位皆 ADD COLUMN nullable（提供 backfill grace + backward compat）。型別選擇優先考慮 calibration-report query 性能。

### 2.1 Decision-side prediction columns (11)

加到 `audit_outbox` table（事件型別 `spendguard.audit.decision`）。

> Round-3 fix M1: type cells updated to match the round-2 SLICE_01 implementation (BIGINT for token counts per round-2 M4; NUMERIC(4,3) for prediction_confidence per round-2 M12; enums realized as TEXT + CHECK constraints rather than Postgres ENUM types). The CHECK constraints are declared NOT VALID + VALIDATEd per round-2 M6 / round-3 M14 deployment-safe pattern. Partial NOT-NULL via event_type-scoped CHECK with cutoff 2027-01-01 per round-3 B5.

| Column | Type | Nullable | 何時填 | Source |
|---|---|---|---|---|
| `predicted_a_tokens` | `BIGINT` (CHECK ≥ 0; >0 on .decision per round-3 M13) | NO on .decision past 2027-01-01 cutoff | 每次 decision | `output_predictor.Predict` strategy A |
| `predicted_b_tokens` | `BIGINT` (CHECK ≥ 0; >0 when strategy='B') | YES — null 表「該 bucket 樣本不足」 | 當 stats_aggregator 對 `(tenant, model, agent_id, prompt_class)` bucket 有 ≥30 samples | `output_predictor.Predict` strategy B |
| `predicted_c_tokens` | `BIGINT` (CHECK ≥ 0; >0 when strategy='C') | YES — null 表 customer plugin 未配置 / 失敗 / fallback | 當 tenant 有配 plugin endpoint 且健康 | `output_predictor.Predict` strategy C (delegated) |
| `reserved_strategy` | `TEXT` (CHECK IN ('A','B','C')) | NO on .decision past 2027-01-01 cutoff | 每次 decision | Sidecar policy resolver（per `predictor-architecture-spec-v1alpha1.md` §5） |
| `prediction_strategy_used` | `TEXT` (CHECK IN ('A','B','C')) | NO on .decision past 2027-01-01 cutoff；可能 != reserved_strategy（policy 為 STRICT_CEILING 時 reserved=A 但 prediction_strategy_used 可能仍是 B/C） | Sidecar |
| `prediction_policy_used` | `TEXT` (CHECK IN ('STRICT_CEILING','EMPIRICAL_RUN_CEILING','ADAPTIVE_CEILING','SHADOW_ONLY')) | NO on .decision past 2027-01-01 cutoff | 每次 decision | Contract evaluator |
| `tokenizer_tier` | `TEXT` (CHECK IN ('T1','T2','T3')) | NO on .decision past 2027-01-01 cutoff | 每次 decision | tokenizer service response |
| `tokenizer_version_id` | `UUID` | YES — null 表 Tier 3 fallback | T1/T2 時填；FK to `tokenizer_versions` registry table（new in SLICE 01）；partial index per round-3 M7 | tokenizer service |
| `prediction_confidence` | `NUMERIC(4,3)` (CHECK 0.000-1.000) | YES | B / C 有 sample 時填；A 永遠 null（A 是 lookup，不算 confidence；calibration-report filters `WHERE prediction_confidence IS NOT NULL`） | output_predictor 算 |
| `prediction_sample_size` | `BIGINT` (CHECK ≥ 0) | YES — null 表 cold-start / A | B 採樣大小；C 由 plugin 回報 | stats_aggregator / customer plugin |
| `cold_start_layer_used` | `TEXT` (CHECK IN ('L1','L2','L3','L4')) | YES — 只在 cold-start 觸發時填 | 當 B / C lookup fall through layer fallback | output_predictor |

註：`reserved_strategy` 與 `prediction_strategy_used` 在 `STRICT_CEILING` policy 下分別永遠是 `A` 與「實際 picked」。寫兩個欄位是為了 calibration-report 可以同時看「我們 reserved 的策略」與「我們會建議的策略」。

> Reviewer 注意：上表共 11 個欄位，但 HANDOFF §7 SLICE 01 說「10 prediction columns」。差異是本 spec 把 `cold_start_layer_used` 提升為獨立欄位（HANDOFF 將其 embed 在 prediction_confidence 的 metadata 中）。新增 1 個欄位的代價（trigger + CloudEvent mirror 各加 1 entry）對 calibration-report 「per layer 計算 ratio」的查詢性能換取明顯划算。請 review。

### 2.2 Run-level projection columns (3)

| Column | Type | Nullable | 何時填 | Source |
|---|---|---|---|---|
| `run_projection_at_decision_atomic` | `NUMERIC(38,0)` (CHECK ≥ 0 AND ≤ int64 max per round-2 M5) | NO on .decision past 2027-01-01 cutoff | 每次 decision | run_cost_projector |
| `run_predicted_remaining_steps` | `INT` (CHECK ≥ -1; -1 sentinel = projector unreachable) | YES — null 表 projector unreachable | 每次 decision；signal 1/2 算出 | run_cost_projector |
| `run_steps_completed_so_far` | `BIGINT` (CHECK ≥ 0 per round-2 M4) | NO on .decision past 2027-01-01 cutoff | 每次 decision | sidecar in-process state cache |

### 2.3 Commit-side actual columns (4)

加到 `audit_outbox` row of event_type = `'spendguard.audit.outcome'`（commit_estimated / provider_report event 的 row）。

| Column | Type | Nullable | 何時填 | Source |
|---|---|---|---|---|
| `actual_input_tokens` | `BIGINT` (CHECK ≥ 0) | NO on .outcome past 2027-01-01 cutoff | commit_estimated / provider_report | `LlmCallPostPayload.provider_reported.input_tokens` 或 sidecar 計算 |
| `actual_output_tokens` | `BIGINT` (CHECK ≥ 0) | NO on .outcome past 2027-01-01 cutoff | 同上 | 同上 |
| `delta_b_ratio` | `REAL` (CHECK ≥ 0.0 AND non-NaN per round-2 M3) | YES — null 表 prediction B 當時 null | commit_estimated；`actual_output_tokens / predicted_b_tokens` | sidecar 在 commit 時算 |
| `delta_c_ratio` | `REAL` (CHECK ≥ 0.0 AND non-NaN per round-2 M3) | YES — null 表 prediction C 當時 null | 同上 | 同上 |

註：`delta_a_ratio` 不加 —— A 永遠是 ceiling，`actual / A` 永遠 ≤ 1.0，calibration-report 直接從 `actual_output_tokens / predicted_a_tokens` 算即可，無需 materialize column。`delta_b/c_ratio` materialize 因為 B/C 是 mean estimator，ratio 是 calibration 主要訊號，frequent aggregation 該預先算好。

### 2.4 為什麼這 18 而不是 17 / 19

> Round-2 update：原本本節 caption 為「17 而不是 16/18」；現已合併 §2.1 reviewer-flagged 11th column (`cold_start_layer_used`) 的 +1 promotion，column 總數 17 → 18，所以 baseline 對比也 +1 為「18 而不是 17/19」。

- 17 個：合併 `reserved_strategy` 與 `prediction_strategy_used`。**拒絕**：兩者在非 STRICT_CEILING policy 下會分歧；calibration-report 需要兩個 signal 比對。
- 17 個（alternate）：把 `cold_start_layer_used` 留在 prediction_confidence 的 metadata blob 內。**拒絕**：calibration-report 需要 `WHERE cold_start_layer_used = 'L4'` 高效 query；blob-internal 欄位不能 indexed。
- 19 個：加 `delta_a_ratio`。**拒絕**：可從 `actual_output_tokens / predicted_a_tokens` 即時算，無 storage benefit。

---

## §3. CloudEvent proto mirror（為什麼 + 怎麼做）

### 3.1 為什麼必須 mirror

`audit_outbox.cloudevent_payload_signature` 在現有實作中（`services/canonical_ingest/src/verifier.rs::canonical_bytes_proto`）是對 **CloudEvent proto encoded bytes**（清空 signature 後）取 sha256 簽 Ed25519/KMS-ECDSA-P256。

如果新 prediction columns 只存在 audit_outbox row column 而**不在 CloudEvent proto 內**，則：

- column 值 **未被 signature 覆蓋**
- 即便 `reject_audit_outbox_immutable_columns` trigger 阻止 UPDATE，DBA 可直接從 Postgres backup 修改 column value 後 restore（trigger 在 backup restore 時不執行的場景）
- Operator 可主張「我們的 calibration P95 是 1.04」而沒有 cryptographic 證據

**Mirror 解決方案**：CloudEvent proto 新增 18 個對應 fields（與 column 1:1 mirror）。Producer 寫 column 同時也寫 CloudEvent field（同一 transaction）。Canonical_bytes_proto 自動 encode 新 fields（proto3 additive evolution）→ signature 覆蓋新 fields → 任何竄改 column 後不能再讓 verify_cloudevent 過。

### 3.2 Proto schema additions

加到 `proto/spendguard/common/v1/common.proto` 的 `CloudEvent` message 末尾，tag 從 300 開始：

```protobuf
message CloudEvent {
  // ... existing fields 1-203 ...

  // === Prediction extension attributes (per audit-chain-prediction-extension-v1alpha1.md) ===
  // Tags 300+. All optional (proto3 default = unset = "field absent on this event").
  // Decision-side fields populated only on event_type "spendguard.audit.decision".
  // Outcome-side fields populated only on event_type "spendguard.audit.outcome".

  int64  predicted_a_tokens = 300;             // always populated on .decision; mirror of BIGINT col
  int64  predicted_b_tokens = 301;             // populated when B available; mirror of BIGINT col
  int64  predicted_c_tokens = 302;             // populated when C available; mirror of BIGINT col
  string reserved_strategy = 303;              // "A" | "B" | "C"
  string prediction_strategy_used = 304;       // "A" | "B" | "C"
  string prediction_policy_used = 305;         // STRICT_CEILING | EMPIRICAL_RUN_CEILING | ADAPTIVE_CEILING | SHADOW_ONLY
  string tokenizer_tier = 306;                 // "T1" | "T2" | "T3"
  string tokenizer_version_id = 307;           // UUID v7; empty string on Tier 3
  float  prediction_confidence = 308;          // 0.0-1.0; absent = column-NULL on Strategy A row per §6.3 round-2 M11
  int64  prediction_sample_size = 309;         // round-3 M3: int32 → int64 to match BIGINT col; default 0 = "not applicable"
  string cold_start_layer_used = 310;          // "L1" | "L2" | "L3" | "L4"; empty = no cold start

  int64  run_projection_at_decision_atomic = 311;  // NUMERIC(38,0) serialized as int64 (assumes < 2^63 per round-2 M5)
  int32  run_predicted_remaining_steps = 312;       // default -1 = "projector unreachable" (use sentinel since proto3 default 0 conflates with "0 steps remaining")
  // Round-4 fix M1: int32 → int64 to match audit_outbox.run_steps_completed_so_far
  // (BIGINT). Wire-compatible with the round-3 int32 form because varint
  // encoding is identical for non-negative values per proto3 spec.
  int64  run_steps_completed_so_far = 313;

  int64 actual_input_tokens = 314;             // only on .outcome; mirror of BIGINT col
  int64 actual_output_tokens = 315;            // only on .outcome; mirror of BIGINT col
  float delta_b_ratio = 316;                   // only on .outcome; default 0.0 sentinel = "B prediction was null at decision time"
  float delta_c_ratio = 317;                   // only on .outcome
}
```

### 3.3 Sentinel value 設計

proto3 沒有「field absent」的原生語意（fields with default values are wire-encoded the same as unset fields）。對需要分辨「真的是 0」與「未填」的 fields 採 sentinel：

- `run_predicted_remaining_steps = -1` 表示「projector unreachable」（vs `0` 表示「真的剩 0 步」）
- `delta_b_ratio = 0.0` 表示「decision-time B was null」（vs `0.001` 等正常 ratio）；calibration-report 過濾 `WHERE delta_b_ratio > 0`
- `tokenizer_version_id = ""` 表示「Tier 3 fallback, no version」（vs UUID string）

審計層讀 audit_outbox column（非 CloudEvent proto）時看到的是 SQL NULL；wire 層 mirror 不到 NULL，只能 0/-1/""。Producer 需做 NULL ↔ sentinel 雙向轉譯（per §6.3 column-vs-proto consistency table）。

### 3.4 為什麼選 mirror approach 而非「擴展 canonical_bytes derivation 從 column 讀」

| 方案 | 優勢 | 風險 |
|---|---|---|
| **A. Mirror（採用）** | Canonical bytes derivation 不變；既有 verify_cloudevent code path 零修改；rollout 風險低；新欄位簽章自動覆蓋 | 寫入時 sidecar 需把同樣值寫到 column 與 CloudEvent field（重複），但同一 transaction 內所以無 consistency 風險 |
| B. Canonical bytes derivation 改造（從 audit_outbox row 讀 column）| 無 mirror duplication | canonical_bytes_proto 與 canonical_bytes_json 兩個 path 都要改；producer-side（sidecar）與 verifier-side（canonical_ingest）必須完全同步；任何不同步 = 簽章失敗；rollout 風險高 |
| C. 不簽章新欄位 | 最簡單 | 違反「calibration-grade audit」產品承諾；不可接受 |

選 A。Mirror duplication 的代價是 sidecar producer 多寫 18 個 proto field（內存中），這個 cost negligible vs B 的 rollout 風險。

---

## §4. Schema migration strategy

### 4.1 SLICE 01 migration（一次性）

新檔案：`services/canonical_ingest/migrations/0044_audit_outbox_prediction_columns.sql`（編號取決於 SLICE 01 實作時當前 migration 編號）。

```sql
-- Prediction extension columns per audit-chain-prediction-extension-v1alpha1.md §2.
-- All ADD COLUMN with explicit NULL default. No backfill — old rows stay NULL.
--
-- Round-2 / round-3 updates baked in:
--   * BIGINT for token counts (round-2 M4)
--   * NUMERIC(4,3) for prediction_confidence (round-2 M12 deterministic AVG)
--   * NOT VALID + VALIDATE for every CHECK (round-2 M6 / round-3 M14
--     deployment-safe lock pattern)
--   * Partial NOT-NULL via event_type-scoped CHECK with 2027-01-01 cutoff
--     (round-3 B5 — was 2026-07-01 calendar bomb)
--   * Sentinel-collision guards (round-3 M13) for predicted_a/b/c_tokens > 0
--   * Per-table TRUNCATE statement-level trigger using
--     reject_truncate_on_immutable_table() (round-3 M6 — replaces the
--     misleading reject_immutable_ledger_entry_mutation)
--
-- See services/ledger/migrations/0046_audit_outbox_prediction_columns.sql
-- and services/canonical_ingest/migrations/0013_canonical_events_prediction_columns.sql
-- for the verbatim production DDL — the snippet below is illustrative.

ALTER TABLE audit_outbox
  ADD COLUMN predicted_a_tokens         BIGINT,
  ADD COLUMN predicted_b_tokens         BIGINT,
  ADD COLUMN predicted_c_tokens         BIGINT,
  ADD COLUMN reserved_strategy          TEXT,
  ADD COLUMN prediction_strategy_used   TEXT,
  ADD COLUMN prediction_policy_used     TEXT,
  ADD COLUMN tokenizer_tier             TEXT,
  ADD COLUMN tokenizer_version_id       UUID,
  ADD COLUMN prediction_confidence      NUMERIC(4,3),
  ADD COLUMN prediction_sample_size     BIGINT,
  ADD COLUMN cold_start_layer_used      TEXT,

  ADD COLUMN run_projection_at_decision_atomic NUMERIC(38,0),
  ADD COLUMN run_predicted_remaining_steps     INT,
  ADD COLUMN run_steps_completed_so_far        BIGINT,

  ADD COLUMN actual_input_tokens  BIGINT,
  ADD COLUMN actual_output_tokens BIGINT,
  ADD COLUMN delta_b_ratio        REAL,
  ADD COLUMN delta_c_ratio        REAL;

-- Then ALTER TABLE ... ADD CONSTRAINT ... CHECK (...) NOT VALID for every
-- domain enum + sentinel-collision guard; then ALTER TABLE ... VALIDATE
-- CONSTRAINT for each. See 0046 step 2 + step 3 + step 3b.

-- Indexes for calibration-report (CLI per calibration-report-spec-v1alpha1.md).
-- Round-2 M9: tenant_id first; tenant_id-scoped query patterns get
-- index-only scans without bitmap heap pass.
CREATE INDEX audit_outbox_calibration_idx
  ON audit_outbox (tenant_id, recorded_month, prediction_strategy_used, prediction_policy_used)
  WHERE event_type = 'spendguard.audit.decision';

CREATE INDEX audit_outbox_tier_idx
  ON audit_outbox (tenant_id, recorded_month, tokenizer_tier)
  WHERE event_type = 'spendguard.audit.decision';

CREATE INDEX audit_outbox_outcome_calibration_idx
  ON audit_outbox (tenant_id, recorded_month, prediction_strategy_used)
  INCLUDE (delta_b_ratio, delta_c_ratio, actual_output_tokens)
  WHERE event_type = 'spendguard.audit.outcome'
    AND (delta_b_ratio IS NOT NULL OR delta_c_ratio IS NOT NULL);

-- Round-3 M7: partial index supporting the FK from
-- audit_outbox.tokenizer_version_id -> tokenizer_versions.
CREATE INDEX audit_outbox_tokenizer_version_id_idx
  ON audit_outbox (tokenizer_version_id)
  WHERE tokenizer_version_id IS NOT NULL;

-- New tokenizer_versions registry table per tokenizer-service-spec-v1alpha1.md §6.
-- Final DDL lives in services/ledger/migrations/0048_tokenizer_versions.sql:
-- immutability trigger + TRUNCATE guard + REVOKE PUBLIC + role grants.
CREATE TABLE IF NOT EXISTS tokenizer_versions (
  tokenizer_version_id UUID PRIMARY KEY,
  kind                 TEXT NOT NULL CHECK (kind IN ('OPENAI_TIKTOKEN','ANTHROPIC_BPE','GEMINI_BPE','COHERE_BPE','SENTENCEPIECE_LLAMA','HEURISTIC')),
  encoder_name         TEXT NOT NULL,
  version_string       TEXT NOT NULL,
  asset_sha256         TEXT NOT NULL,
  registered_at        TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
  retired_at           TIMESTAMPTZ,
  UNIQUE (kind, encoder_name, version_string)
);
```

### 4.2 Partition 互動

`audit_outbox` 是 partitioned by `recorded_month`。`ALTER TABLE ... ADD COLUMN nullable` 對 partition 是 cheap operation —— PostgreSQL 不需要 rewrite 既有 rows，所有新 columns 預設 NULL。Migration 對 production-scale partition 無 downtime。

### 4.3 Backfill 策略

**No backfill**。既有 demo rows 與 production rows 的新欄位永遠保持 NULL。理由：

- 既有 rows 的 prediction values 是 historical fact 不可重建（18 個 heuristic 欄位都不會回填合理的 B/C 預測值）
- `verify-chain` 對 NULL 的 column + proto3 default 的 CloudEvent field 仍能正確 verify（per §7）
- calibration-report 對 NULL 過濾，只計新 rows

---

## §5. Immutability trigger update（critical）

### 5.1 為什麼這節是 critical

`services/ledger/migrations/0011_immutability_triggers.sql` 的 `reject_audit_outbox_immutable_columns` 函式 hardcodes 14 個 immutable columns 的 OLD/NEW comparison。**任何不在此清單的 column 在 UPDATE attempt 中會被忽略**（trigger 不檢查 = 視為可變）。

如果 SLICE 01 只 ADD COLUMN 而**未同步更新 trigger**，新欄位將：

- 可被 outbox_forwarder 的 UPDATE pending_forward 路徑誤改（forwarder 用 ORM 或某些 ON CONFLICT 寫法可能誤觸 UPDATE all columns）
- 可被 DBA 手動 UPDATE 改寫 calibration evidence

**這就是 HANDOFF Step 4 discrepancy #4 的 risk**。

### 5.2 Trigger function update（必為 SLICE 01 同一 migration 的一部分）

```sql
-- Replace the function with updated OLD/NEW comparison list (additive over 0011).
CREATE OR REPLACE FUNCTION reject_audit_outbox_immutable_columns()
RETURNS TRIGGER AS $$
BEGIN
    IF (OLD.audit_outbox_id, OLD.audit_decision_event_id, OLD.decision_id,
        OLD.tenant_id, OLD.ledger_transaction_id, OLD.event_type,
        OLD.cloudevent_payload, OLD.cloudevent_payload_signature,
        OLD.ledger_fencing_epoch, OLD.workload_instance_id,
        OLD.recorded_at, OLD.recorded_month,
        OLD.producer_sequence, OLD.idempotency_key,
        -- === NEW prediction columns (per audit-chain-prediction-extension §5.2) ===
        OLD.predicted_a_tokens, OLD.predicted_b_tokens, OLD.predicted_c_tokens,
        OLD.reserved_strategy, OLD.prediction_strategy_used,
        OLD.prediction_policy_used, OLD.tokenizer_tier, OLD.tokenizer_version_id,
        OLD.prediction_confidence, OLD.prediction_sample_size, OLD.cold_start_layer_used,
        OLD.run_projection_at_decision_atomic, OLD.run_predicted_remaining_steps,
        OLD.run_steps_completed_so_far,
        OLD.actual_input_tokens, OLD.actual_output_tokens,
        OLD.delta_b_ratio, OLD.delta_c_ratio)
       IS DISTINCT FROM
       (NEW.audit_outbox_id, NEW.audit_decision_event_id, NEW.decision_id,
        NEW.tenant_id, NEW.ledger_transaction_id, NEW.event_type,
        NEW.cloudevent_payload, NEW.cloudevent_payload_signature,
        NEW.ledger_fencing_epoch, NEW.workload_instance_id,
        NEW.recorded_at, NEW.recorded_month,
        NEW.producer_sequence, NEW.idempotency_key,
        NEW.predicted_a_tokens, NEW.predicted_b_tokens, NEW.predicted_c_tokens,
        NEW.reserved_strategy, NEW.prediction_strategy_used,
        NEW.prediction_policy_used, NEW.tokenizer_tier, NEW.tokenizer_version_id,
        NEW.prediction_confidence, NEW.prediction_sample_size, NEW.cold_start_layer_used,
        NEW.run_projection_at_decision_atomic, NEW.run_predicted_remaining_steps,
        NEW.run_steps_completed_so_far,
        NEW.actual_input_tokens, NEW.actual_output_tokens,
        NEW.delta_b_ratio, NEW.delta_c_ratio) THEN
        RAISE EXCEPTION 'audit_outbox immutable columns cannot be changed (incl. prediction extension cols)'
            USING ERRCODE = '42P10';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
```

**SLICE 01 PR 必含此 trigger update。Adversarial review checklist 必驗證**：對 18 個新欄位的 UPDATE attempt（在 demo Postgres 上）全部 raise `42P10`。

### 5.3 Forwarder UPDATE path 仍允許 4 個 forwarder state columns

per `0009_audit_outbox.sql` 註解：「Only forwarder state fields are UPDATE-able」 —— `pending_forward / forwarded_at / forward_attempts / last_forward_error` 仍可被 outbox_forwarder UPDATE。Trigger 邏輯（OLD/NEW IS DISTINCT FROM 整個 tuple）正確處理：forwarder UPDATE 只改 4 個允許欄位時，其他 14+18=32 個 columns 的 OLD 與 NEW 相等 → tuple 整體相等 → trigger 通過。

---

## §6. Canonical bytes derivation impact

### 6.1 結論：不改 code

`services/canonical_ingest/src/verifier.rs` 與 producer-side 對應的 `canonical_bytes` 函式（住在 `services/sidecar/src/audit.rs` / `services/webhook_receiver/src/audit.rs` / `services/ttl_sweeper/src/audit.rs` / `services/ledger/src/handlers/invoice_reconcile.rs`）**全部保持現狀**。

理由：
- proto3 additive evolution 自動處理新 fields —— `prost::Message::encode_to_vec` encode 所有 set fields，包括新加的 300-317
- `canonical_bytes_proto` 對整個 CloudEvent encode（signature 清空後）—— 新 fields 自動進 hash 輸入
- 既有 rows 的 CloudEvent 沒有 300-317 fields → encode 為 proto3 unset → 與 verify 時 reproduce 一致

### 6.2 為什麼 JSON canonical path 同樣不需改

`canonical_bytes_json` 用於 ledger-minted decision rows（`producer_id.starts_with("ledger:")`），目前用於 `InvoiceReconcile` decision row。Invoice reconcile path **不寫 prediction columns**（reconcile event 是 commit-side 的 invoice 對齊，不是 decision-side prediction）。所以 JSON path 不涉及新欄位。

若未來 InvoiceReconcile 需要記錄 prediction context（unlikely），需擴展 `canonical_bytes_json` 的 payload 物件 keys 清單；本 spec lock 時不需要。

### 6.3 Producer 端 column ↔ proto field 一致性

每個 audit producer service（sidecar 等）寫 audit_outbox row 時必須**同時**填 column 與對應 CloudEvent proto field。對應表：

| audit_outbox column | CloudEvent proto field tag | NULL → proto 對應值 |
|---|---|---|
| `predicted_a_tokens` | 300 | (always non-null; no NULL case) |
| `predicted_b_tokens` | 301 | `0`（proto3 default；calibration-report 過濾 `WHERE predicted_b_tokens IS NOT NULL`） |
| `predicted_c_tokens` | 302 | `0` |
| `reserved_strategy` | 303 | (always non-null) |
| `prediction_strategy_used` | 304 | (always non-null) |
| `prediction_policy_used` | 305 | (always non-null) |
| `tokenizer_tier` | 306 | (always non-null) |
| `tokenizer_version_id` | 307 | `""`（empty string for Tier 3） |
| `prediction_confidence` | 308 | **0.0-1.0 範圍；absent = column-NULL on Strategy A row**（round-2 fix M11: 原 v1 寫「0.0 = N/A (Strategy A)」是錯的 —— Strategy A row 的 column 是 SQL NULL，proto3 沒有 NULL 概念故 wire 上 encode 為 0.0，但 calibration-report query 必須 filter `WHERE prediction_confidence IS NOT NULL`；0.0 不是 sentinel，是 column-NULL 的 proto3 default。詳見 round-2 spendguard-prediction-mirror crate 的 mapping table。） |
| `prediction_sample_size` | 309 | `0`（NULL → 0 proto3 default; filter `IS NOT NULL`） |
| `cold_start_layer_used` | 310 | `""`（empty string for "no cold start"） |
| `run_projection_at_decision_atomic` | 311 | (always non-null; constrained ≤ int64 max per round-2 M5) |
| `run_predicted_remaining_steps` | 312 | `-1`（sentinel for "projector unreachable"; distinguishes from "0 steps remaining"） |
| `run_steps_completed_so_far` | 313 | `0`（round-4 fix M10 + M1: wire type int64 to match BIGINT col; NULL → proto3 default 0 acceptable per §3.3 because calibration-report filters `WHERE run_steps_completed_so_far IS NOT NULL`; mirror crate variant MirrorField::RunStepsCompletedSoFar added round-4） |
| `actual_input_tokens` | 314 | (always non-null on .outcome) |
| `actual_output_tokens` | 315 | (always non-null on .outcome) |
| `delta_b_ratio` | 316 | `0.0`（sentinel; filter `WHERE delta_b_ratio > 0` in calibration-report） |
| `delta_c_ratio` | 317 | `0.0`（sentinel; filter `WHERE delta_c_ratio > 0` in calibration-report） |

**Invariant**：寫入後，`audit_outbox` row 的 column 值與 stored CloudEvent 的 proto field 值必須**邏輯一致**（NULL ↔ sentinel 對應）。verify-chain CLI 在 replay 時 enforce 此一致性（§11.2）。

---

## §7. verify_cloudevent compatibility

### 7.1 Backward compat — 既有 rows

既有 demo + production rows 的 CloudEvent payload 沒有 prediction proto fields（tags 300-317）。verify_cloudevent 重 derivation canonical_bytes 時：

```
producer 簽 = sha256(encode_to_vec(CloudEvent_old_without_300_317))
verifier 算 = sha256(encode_to_vec(CloudEvent_old_without_300_317))
→ 簽章 match
```

proto3 unknown-field semantics 規定：encoded bytes 不含未設 fields。所以 producer-side encode 與 verifier-side encode 對既有 rows 是 byte-identical。**既有 audit chain 100% 不受影響**。

### 7.2 Forward compat — 新 rows on old verifier (假設情境)

若某客戶部署舊版 canonical_ingest 接收新版 sidecar 的 events（rolling upgrade gap）：

```
新 producer 簽 = sha256(encode_to_vec(CloudEvent_with_300_317))
舊 verifier 算 = sha256(encode_to_vec(CloudEvent_with_unknown_fields_300_317))
```

**Critical question**：prost 對 unknown fields 的處理？

> **Round-2 update (SLICE_01 implementation)**：實測 `prost 0.13` **不保留** proto3 unknown fields（upstream issue tokio-rs/prost#879）。原本本節假設「prost 自動保留 unknown wire bytes」**並不成立**。

實作影響 + 取代方案（rollout invariant）：

- 舊 canonical_ingest decode 新 CloudEvent → tag 300-317 fields 被**丟棄**
- 舊 canonical_ingest re-encode → encoded bytes **不含** 300-317 → canonical_bytes **與 producer 簽的 hash 不同** → verify FAIL
- 因此：**rolling upgrade 期間 canonical_ingest pods 必須先全部 upgrade，才可讓任何 sidecar / webhook_receiver / ttl_sweeper 寫 tag-300+ fields**
- 此 invariant 由 Helm chart 的 deployment ordering 強制執行（charts/spendguard/templates/migrations.yaml + NOTES.txt 告警 operator）
- prost upstream 修好後（pre-GA 目標），本節恢復原本 byte-identical re-encode 論證；現在 invariant 寫在 build.rs 註解 + spec §7.2 + slice §7

SLICE_01 acceptance 包含 `services/canonical_ingest/src/verifier.rs::tests` 的兩個 property test：
- `prost_roundtrip_preserves_tag_300_to_317_fields` —— 確認**已知 fields** 的 round-trip byte-identical
- `legacy_event_signature_survives_proto_bump` —— 確認既有 rows（tag 300+ fields 為 proto3 default）在新 verifier 下 verify 仍通過（§7.1 invariant）

未涵蓋：unknown-field preservation property test —— prost 0.13 不支援，等 upstream 進展。

### 7.3 Forward compat — 舊 rows on new verifier

新 verifier 對舊 rows decode 時：300-317 fields 全部 proto3 default。encode_to_vec → 不含 300-317 wire bytes → 與舊 producer 簽的 hash 一致 → verify pass。

### 7.4 與既有兩種 canonical encoding 共存

`producer_id.starts_with("ledger:")` 判別仍生效。Mirror 機制 only on sidecar/webhook_receiver/ttl_sweeper（proto path），不影響 ledger-minted JSON path（per §6.2）。

---

## §8. canonical_ingest replication impact

### 8.1 行為不變

`outbox_forwarder` 讀 `pending_forward=TRUE` row 後 push to `canonical_ingest.AppendEvents`。新欄位以 CloudEvent proto field 形式 ride along，canonical_ingest 對 `AppendEvents` 流程：

1. dedupe by `event_id` — 不變
2. per-`(tenant_id, decision_id)` sequence check — 不變
3. `verify_cloudevent` signature check — 不變（per §7）
4. write to storage class —— **不變**（全部仍是 `immutable_audit_log` class）

### 8.2 為什麼不需要新 storage class

per `trace-schema-spec-v1alpha1.md` §10.2，三層 storage class 由 retention + cleartext 規則區分。新 prediction fields：

- 不含 PII（純 token count + 策略字串）
- 7-year SOX retention 適用（calibration evidence 屬規範性審計範圍）
- 不需要 RTBF 刪除（無 user-identifiable subject）

→ 完美 fit `immutable_audit_log` class。無新 class 需求。

---

## §9. Signature schema bump policy

### 9.1 結論：no signature schema bump

`signing_key_id` 與 producer_signature 的 wire format 不變。Ed25519 / KMS-ECDSA-P256 algorithm 不變。

### 9.2 Schema bundle id rotation

per `trace-schema-spec-v1alpha1.md` §12，每個 producer 在 startup 註冊 `schema_bundle_id` 給 canonical_ingest。當 CloudEvent proto schema 改變（即便 additive）時，sensible practice 是 rotate schema_bundle_id 並通知 canonical_ingest 註冊新 bundle。

具體（round-3 fix B3 update）：

- **SLICE_01 不再** insert 任何 schema_bundle 列。Round-2 曾嘗試 ship `0014_schema_bundle_prediction_v1alpha1.sql` with a placeholder hash + NULL cosign_verified_at，被 round-3 security review (B3) 反向 — placeholder hash 為 `sha256("spendguard.v1alpha1+prediction")` 可逆 + cosign 未驗證 = supply-chain hostile（攻擊者拿 producer signing key 就能 synth events that the placeholder "verifies"）。0014 已被刪除。
- SLICE_06 producer slice 負責 register the real cosigned bundle row：operator-side bundle builder 對 canonicalized proto bytes 計算實際 sha256 + 記錄 cosign verification。SLICE_06 producers MUST NOT write tag-300+ CloudEvent fields BEFORE this row exists.
- canonical_ingest 收到第一個新 bundle event 時 emit `schema_bundle_registered` audit event
- 既有 events 仍引用舊 bundle_id；canonical_ingest 對 mixed bundle stream 已有處理（per Trace §12 conformance test corpus）

---

## §10. outbox_forwarder impact

**零行為變更**。forwarder 讀 `pending_forward=TRUE` row、push 到 canonical_ingest、UPDATE `pending_forward=FALSE / forwarded_at / forward_attempts`。新欄位 ride along 在 cloudevent_payload + 新 prediction columns，forwarder 不關心它們的語意。

唯一 implicit 影響：UPDATE statement 若用 `UPDATE audit_outbox SET pending_forward = ...` 不會觸發 immutability trigger（因為 trigger 比較整 tuple，僅有允許的 forwarder columns 變）。SLICE 01 acceptance 必驗證 forwarder UPDATE path 仍 work。

---

## §11. verify-chain CLI impact

### 11.1 既有 verify-chain 行為

`verify-chain` CLI（住在 canonical_ingest service / 或獨立 binary，per `trace-schema-spec-v1alpha1.md`）對 audit chain 做 end-to-end replay verification：

1. 對 query 範圍內每個 audit_outbox row，重新 derive canonical_bytes
2. 與 stored `cloudevent_payload_signature` 對 `signing_key_id` 對應的 verifying key 驗 signature
3. 報告 PASS / FAIL per row

### 11.2 新增 cross-storage consistency check

verify-chain CLI 在 §11.1 step 1-2 之上新增 step 3：

```
for each audit_outbox row:
  derive canonical_bytes from cloudevent_payload  # 既有 step
  verify signature  # 既有 step
  # === NEW: cross-storage consistency ===
  for each (column_name, proto_field_tag) in PREDICTION_MIRROR_TABLE:
    col_value = row[column_name]
    proto_value = parsed_cloudevent.get_field(proto_field_tag)
    expected_proto_value = column_to_proto_sentinel(col_value)
    if proto_value != expected_proto_value:
      FAIL with "Mirror inconsistency: column %s = %s but CloudEvent.%d = %s"
```

此 check 抓的攻擊面：column 值被 tampered（unlikely 因為有 immutability trigger，但 trigger 不在 backup restore 時觸發）。第二層防線。

### 11.3 CLI flag

verify-chain CLI gains `--check-prediction-mirror` flag（default `true` for new versions; `false` 可用於 audit 既有 NULL-prediction rows，跳過 mirror check 但仍跑 signature verify）。

---

## §12. Failure modes

| 場景 | 何時發生 | 行為 |
|---|---|---|
| sidecar writes column but proto field mismatch | sidecar bug；應在 producer-side unit test catch | UPDATE rejected by trigger（不允許 row-state divergence post-INSERT）；INSERT 仍會 succeed → verify-chain 後續 catch mismatch → row 標 `quarantined` |
| Producer signs CloudEvent but DB INSERT fails | rare; e.g., disk full mid-transaction | Tx rollback → 無 audit_outbox row → 無 effect（per Contract §6.1 invariant） |
| 新欄位的 sentinel 與正常值衝突 | e.g., `run_predicted_remaining_steps = -1` 因 projector unreachable 與「真的剩 -1」混淆 | sentinel 設計刻意避開合法範圍（remaining_steps 必 ≥ 0；delta_b_ratio 必 > 0 when populated）；calibration-report SQL 對 sentinel 過濾顯式 |
| 舊版 verify_cloudevent 對新 CloudEvent | rolling upgrade | prost preserve unknown fields → re-encode byte-identical → verify pass |
| migration ADD COLUMN 與 partition 巨大 | 5M+ rows, 12-month partitioned | ADD COLUMN nullable 是 metadata-only operation；無 row rewrite；migration 秒級完成 |

---

## §13. GA prerequisites

於 §0.3 列出。額外：

1. `verify-chain --check-prediction-mirror` 對 100K+ rows mixed-version sample 全綠
2. Backup-restore drill：模擬 backup 後 manually tamper 一個 prediction column → restore → verify-chain --check-prediction-mirror catch the tamper
3. Cross-region replication（若客戶啟用 active-passive）對新 columns 同步 lag < 1s p99

---

## §14. Adoption history

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder — filled during Codex / panel adversarial review rounds per HANDOFF §9) |

---

## §15. Lock 後的下一步

1. SLICE 01 migration（`0044_audit_outbox_prediction_columns.sql`）+ trigger update + tokenizer_versions table 三合一 migration
2. CloudEvent proto bump（`proto/spendguard/common/v1/common.proto` add tags 300-317）+ schema_bundle_id rotation
3. Producer-side mirror logic 加進 `services/sidecar/src/audit.rs`（或對應 commit-side service）
4. verify-chain CLI 加 `--check-prediction-mirror` flag 與實作
5. Acceptance test：8+ demo modes 全綠；`verify-chain` regression 對既有 + 新 rows 全綠；mirror tamper 攻擊測試 catch

---

*Document version: audit-chain-prediction-extension-v1alpha1 (DRAFT) | Drafted: 2026-05-29 | Companion: full predictor-upgrade spec set; locks together per `predictor-architecture-spec-v1alpha1.md` §0.2 | Critical surface: §5 trigger update; §3 CloudEvent mirror; §7 verify_cloudevent compat | Branch: `design/predictor-upgrade`*
