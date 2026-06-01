# Stats Aggregator Specification — v1alpha1 (DRAFT)

> 📝 **Status: DRAFT** (writing in design phase on branch `design/predictor-upgrade`)
> **DRAFT → LOCKED criteria**: locks together with the predictor-upgrade spec set per `predictor-architecture-spec-v1alpha1.md` §0.2; additionally requires (a) `output_distribution_cache` freshness ≤ 1h p99 in production for 7 consecutive days, (b) `prediction_drift_alert` event detection precision ≥ 95% on a synthetic drift injection corpus, (c) zero cross-tenant data leakage verified by adversarial query injection chaos test.
> **Companion specs (this set)**: `predictor-architecture-spec-v1alpha1.md` (umbrella; pillar reasoning Q1 no-ML), `output-predictor-service-spec-v1alpha1.md` (consumer of cache + drift alerts), `audit-chain-prediction-extension-v1alpha1.md` (defines the `canonical_events` columns this service reads), `calibration-report-spec-v1alpha1.md` (alt consumer of the same cache).
> **Pre-existing LOCKED dependencies**: `ledger-storage-spec-v1alpha1.md` (canonical_events upstream), `trace-schema-spec-v1alpha1.md` (CloudEvent envelope), `sidecar-architecture-spec-v1alpha1.md` (multi-tenant isolation invariant).
> **Compatibility policy**: alpha — cache schema additive; SQL aggregation queries versioned via `aggregation_version` column in cache rows; drift threshold per-tenant override allowed via control plane API.

---

## §0. Lock status & prerequisites

### 0.1 範圍

本 spec 定義 **stats_aggregator service**：純 SQL 聚合 job，從 `canonical_events`（per `audit-chain-prediction-extension-v1alpha1.md` §2 newly-populated columns）算 per-(tenant, model, agent_id, prompt_class) bucket 的 P50/P95/P99 output token 分布，寫進 `output_distribution_cache`，並偵測 2σ drift 後 emit `prediction_drift_alert` 事件。

**正名**：本 service 在 HANDOFF / 早期 draft 曾名為 `predictor_trainer`。**廢棄該名稱**。本 service 不訓練任何 ML model，名稱必須反映「pure SQL stats job」這事實，避免讓 reviewer / customer 誤解架構（per `predictor-architecture-spec-v1alpha1.md` §3.1 Q1 reasoning #3）。

**不在本 spec 範圍**：

- ML model training（永遠不做；per Q1 lock）
- 預測值的計算與消費（推給 `output-predictor-service-spec-v1alpha1.md`）
- 新欄位的 audit chain 結構（推給 `audit-chain-prediction-extension-v1alpha1.md`）
- Calibration-report CLI 的 query 形態（推給 `calibration-report-spec-v1alpha1.md`）

### 0.2 DRAFT → LOCKED criteria

進入 LOCKED 之前下列 5 項必達成：

1. SLICE 06 PR merged：stats_aggregator service skeleton + `output_distribution_cache` table + 第一個 aggregation cycle run end-to-end
2. Aggregation latency p99 ≤ 15 min 對 100K daily decision events
3. Cache freshness p99 ≤ 1h（per `§11.1` SLO）on demo deployment
4. Drift detection 對 synthetic injected drift（2σ+ ratio shift）precision ≥ 95% recall ≥ 90%
5. Cross-tenant query injection chaos test 全綠（adversarial query attempts to read other tenant's cache rows 全被 RLS 拒絕）

### 0.3 GA prerequisites

於 `predictor-architecture-spec-v1alpha1.md` §0.3 列出。本 spec 額外要求：

1. 5 production tenants × 30 日 cache 連續 freshness < 1h
2. 至少一個真實 drift 事件被偵測 + operator 後驗證 root cause（vendor model release / customer agent change）
3. Aggregation job 在 partitioned canonical_events（≥ 6 months × 5M+ rows/month）效率測試通過

### 0.4 何時可能需要 v2

- 新增第 8 個 prompt class（超過 §1.4 七大 bucket）
- Aggregation 需 sub-hour cadence（per-call real-time signal demand）
- 引入 ML（永遠不會；觸發 v2 = 違反 Q1 lock）

---

## §1. Context (self-contained)

### 1.1 為什麼有這份 spec

`output_predictor.Predict` 的 Strategy B 需要 「per-(tenant, model, agent_id, prompt_class) 的歷史 output token P95」 lookup。這個 lookup 必須：

- **Pre-computed**：query path 不能 ad-hoc 算 `percentile_cont(0.95) WITHIN GROUP` over 1M+ rows（hot path 50ms SLO 不允許）
- **Per-tenant isolated**：tenant A 的 prompt patterns 不可洩漏給 tenant B
- **Drift-aware**：當分布顯著偏移（vendor upgrade、agent behavior change、prompt class redefine），下游必須得知

stats_aggregator 是滿足這 3 個條件的最小設計：跑 SQL aggregation，寫 pre-computed cache，比較 historical baseline 偵 drift。

### 1.2 為什麼 NOT ML（強調）

per `predictor-architecture-spec-v1alpha1.md` §3.1 Q1 4 條 reasoning：

1. Multi-tenant ML 跨租戶 prompt leak
2. 客戶自己的 ML 更懂 agent
3. ML lifecycle 屬另一個產品
4. Deterministic enforcement 是 regulated-environment 採購優勢

ML training 整套擺進 customer plugin（per `output-predictor-plugin-contract-v1alpha1.md`）—— SpendGuard 提供 Strategy C 的 contract，不 host model。

### 1.3 v1alpha1 核心哲學

> **Pure SQL；no GPU；no model registry；no A/B framework**。基礎設施複雜度限制在 Postgres + scheduler。
>
> **Aggregation 是 derived data**；canonical_events 是 source of truth，cache 只是優化；丟掉 cache 可重建。
>
> **Drift detection 是統計判斷**；無 ML model；用 rolling window 2σ shift threshold。
>
> **Multi-tenant 強制隔離**；每個 query 必含 tenant_id；RLS policy 強制；adversarial query injection 必須失敗。

---

## §2. Service surface

### 2.1 不是 gRPC service —— 是 cron 化 SQL job

`stats_aggregator` 不暴露 RPC endpoint。它是一個 daemon process + cron scheduler：

```
services/stats_aggregator/
├── Cargo.toml
├── src/
│   ├── main.rs                 # daemon + signal handling + graceful shutdown
│   ├── scheduler.rs            # cron-like trigger (default hourly)
│   ├── aggregation.rs          # SQL queries + percentile computation
│   ├── drift_detector.rs       # 2σ comparison + event emission
│   ├── run_length.rs           # (tenant, agent_id) run-length distribution
│   └── db.rs                   # connection pool to ledger Postgres
```

**Output**：寫 `output_distribution_cache` table + emit CloudEvents（`spendguard.audit.prediction_drift_alert.v1alpha1`）to canonical_ingest。

> HARDEN_04 drift reconciliation: SLICE_06 merge `d00287f` / implementation commit `f8dc34c` made the drift alert audit-routed, and HARDEN_03 merge `16f0194` preserves durable append-result checking. The earlier non-audit draft type is non-authoritative.

**Input**：read-only 對 `canonical_events`（`audit_outbox` 透過 `outbox_forwarder` 傳到的鏡像）。

### 2.2 Deployment 形態

| 部署模式 | 形態 |
|---|---|
| K8s SaaS managed | Single replica Deployment + Leader election via Postgres advisory lock |
| K8s self-hosted | 同上 |
| Lambda | 不適用 —— 持續 daemon 不符合 Lambda lifecycle |
| Air-gapped | Single replica VM daemon |

只允許 single writer（per tenant；可 sharded by tenant range for huge multi-tenant deployments）—— double aggregation 會造成 cache row UPDATE 衝突 + 浪費 compute。

---

## §3. Subscription model

### 3.1 Read source

`canonical_events` 是 audit chain 終端（per `trace-schema-spec-v1alpha1.md` §10.2）。stats_aggregator 對其 read-only：

```sql
SELECT
  tenant_id,
  model,             -- mirror column populated from canonical_events.payload_json
  agent_id,
  prompt_class,
  actual_output_tokens
FROM canonical_events
WHERE event_type = 'spendguard.audit.outcome'
  AND actual_output_tokens IS NOT NULL
  AND ingest_at >= now() - interval '30 days'
  AND tenant_id = $1;
```

（real query 更複雜；details §4。此段示範 input shape。`payload_json` 是 canonical CloudEvent envelope 欄位；SLICE_06 implementation commit `8436cd4` / HARDEN_03 merge `16f0194` made `model`, `agent_id`, `run_id_mirror`, `prompt_class`, and `prompt_class_fingerprint` first-class mirror columns so the hot aggregation query does not repeatedly decode JSON.）

### 3.2 Bucket key

四元組（per HANDOFF §3.4 + `output-predictor-service-spec-v1alpha1.md` §7）：

```
(tenant_id, model, agent_id, prompt_class)
```

- `tenant_id`：multi-tenant isolation
- `model`：跨 model token 分布顯著不同（per `proto/spendguard/common/v1/common.proto` UnitRef.model_family doc）
- `agent_id`：同 tenant 不同 agent（customer-support vs code-review）有極大分布差異
- `prompt_class`：7-class classifier label（chat_short / chat_long / code_gen / summarization / rag / tool_calling / vision）used by the hot output-predictor lookup and by `output_distribution_cache` primary key.
- `prompt_class_fingerprint`：non-key audit metadata over canonicalized template structure（NOT content）for later forensic inspection; it is carried in `canonical_events` but not used as the stats_aggregator bucket key.

HARDEN_04 reconciliation: shipped migrations `0016_output_distribution_cache.sql`
and `0018_canonical_events_aggregator_mirror_columns.sql`, plus implementation
commit `8436cd4`, key aggregation rows on `prompt_class`; the fingerprint is
retained only as mirror/audit metadata.

### 3.3 Prompt class and fingerprint derivation

`prompt_class_fingerprint` 是 sidecar 在 decision time 算出來寫進 audit row 的 hash。算法：

```rust
// Pseudocode
fn prompt_class_fingerprint(messages: &[Message], model: &str) -> String {
    let canonical = canonicalize_template(messages);  // strip content, keep structure
    let class = classify(canonical, model);  // 7-way classifier; rule-based
    format!("v1:{}", class)  // versioned for future refactor
}
```

完整 classifier 規則 + 7 個 bucket 邊界定義在 `output-predictor-service-spec-v1alpha1.md` §8。stats_aggregator **不**重算 fingerprint；it groups by the stored `prompt_class` mirror column and preserves the fingerprint only for audit/debug correlation.

---

## §4. Aggregation queries

### 4.1 主 aggregation query (per bucket)

```sql
-- Run once per aggregation cycle (hourly default).
-- Writes one row per (tenant, model, agent_id, prompt_class) bucket
-- to output_distribution_cache.

WITH events_30d AS (
  SELECT
    tenant_id,
    model,
    agent_id,
    prompt_class,
    actual_output_tokens,
    ingest_at
  FROM canonical_events
  WHERE event_type = 'spendguard.audit.outcome'
    AND actual_output_tokens IS NOT NULL
    AND ingest_at >= now() - interval '30 days'
    AND recorded_month >= DATE_TRUNC('month', now() - interval '30 days')::DATE
    AND model IS NOT NULL
    AND agent_id IS NOT NULL
    AND prompt_class IS NOT NULL
),
agg_30d AS (
  SELECT
    tenant_id, model, agent_id, prompt_class,
    percentile_cont(0.50) WITHIN GROUP (ORDER BY actual_output_tokens) AS p50_30d,
    percentile_cont(0.95) WITHIN GROUP (ORDER BY actual_output_tokens) AS p95_30d,
    percentile_cont(0.99) WITHIN GROUP (ORDER BY actual_output_tokens) AS p99_30d,
    avg(actual_output_tokens)::REAL AS mean_30d,
    stddev_samp(actual_output_tokens)::REAL AS stddev_30d,
    count(*) AS sample_size_30d
  FROM events_30d
  GROUP BY tenant_id, model, agent_id, prompt_class
),
agg_7d AS (
  SELECT
    tenant_id, model, agent_id, prompt_class,
    percentile_cont(0.50) WITHIN GROUP (ORDER BY actual_output_tokens) AS p50_7d,
    percentile_cont(0.95) WITHIN GROUP (ORDER BY actual_output_tokens) AS p95_7d,
    percentile_cont(0.99) WITHIN GROUP (ORDER BY actual_output_tokens) AS p99_7d,
    avg(actual_output_tokens)::REAL AS mean_7d,
    stddev_samp(actual_output_tokens)::REAL AS stddev_7d,
    count(*) AS sample_size_7d
  FROM events_30d
  WHERE ingest_at >= now() - interval '7 days'
  GROUP BY tenant_id, model, agent_id, prompt_class
)
INSERT INTO output_distribution_cache (
  tenant_id, model, agent_id, prompt_class,
  p50_7d, p95_7d, p99_7d, mean_7d, stddev_7d, sample_size_7d,
  p50_30d, p95_30d, p99_30d, mean_30d, stddev_30d, sample_size_30d,
  computed_at, aggregation_version
)
SELECT
  a30.tenant_id, a30.model, a30.agent_id, a30.prompt_class,
  a7.p50_7d, a7.p95_7d, a7.p99_7d, a7.mean_7d, a7.stddev_7d, a7.sample_size_7d,
  a30.p50_30d, a30.p95_30d, a30.p99_30d, a30.mean_30d, a30.stddev_30d, a30.sample_size_30d,
  now(), 'v1alpha1'
FROM agg_30d a30
LEFT JOIN agg_7d a7 USING (tenant_id, model, agent_id, prompt_class)
ON CONFLICT (tenant_id, model, agent_id, prompt_class)
  DO UPDATE SET
    p50_7d = EXCLUDED.p50_7d,
    -- ... all columns ...
    computed_at = EXCLUDED.computed_at;
```

### 4.2 為什麼兩個 window（7d + 30d）

- **30d** baseline：穩定的 P95 lookup for Strategy B
- **7d** signal：drift 偵測比較對象（current 7d vs prior 7d 與 historical 30d）

詳 §7 drift detection。

### 4.3 Materialized view 還是 INSERT/UPSERT

選 INSERT/UPSERT 而非 materialized view 因為：

- Postgres MV 不支援 partial refresh（per partition）
- UPSERT 允許 incremental update（只跑 active buckets，不重算 retired）
- ON CONFLICT 語意明確、可 audit（rows 的 `computed_at` 可追蹤新鮮度）

---

## §5. `output_distribution_cache` schema

```sql
CREATE TABLE output_distribution_cache (
    tenant_id            UUID NOT NULL,
    model                TEXT NOT NULL,
    agent_id             TEXT NOT NULL,
    prompt_class         TEXT NOT NULL,

    -- 7-day rolling window
    p50_7d               REAL,
    p95_7d               REAL,
    p99_7d               REAL,
    mean_7d              REAL,
    stddev_7d            REAL,
    sample_size_7d       INT CHECK (sample_size_7d IS NULL OR sample_size_7d >= 0),

    -- 30-day rolling window
    p50_30d              REAL,
    p95_30d              REAL,
    p99_30d              REAL,
    mean_30d             REAL,
    stddev_30d           REAL,
    sample_size_30d      INT CHECK (sample_size_30d IS NULL OR sample_size_30d >= 0),

    -- Metadata
    computed_at          TIMESTAMPTZ NOT NULL,
    aggregation_version  TEXT NOT NULL DEFAULT 'v1alpha1',

    PRIMARY KEY (tenant_id, model, agent_id, prompt_class)
);

CREATE INDEX output_distribution_cache_freshness_idx
  ON output_distribution_cache (computed_at);

CREATE INDEX output_distribution_cache_tenant_lookup_idx
  ON output_distribution_cache (tenant_id, model, agent_id, prompt_class);

-- Row-Level Security
ALTER TABLE output_distribution_cache ENABLE ROW LEVEL SECURITY;

CREATE POLICY output_distribution_cache_tenant_isolation
  ON output_distribution_cache
  USING (tenant_id = current_setting('app.current_tenant_id')::uuid);
```

`sample_size_7d` and `sample_size_30d` are COUNT-derived values and must
never be negative. Migrations pin this with CHECK constraints; the
aggregation writer only produces non-negative integers, and migration
smoke checks assert explicit negative inserts are rejected.

### 5.1 為什麼不 partition

- 量級 estimate：1000 tenants × 20 models × 50 agents × 7 classes = 7M rows max（per-tenant 7K rows）
- ON CONFLICT UPSERT 在 7M 表現良好
- Partition 帶來複雜度（partition pruning + cross-partition uniqueness）不值得

若未來 multi-tenant 規模超 50K tenants，再 partition by `(tenant_id % 256)` 或類似 hash partitioning。

---

## §6. (tenant, agent_id) run-length distribution

per `run-cost-projector-spec-v1alpha1.md` Signal 1 需要 「per-(tenant, agent_id) historical P95 run-length（steps per run）」。stats_aggregator 同 cycle 算這個 distribution：

```sql
WITH run_lengths AS (
  SELECT
    tenant_id,
    agent_id,
    run_id_mirror AS run_id,
    count(*) AS steps_in_run
  FROM canonical_events
  WHERE event_type = 'spendguard.audit.decision'
    AND ingest_at >= now() - interval '30 days'
    AND recorded_month >= DATE_TRUNC('month', now() - interval '30 days')::DATE
    AND agent_id IS NOT NULL
    AND run_id_mirror IS NOT NULL
  GROUP BY tenant_id, agent_id, run_id
)
INSERT INTO run_length_distribution_cache (
  tenant_id, agent_id,
  p50_steps_30d, p95_steps_30d, p99_steps_30d,
  mean_steps_30d, stddev_steps_30d, sample_size_30d,
  computed_at, aggregation_version
)
SELECT
  tenant_id, agent_id,
  percentile_cont(0.50) WITHIN GROUP (ORDER BY steps_in_run),
  percentile_cont(0.95) WITHIN GROUP (ORDER BY steps_in_run),
  percentile_cont(0.99) WITHIN GROUP (ORDER BY steps_in_run),
  avg(steps_in_run)::REAL,
  stddev_samp(steps_in_run)::REAL,
  count(*),
  now(), 'v1alpha1'
FROM run_lengths
GROUP BY tenant_id, agent_id
ON CONFLICT (tenant_id, agent_id)
  DO UPDATE SET
    p50_steps_30d = EXCLUDED.p50_steps_30d,
    p95_steps_30d = EXCLUDED.p95_steps_30d,
    -- ...
    computed_at = EXCLUDED.computed_at;
```

Table schema 與 `output_distribution_cache` 平行（同 RLS + 同新鮮度 index）。`sample_size_30d`
is also constrained as `CHECK (sample_size_30d IS NULL OR sample_size_30d >= 0)`;
it is derived from `count(*)` and is therefore a non-negative hard
invariant, not an advisory convention.

---

## §7. Drift detection

### 7.1 演算法

```
For each (tenant, model, agent_id, prompt_class) bucket:
  baseline_mean = mean_30d (excluding last 7 days; computed via separate query window)
  baseline_stddev = stddev_30d (same exclusion)
  current_mean = mean_7d

  z_score = (current_mean - baseline_mean) / baseline_stddev

  IF |z_score| > 2.0 AND sample_size_7d >= MIN_SAMPLES_FOR_ALERT:
    emit prediction_drift_alert
```

`MIN_SAMPLES_FOR_ALERT` default 100。理由：小樣本 7-day window 容易 false-positive。

### 7.2 Alert event schema

```yaml
# CloudEvent emitted to canonical_ingest
type: spendguard.audit.prediction_drift_alert.v1alpha1
source: spendguard://stats-aggregator/<tenant_id>
data:
  tenant_id: <uuid>
  model: <string>
  agent_id: <string>
  prompt_class: <string>
  baseline_period: { start: <ts>, end: <ts>, mean: <float>, stddev: <float> }
  current_period: { start: <ts>, end: <ts>, mean: <float> }
  z_score: <float>
  sample_size_7d: <int>
  sample_size_30d: <int>
  suggested_action: "review_predictor_baseline" | "retrain_strategy_c_plugin" | "investigate_agent_change"
```

Implementation reference: `services/stats_aggregator/src/drift_detector.rs` commit `f8dc34c` defines `PREDICTION_DRIFT_ALERT_EVENT_TYPE = "spendguard.audit.prediction_drift_alert.v1alpha1"`; calibration-report commit `8ee35ca` reads the same event type from `canonical_events.payload_json`.

Audit routing discipline: the `spendguard.audit.*` prefix is required so
canonical_ingest routes this event to ImmutableAuditLog. The source URI
format matches `build_drift_alert`: `spendguard://stats-aggregator/<tenant_id>`.
Older examples that used `stats-aggregator://<instance>` or
`spendguard.prediction.drift_alert` are non-authoritative draft text.

Signed + immutable per audit chain。

### 7.3 Suggested action 規則

| 條件 | suggested_action |
|---|---|
| z_score > 0（actual 高於 baseline）且 baseline 多月穩定 | `investigate_agent_change` |
| z_score < 0（actual 低於 baseline）且 Tier 1 shadow 也有 drift | `review_predictor_baseline`（vendor tokenizer 升級可能性） |
| z_score > 0 且 customer 有 Strategy C plugin | `retrain_strategy_c_plugin` |

純 heuristic；operator 仍要人工判斷。

### 7.4 與 `RUN_DRIFT_DETECTED` 區別

per `contract-dsl-spec-v1alpha2.md` §3.2 明確分離：

- `prediction_drift_alert`（本 spec）：**bucket-level**，hourly aggregation 後 emit；不阻擋 decisions
- `RUN_DRIFT_DETECTED`（contract-dsl v1alpha2）：**run-instance-level**，hot-path emit；可阻擋 decisions

兩者各有 audit row；calibration-report 同時聚合。

---

## §8. Scheduling

### 8.1 Default

每 1 hour 跑一次完整 aggregation cycle。對所有 tenants × all buckets 一次。

### 8.2 Per-tenant override

控制 plane API `POST /tenants/{id}/stats-aggregator-cadence { cadence_minutes: 30 }` 允許 tenant 拉高頻次（最低 15 min；保護 DB）。

每次 cycle 對該 tenant 多跑 1 次（按 tenant_id 過濾）；其他 tenants 仍走 1h cadence。

### 8.3 Aggregation cycle structure

```
cycle_start:
  ACQUIRE_ADVISORY_LOCK('stats_aggregator_singleton')  -- only one instance runs
  FOR EACH tenant_id:
    BEGIN
      run main aggregation query (per §4.1)
      run run-length aggregation query (per §6)
      run drift detection per bucket (per §7.1)
      emit drift_alert events as needed
      RECORD cycle metadata (computed_at, sample_sizes, alerts_emitted)
    COMMIT (per tenant; isolation)
  RELEASE_ADVISORY_LOCK
cycle_end
```

Per-tenant commit 確保部分 tenant 失敗不影響其他。

---

## §9. Multi-tenant isolation

### 9.1 機制

1. **Row-Level Security** on cache tables（per §5）
2. **Connection-level `app.current_tenant_id` setting** before any cache query
3. **No cross-tenant SQL**：每個 query 必含 explicit `tenant_id = $tenant_var`
4. **Aggregation cycle isolation**：每 tenant 自己 transaction；不 batch cross-tenant aggregation

### 9.2 Adversarial test

SLICE 06 acceptance 必含「cross-tenant injection attempt」test：

```sql
-- Attempt to read tenant A's cache as tenant B
SET app.current_tenant_id = '<tenant_b_uuid>';
SELECT * FROM output_distribution_cache WHERE tenant_id = '<tenant_a_uuid>';
-- Expected: 0 rows (RLS blocks)
```

---

## §10. Failure modes

| 場景 | 行為 |
|---|---|
| Aggregation cycle 失敗（query timeout / OOM）| 跳過本 cycle；emit `stats_aggregator_cycle_failed` event；下次 cycle 重試 |
| Single tenant aggregation 失敗 | 其他 tenants 繼續；該 tenant cache 變 stale |
| Cache 變 stale > 1h | `output_predictor` 偵測 `computed_at < now() - 1h` → fall through to cold-start chain（per `output-predictor-service-spec-v1alpha1.md` §7） |
| Advisory lock acquisition 失敗（其他 instance 仍 holding）| Skip this cycle；emit `stats_aggregator_skipped_lock_held` event |
| canonical_events 斷線 | Aggregation 失敗；emit failure event；下次重試 |
| Drift detection false positive | Operator review event；無誤殺 hot path 影響 |

---

## §11. SLO

### 11.1 Cache freshness

- p99 ≤ 1 hour（`now() - max(computed_at) < 1h` 99% of the time）
- p99.9 ≤ 2 hours（容忍偶發 cycle 跳過）
- 突破 → control plane alert

### 11.2 Aggregation latency

- Full cycle 結束時間 p99 ≤ 15 min over 100K daily events
- 突破 → 需 shard by tenant range

### 11.3 Drift alert latency

- bucket 進入 drift 至 alert event landed in canonical_ingest：p99 ≤ next cycle + cycle duration（i.e., ≤ 1h + 15min = 75 min worst case for hourly cadence）

---

## §12. GA prerequisites

於 `§0.3` 列出。本 spec 不重複。

---

## §13. Adoption history

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder — filled during Codex / panel adversarial review rounds per HANDOFF §9) |

---

## §14. Lock 後的下一步

1. SLICE 06 PR：stats_aggregator service skeleton + `output_distribution_cache` + `run_length_distribution_cache` tables + first aggregation cycle + RLS + adversarial test
2. SLICE 06 acceptance：cycle 完整跑 + drift detection 對 synthetic injection 命中 + cache freshness < 1h on demo
3. Per-tenant cadence API 推到 control plane（SLICE-extra；POC 階段全 tenant hourly）
4. Sharding by tenant range 推延到 50K tenant scale

---

*Document version: stats-aggregator-spec-v1alpha1 (DRAFT) | Drafted: 2026-05-29 | Critical surface: §1.2 NOT ML rationale; §4 aggregation queries; §7 drift detection algorithm; §9 multi-tenant isolation | Naming: stats_aggregator (formerly draft `predictor_trainer`, DEPRECATED) | Branch: `design/predictor-upgrade`*
