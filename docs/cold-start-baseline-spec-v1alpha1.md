# Cold-Start Baseline Specification — v1alpha1 (DRAFT)

> 📝 **Status: DRAFT** (writing in design phase on branch `design/predictor-upgrade`)
> **DRAFT → LOCKED criteria**: locks together with the predictor-upgrade spec set per `predictor-architecture-spec-v1alpha1.md` §0.2; additionally requires (a) `model_default_distribution.toml` covers ≥ 10 models × 7 prompt classes = 70 baseline distributions with cited sources, (b) L4 promotion threshold validated by simulation showing 30-sample threshold yields ≤ 5% prediction variance, (c) L3 federated design reviewed but implementation deferred per HANDOFF §5.4 (until ≥10 prod tenants opt-in).
> **Companion specs (this set)**: `predictor-architecture-spec-v1alpha1.md` (umbrella; pillar Q4 reasoning), `output-predictor-service-spec-v1alpha1.md` (consumer of layered fallback), `stats-aggregator-spec-v1alpha1.md` (provides L4 data), `audit-chain-prediction-extension-v1alpha1.md` (audit `cold_start_layer_used` column).
> **Pre-existing LOCKED dependencies**: `trace-schema-spec-v1alpha1.md` (CloudEvent type for L2 source citation events).
> **Compatibility policy**: alpha — TOML schema additive; `model_default_distribution.toml` versioned via top-level `schema_version` field; new model entries can be added without bumping spec version; class redefinition triggers spec bump.

---

## §0. Lock status & prerequisites

### 0.1 範圍

本 spec 定義 **cold-start 四層 fallback chain**：L1 → L2 → L3 → L4，從 least-specific 到 most-specific 倒著 lookup（output_predictor 從 L4 開始找 first 命中）。涵蓋：

1. 4 個 layer 的 semantics + lookup 規則 + 何時降級
2. 7 個 prompt class 定義 + classifier 規則
3. `model_default_distribution.toml` schema + 範例 + source citation 規則
4. L3 federated aggregate 完整 design（實作 deferred）
5. L4 promotion threshold（30 samples）的 statistical justification
6. L2 source corpus curation flow

**不在本 spec 範圍**：

- 四層 lookup 的 actual implementation in `output_predictor`（推給 `output-predictor-service-spec-v1alpha1.md` §7）
- `prompt_class_fingerprint` hashing algorithm details（推給 `output-predictor-service-spec-v1alpha1.md` §8）
- L4 數據如何 aggregate（推給 `stats-aggregator-spec-v1alpha1.md`）

### 0.2 DRAFT → LOCKED criteria

進入 LOCKED 之前下列 5 項必達成：

1. `model_default_distribution.toml` 初始版本 covers ≥ 10 models × 7 classes = 70 entries with full source citations
2. `docs/cold-start-baseline-sources.md` 文件化每筆 entry 引用的 public benchmark
3. SLICE 08 PR merged：TOML file + loader module + 7-class classifier + L1 hard fallback
4. Simulation：人工合成 (model, class) bucket with N samples → 確認 30-sample threshold 在 P95 prediction ≤ 5% variance
5. L3 federated aggregate design reviewed by Codex round 2 (although NOT implemented per `§5.4` deferral)

### 0.3 GA prerequisites

於 `predictor-architecture-spec-v1alpha1.md` §0.3 列出。本 spec 額外要求：

1. L2 TOML 每季 refresh cadence 確立 + 至少 1 次成功 refresh drill
2. L3 federated aggregate 設計通過第三方 security review（k-anonymity ≥5 + opt-in 機制）
3. L4 sample-size threshold per-class override 機制存在但 default 30 適用 ≥ 90% buckets

### 0.4 何時可能需要 v2

- 新增第 8 個 prompt class（極可能 multi-modal 相關）
- L3 federated 設計 break（GDPR / k-anonymity threshold 改變）
- L1 hard fallback formula 改變（罕見；觸發 v2）

---

## §1. Context (self-contained)

### 1.1 Cold start problem

新 tenant、新 agent、新 model、新 prompt class —— 任一組合的 `(tenant, model, agent_id, prompt_class_fingerprint)` bucket 在 `output_distribution_cache`（per `stats-aggregator-spec-v1alpha1.md`）無 samples 或 samples < 30 時，Strategy B P95 lookup 不可靠。

未處理結果（per HANDOFF §3.4 Q4 reasoning）：

- output_predictor 直接 collapse 到 Strategy A
- A 是 `min(max_tokens, context_window - input)` —— 是 ceiling，不是 typical case
- 對 customer experience：每 call reserve ceiling = 大量 budget 預占 = 並發 calls 馬上 starve
- First-impression demo：「我們的預測超保守」= 採購反感

### 1.2 為什麼是 4 層

HANDOFF §3.4 + Q4 reasoning lock 了 4 層 fallback。每層 cover 一個 specific 失效情境：

| Layer | Cover 的失效情境 | Lookup specificity |
|---|---|---|
| L4 | bucket 有足夠 samples → 直接用 customer's own distribution | Most specific |
| L3 | bucket samples 不足、但 federated aggregate 有 cross-customer samples for (model, class) | Per (model, class) |
| L2 | L3 不足、但 public benchmark 對 (model, class) 有 distribution | Per (model, class) baseline |
| L1 | 全部不足、unknown model → A 的同值（hard ceiling） | Generic |

### 1.3 v1alpha1 核心哲學

> **L4 是長期目標**；客戶實際 workload 是真相，但需要時間 accumulate。
>
> **L3 是 long-term winner**；多客戶 aggregate 比個別 tenant 小樣本更穩，但需要客戶體量 seed 才能啟用。
>
> **L2 是 ship-with-product 安全網**；hand-curated 公開 benchmark distribution；first-impression UX 全靠它。
>
> **L1 是 last resort**；等於 Strategy A 本身；不該被頻繁使用（per `predictor-architecture-spec-v1alpha1.md` §0.2 health invariant）。
>
> **Lookup 從 L4 開始**；most specific first；命中即用；不命中往上走。

---

## §2. The four-layer fallback (full semantics)

### 2.1 L4 — customer's own B distribution

**何時使用**：bucket `(tenant_id, model, agent_id, prompt_class)` 在 `output_distribution_cache` 有 `sample_size_30d >= 30`（per §6 promotion threshold）。

**Lookup**：直接讀 cache row `p50_30d / p95_30d / p99_30d`。

**Confidence**：高（>= 0.9）。

**Audit row**：`cold_start_layer_used = NULL`（per audit-chain extension §2.1 nullable rule —— L4 不算 cold start）。

### 2.2 L3 — federated cross-customer aggregate (deferred implementation)

**何時使用**：L4 不足 AND federated aggregate 對 `(model, prompt_class)` 有來自 ≥ 5 opt-in customers 的 samples。

**Lookup**：讀 federated aggregate 的 (model, class) 對應 distribution。

**Confidence**：中（0.5-0.7）。

**Audit row**：`cold_start_layer_used = 'L3'`。

**Status**：design now, build deferred until SpendGuard has ≥ 10 production tenants opt-in。Design 內容詳 §5。

### 2.3 L2 — public-benchmark-derived baseline

**何時使用**：L4 + L3 不足 AND `model_default_distribution.toml` 對 `(model, prompt_class)` 有 entry。

**Lookup**：讀 TOML 對應 entry 的 P50/P95/P99。

**Confidence**：中低（0.3-0.5；隨 sample_size 與 source 質量）。

**Audit row**：`cold_start_layer_used = 'L2'`。

### 2.4 L1 — hard fallback

**何時使用**：L4 + L3 + L2 全部不足 OR unknown model（dispatch table 無 entry）。

**Lookup**：`model.context_window - input_tokens` （= Strategy A 算式本身）。

**Confidence**：低（0.1）—— A 是 ceiling 不是 typical estimate。

**Audit row**：`cold_start_layer_used = 'L1'` + `tokenizer_tier = 'T3'`（typically；除非 model 在 dispatch 但 bucket 無樣本）。

### 2.5 Lookup 演算法

```rust
// Pseudocode for output_predictor::layered_b_lookup

fn lookup_b(req: PredictRequest) -> (Option<Distribution>, Layer) {
    // L4
    if let Some(row) = read_output_distribution_cache(req) {
        if row.sample_size_30d >= 30 {
            return (Some(row.into_distribution()), Layer::L4);
        }
    }

    // L3 (deferred; returns None if not enabled)
    if L3_ENABLED {
        if let Some(agg) = read_federated_aggregate(req.model, req.prompt_class) {
            if agg.contributing_customers >= 5 {
                return (Some(agg.into_distribution()), Layer::L3);
            }
        }
    }

    // L2
    if let Some(entry) = MODEL_DEFAULT_DISTRIBUTION.get(req.model, req.prompt_class) {
        return (Some(entry.into_distribution()), Layer::L2);
    }

    // L1
    (None, Layer::L1)  // caller (output_predictor) falls back to Strategy A value
}
```

L1 return None 是 explicit signal 給 output_predictor「沒有 B distribution；用 A 算式」。

---

## §3. 7 Prompt class buckets

### 3.1 Bucket 定義

| ID | Class name | 典型樣態 | Output token 規模 |
|---|---|---|---|
| 1 | `chat_short` | Single-turn small chat；user message < 100 tokens | typically 50-300 tokens |
| 2 | `chat_long` | Multi-turn conversation；context > 2K tokens | typically 200-1500 tokens |
| 3 | `code_gen` | Programming task；request 含 code context | typically 500-3000 tokens（output 高） |
| 4 | `summarization` | 大量 input → 短 output | typically 100-500 tokens（output 顯著小於 input） |
| 5 | `rag` | RAG with retrieved context；structured prompts | typically 100-800 tokens |
| 6 | `tool_calling` | Agent with tool definitions；output 含 tool_call JSON | typically 100-600 tokens |
| 7 | `vision` | Multi-modal request；image content | typically 100-800 tokens |

### 3.2 為什麼 7 而不是 5 / 10 / 20

- **不夠粗（5）**：合併 `code_gen` 與 `chat_long` 會讓 P95 失真（code 的 output 量明顯大）
- **過細（20+）**：每 class 樣本量被稀釋；L4 promotion 很慢；運維成本高
- **7 為甜蜜點**：覆蓋主要 use case；每 class 在多數 tenant 都能在 30 日內 accumulate ≥ 30 samples（per §6 simulation）

### 3.3 Classifier 規則摘要

Classifier 住在 `output-predictor-service-spec-v1alpha1.md` §8；本 spec 只給高層規則：

```
IF request.has_image_content: → vision
ELSE IF request.tool_definitions count > 0: → tool_calling
ELSE IF input_tokens > 8000 AND output_max_tokens < 1000: → summarization
ELSE IF input contains code markers (```, def, function, class): → code_gen
ELSE IF input contains retrieval markers (Document N:, [N], Source:): → rag
ELSE IF input_tokens > 1500 OR multi-turn (messages.length > 4): → chat_long
ELSE: → chat_short
```

Rule-based；可被 Codex round 2 攻擊與優化。

### 3.4 Class 不適合的時候

- 客戶在 audit 看到 `prompt_class = chat_short` 但實際是 multi-turn → 是 classifier rule false negative
- 兩個 class 邊界模糊（chat_long vs rag）：取 first-matched class（per §3.3 順序）

Class 是 statistical bucket，不是 strict semantic label。Calibration-report 對 misclassified rows 仍能聚合（per `calibration-report-spec-v1alpha1.md`）。

---

## §4. `model_default_distribution.toml` schema

### 4.1 檔案位置

`services/output_predictor/data/model_default_distribution.toml`

打包進 `output_predictor` binary（同 `spendguard-tokenizer` asset bundling pattern）。

### 4.2 Schema 範例

```toml
schema_version = "v1alpha1"
last_updated = "2026-05-29"
notes = "Refreshed per quarterly cadence; sources cited per (model, class) below"

[[entries]]
model = "gpt-4o-mini"
prompt_class = "chat_short"
p50 = 120
p95 = 280
p99 = 450
sample_size = 1500
source = "MT-Bench-2024-q4"
source_url = "https://mt-bench.example/2024-q4"
methodology_doc = "docs/cold-start-baseline-sources.md#mt-bench-q4"
confidence = 0.6

[[entries]]
model = "gpt-4o-mini"
prompt_class = "code_gen"
p50 = 850
p95 = 2400
p99 = 4100
sample_size = 800
source = "HumanEval+MBPP-2024"
source_url = "..."
methodology_doc = "docs/cold-start-baseline-sources.md#humaneval-mbpp"
confidence = 0.55

# ... 70+ entries covering 10+ models × 7 classes ...
```

### 4.3 Entry 必填欄位

- `model` — string；必須 match dispatch table model strings
- `prompt_class` — enum (7 classes)
- `p50 / p95 / p99` — integer tokens
- `sample_size` — integer；用於 confidence 計算與 weighting
- `source` — string；short identifier
- `source_url` — string；指向原 dataset / paper
- `methodology_doc` — string；指向 `docs/cold-start-baseline-sources.md` 對應 section
- `confidence` — float 0.0-1.0；reviewer-judged data quality

### 4.4 Loader

```rust
// crate: spendguard-cold-start-loader
pub struct ModelDefaultDistribution {
    entries: HashMap<(String, String), DistributionEntry>,  // (model, class) -> entry
}

impl ModelDefaultDistribution {
    pub fn load() -> Result<Self, LoadError>;  // embedded asset
    pub fn get(&self, model: &str, class: &str) -> Option<&DistributionEntry>;
    pub fn schema_version(&self) -> &str;
    pub fn last_updated(&self) -> &str;
}
```

啟動時 eager-load + sanity check（無 duplicate keys；所有 confidence in [0,1]）。

---

## §5. L3 federated aggregate design (deferred implementation)

### 5.1 為什麼 design now, build later

- 設計時間 affordable now；implementation 需要客戶體量
- 客戶 ≥ 10 prod tenants opt-in 之前實作 = 沒人能 contribute samples = 沒有 aggregate
- 提前 design lock 框架，避免後續客戶 lock-in 後改 schema

### 5.2 Aggregate schema

```sql
-- Lives in SpendGuard's central platform DB (not per-tenant DB).
CREATE TABLE federated_distribution_aggregate (
    aggregation_period   TEXT NOT NULL,        -- "2026-Q2"
    model                TEXT NOT NULL,
    prompt_class         TEXT NOT NULL,
    p50                  REAL NOT NULL,
    p95                  REAL NOT NULL,
    p99                  REAL NOT NULL,
    contributing_customers INT NOT NULL,        -- k-anonymity; minimum 5 to be queryable
    total_samples        INT NOT NULL,
    computed_at          TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (aggregation_period, model, prompt_class)
);

CREATE INDEX federated_dist_lookup_idx
  ON federated_distribution_aggregate (model, prompt_class, aggregation_period DESC);
```

### 5.3 Opt-in 機制

```yaml
# Per-tenant control plane setting
federated_contribution:
  enabled: true / false  # default false
  share_buckets: ["chat_short", "code_gen", ...]  # subset; default all
  consent_audit_event_id: <uuid>  # event recorded when tenant opted in
```

只有 `enabled = true` 的 tenants 的 buckets 進 federated aggregate computation。

### 5.4 k-anonymity ≥ 5

任何 (model, class) aggregate 對外 query 前必驗證 `contributing_customers >= 5`。<5 → return null（視為 L3 unavailable）→ fallback to L2。

防止單一 customer 透過反查推斷其他 customer 的 distribution。

### 5.5 Privacy / data minimization

Federated aggregate **只**收：

- (model, class, P50, P95, P99, sample_size, period)

**不收**：

- Prompt content / token sequences
- Individual decision_id / run_id / tenant_id（aggregate 後 anonymize）
- Customer-specific bucket lookups

GDPR 角度：no personal data；aggregate 是 statistical fact，不可逆 derive individual。

### 5.6 Build trigger

Implementation slice（命名暫定 SLICE_extra_L3，post-launch）觸發條件：

- SpendGuard 有 ≥ 10 production tenants
- ≥ 5 tenants 已 opt-in `federated_contribution.enabled = true`
- Privacy review 通過

---

## §6. L4 promotion threshold (30 samples)

### 6.1 為什麼 30

統計學常識 + simulation：

- < 30 samples：P95 estimate variance > 30%（small-sample bias）
- 30-100 samples：P95 variance ~10-20%（acceptable）
- > 100 samples：P95 variance < 10%（converged）

30 是「足夠穩定 + 不太慢」的甜蜜點。對中等流量 customer agent（10 calls / day），30 samples = 3 天 accumulate；可接受。

### 6.2 Per-class override

某些 high-variance class（如 `code_gen`）可能需要 ≥ 50 samples 才穩定。本 spec 規定 default 30；`output_distribution_cache` row 加 `confidence` 欄位 derived from `sample_size`（per `stats-aggregator-spec-v1alpha1.md` §5 schema）以表達 unstable buckets。

`output_predictor` 在 L4 lookup 時可選擇「sample_size < 50 且 class in HIGH_VARIANCE_CLASSES → fall to L3/L2」（per future SLICE）。

### 6.3 Simulation validation

SLICE 08 acceptance 必含：人工 inject 10 / 20 / 30 / 50 / 100 samples 到 cache → 對應的 P95 estimate variance vs ground truth。30-sample 變異率 ≤ 5% required。

---

## §7. L2 source curation flow

### 7.1 Source 列表（initial 10 models × 7 classes）

| Model family | 主要 source | 補充 |
|---|---|---|
| GPT-4o / GPT-4o-mini | MT-Bench-2024-q4 | HumanEval + MBPP for code_gen |
| GPT-4-turbo | Same | Same |
| Claude 3.5 Sonnet / Haiku / Opus | LongBench-2024 + MT-Bench | HumanEval for code_gen |
| Gemini 1.5 Pro / Flash | MMLU-Pro + MT-Bench | Same |
| Cohere Command | MT-Bench subset | （TBD per available data） |
| Llama 3 70B | OpenBench | Same |

### 7.2 Refresh cadence

- 每季（Q1 / Q2 / Q3 / Q4）maintenance window
- Trigger：(a) new model release with non-trivial distribution shift；(b) drift_alert 對 L2 baseline 的 (model, class) buckets 持續觸發 → 表示 baseline 過時
- Refresh PR 必含 source citation update + diff explanation + reviewer approval

### 7.3 Source quality bar

- Sample size ≥ 500 per (model, class)
- Public dataset with reproducible methodology
- Reviewer agrees with class assignment

不符 → entry confidence ≤ 0.3 + 明確 note；不 ship 進 default。

### 7.4 `docs/cold-start-baseline-sources.md`

新建檔案，每個 entry 對應一個 section：

```markdown
# Cold-start baseline sources

## MT-Bench-2024-q4

- URL: https://mt-bench.example/2024-q4
- Methodology: 80 prompts × 6 categories; 1500 model responses
- Data extraction: response length percentiles computed from raw outputs
- Caveats: skewed toward English; multi-lingual underrepresented
- Class mapping: chat_short = single-turn; chat_long = multi-turn
- Last refreshed: 2026-05-15
- Maintainer: <handle>
```

每筆 entry 在 TOML 引用此檔對應 section（per §4.3 `methodology_doc` 欄位）。

---

## §8. Failure modes

| 場景 | 行為 |
|---|---|
| TOML asset 壞掉 / signature 不對 | output_predictor refuse-to-start；fail-fast at boot |
| TOML schema_version 不認 | refuse-to-start |
| Specific (model, class) entry missing | L2 lookup return None → fallback to L1（per §2.5） |
| L4 cache row 存在但 sample_size < 30 | 不算命中；繼續 L3 → L2 lookup |
| L3 enabled 但 contributing_customers < 5 | L3 lookup return None；fallback to L2 |
| Classifier mis-classify | calibration-report 對 misclass rows 仍能 group；下次 classifier upgrade 補救 |

---

## §9. Audit chain impact

per `audit-chain-prediction-extension-v1alpha1.md` §2.1 audit_outbox column `cold_start_layer_used`：

| Layer hit | `cold_start_layer_used` value |
|---|---|
| L4 | NULL（not cold start） |
| L3 | `'L3'` |
| L2 | `'L2'` |
| L1 | `'L1'` |

CloudEvent proto mirror at tag 310 per audit-chain extension §3.2。

`prediction_strategy_used` 與 `cold_start_layer_used` 共同辨識「這個 prediction 是 B fallback through which layer」。

---

## §10. GA prerequisites

於 `§0.3` 列出。本 spec 不重複。

---

## §11. Adoption history

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder — filled during Codex / panel adversarial review rounds per HANDOFF §9) |

---

## §12. Lock 後的下一步

1. ✅ SLICE 08 PR shipped (2026-05-30, branch `slice/SLICE_08_cold_start_baseline_table`):
   - `services/output_predictor/data/model_default_distribution.toml` (70 entries, v1alpha1)
   - `docs/cold-start-baseline-sources.md` (source citations per §7.4)
   - `services/output_predictor/src/cold_start_loader.rs` (Layer A sha256 + Layer B fixture cross-check + per-entry sanity)
   - L2 wired in `strategy_b::compute_b` + `cold_start_layer_used` audit column populated per §7.1 truth table
   - Simulation validation: P95 estimate at N=30 within spec §6.3 acceptance gate
2. ✅ SLICE 08 acceptance: 7-class classifier (SLICE_06); 70 TOML entries with sources
3. L3 federated aggregate schema review (Codex round 2); implementation deferred per §5.6 trigger
4. Quarterly refresh playbook 文檔化 + first refresh drill 排程 (Q3 2026 first window)

---

*Document version: cold-start-baseline-spec-v1alpha1 (DRAFT) | Drafted: 2026-05-29 | Critical surface: §2 four-layer fallback;  §3 7 prompt classes;  §5 L3 federated design (deferred build);  §6 30-sample promotion threshold;  §7.1 initial source curation | Branch: `design/predictor-upgrade`*
