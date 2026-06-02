# Output Predictor Service Specification — v1alpha1 (DRAFT)

> 📝 **Status: DRAFT** (writing in design phase on branch `design/predictor-upgrade`)
> **DRAFT → LOCKED criteria**: locks together with the predictor-upgrade spec set per `predictor-architecture-spec-v1alpha1.md` §0.2; additionally requires (a) `Predict` hot-path p99 ≤ 15ms（含 plugin call if enabled）, (b) Strategy A 永遠算成 100% of the time（無 failure mode 讓 A null）, (c) cold-start chain L4→L3→L2→L1 fully exhaustively handled per `cold-start-baseline-spec-v1alpha1.md` §2.5 lookup algorithm.
> **Companion specs (this set)**: `predictor-architecture-spec-v1alpha1.md` (umbrella; defines A/B/C role), `tokenizer-service-spec-v1alpha1.md` (provides input_tokens), `stats-aggregator-spec-v1alpha1.md` (provides B distribution cache), `cold-start-baseline-spec-v1alpha1.md` (L1/L2/L3/L4 fallback), `output-predictor-plugin-contract-v1alpha1.md` (Strategy C delegated mode), `audit-chain-prediction-extension-v1alpha1.md` (audit columns).
> **Pre-existing LOCKED dependencies**: `sidecar-architecture-spec-v1alpha1.md` (§5 mTLS internal transport), `contract-dsl-spec-v1alpha1.md` (§14 latency SLO heritage), `proto/spendguard/common/v1/common.proto` (UnitRef / BudgetClaim shared types).
> **Compatibility policy**: alpha — proto3 additive evolution; strategy selector may add new policies via `contract-dsl-spec-v1alpha2.md` `prediction_policy` enum extensions; lookup-cache invalidation strategies versioned.

---

## §0. Lock status & prerequisites

### 0.1 範圍

本 spec 定義 **output_predictor service**：

1. gRPC `Predict` API — 對單一 decision boundary 計算 Strategy A / B / C 三值 + selector
2. Strategy A 實作（純 lookup）
3. Strategy B 實作（讀 stats_aggregator cache + cold-start L4→L3→L2→L1 fallback chain）
4. Strategy C 實作（透過 plugin contract delegate）
5. Prompt class classifier + fingerprint hashing
6. Selector：根據 `prediction_policy` 決定 `prediction_strategy_used`
7. Failure modes + latency SLO

**不在本 spec 範圍**：

- 預測值如何用作 reservation（推給 `predictor-architecture-spec-v1alpha1.md` §5 policy matrix；本 spec 只算與選 strategy）
- 預測值如何進 audit chain（推給 `audit-chain-prediction-extension-v1alpha1.md`）
- Plugin gRPC contract details（推給 `output-predictor-plugin-contract-v1alpha1.md`）
- B distribution 如何被算出（推給 `stats-aggregator-spec-v1alpha1.md`）
- Cold-start baseline 內容（推給 `cold-start-baseline-spec-v1alpha1.md`）

### 0.2 DRAFT → LOCKED criteria

進入 LOCKED 之前下列 6 項必達成：

1. SLICE 06 + SLICE 07 + SLICE 08 三 slices 全 merged
2. `Predict` p99 ≤ 15ms（含 plugin call if enabled；不含則 ≤ 5ms）
3. Strategy A 在 0 failure mode catch-all：即便所有依賴失敗 A 仍算得出
4. Cold-start chain 對 (model, class) 兩兩 combination 全部命中 L1（即無 entry 也走過）
5. Classifier 對 100 行手動 labeled samples 命中率 ≥ 90%
6. Multi-strategy parallel computation 互不阻擋（A 不等 B；B 不等 C）

### 0.3 GA prerequisites

於 `predictor-architecture-spec-v1alpha1.md` §0.3 列出。本 spec 額外要求：

1. 5 tenants × 7 classes × 30 日 production usage 證實 selector 對 `STRICT_CEILING` / `EMPIRICAL_RUN_CEILING` 兩個 policy 各自正確
2. Classifier 對 production traffic 100K samples × 7 classes 分布合理（無 class < 1% 也無 class > 70%）
3. Strategy C plugin path 在至少 1 design partner 跑 30 日 healthy

### 0.4 何時可能需要 v2

- 新增 strategy（D/E）—— 罕見；觸發 v2
- Classifier 改 ML-based（可能；觸發 v2 因為要決定 ML lifecycle 議題）
- Predict 改 batch（per-call → per-batch）

---

## §1. Context (self-contained)

### 1.1 為什麼有這份 spec

整套 predictor upgrade 的 hot-path 計算中樞。Tokenizer 算 input；stats_aggregator 算 historical distribution；run_cost_projector 算 per-run projection；customer plugin 算 ML prediction。**output_predictor 把 A / B / C 並列計算、選 strategy、輸出給 sidecar**。

無 output_predictor → tokenizer / stats / plugin 等 component 各自孤立 → sidecar 無法 consume → reservation decision 無 prediction metadata 進 audit chain。

### 1.2 在 hot path 的位置

```
egress_proxy or sidecar adapter
  ↓ DecisionRequest with body
sidecar
  ↓ tokenize → input_tokens + tokenizer_tier
sidecar → output_predictor.Predict(req)   ← 本 spec
  ↓ {A, B, C, strategy_used, confidence, cold_start_layer}
sidecar → run_cost_projector.Project()
  ↓
ledger.ReserveSet (with rich BudgetClaim)
```

`Predict` 是 sidecar 每 decision 的一次同步 gRPC call。

### 1.3 v1alpha1 核心哲學

> **A 永遠算成**：純 lookup，無 failure；reservation 安全網。
>
> **B / C 並行算**：parallelize；不互相 block；最後 selector 決定 chosen。
>
> **Selector 是 policy-driven**：`prediction_policy` 決定 reservation 用哪個（per `predictor-architecture-spec-v1alpha1.md` §5）；audit row 永遠寫 chosen + 各自 raw value。
>
> **Cold-start chain L4→L3→L2→L1 嚴格按順序**：first match wins；不混合不投票。
>
> **Classifier 是 rule-based**：deterministic；可被 reviewer 攻擊與優化；未來改 ML 是 v2 大議題。

---

## §2. Service surface

### 2.1 gRPC proto

新檔案：`proto/spendguard/output_predictor/v1/predictor.proto`

```protobuf
syntax = "proto3";
package spendguard.output_predictor.v1;
import "google/protobuf/timestamp.proto";

service OutputPredictor {
  // Hot-path: compute A/B/C predictions + select strategy.
  rpc Predict(PredictRequest) returns (PredictResponse);
}

message PredictRequest {
  string tenant_id = 1;
  string model = 2;
  string agent_id = 3;
  string prompt_class = 4;            // server-classified externally + passed in by sidecar

  int64 input_tokens = 5;             // from tokenizer service
  int64 max_tokens_requested = 6;     // request.max_tokens or 0
  int64 model_context_window = 7;     // from model_context_window lookup

  // Active policy from contract evaluation.
  string prediction_policy = 8;       // "STRICT_CEILING" | ... per contract-dsl-v1alpha2 §4

  // Optional fields for plugin path (Strategy C).
  PluginContextFeatures plugin_features = 9;

  // Identity for tracing/audit.
  string decision_id = 10;
  string run_id = 11;
  string prompt_class_fingerprint = 12;
}

message PluginContextFeatures {
  int32 conversation_depth = 1;
  bool has_tool_calls = 2;
  bool has_system_message = 3;
  int32 num_tool_definitions = 4;
  string user_role_hint = 5;
}

message PredictResponse {
  // Strategy A — always computed; never null.
  int64 predicted_a_tokens = 1;

  // Strategy B — null when bucket insufficient OR cold-start chain returns L1.
  optional int64 predicted_b_tokens = 2;

  // Strategy C — null when no plugin configured / plugin failed / out-of-range.
  optional int64 predicted_c_tokens = 3;

  // Selector decision (per §6).
  string reserved_strategy = 4;          // "A" | "B" | "C"
  string prediction_strategy_used = 5;   // "A" | "B" | "C" — may differ from reserved

  // Confidence + sample (from B's cache row or C's plugin).
  optional float confidence = 6;          // null when A only
  optional int32 sample_size = 7;

  // Cold-start chain output.
  optional string cold_start_layer_used = 8;  // "L1" | "L2" | "L3" | "L4" | empty (not cold-start)

  // Latency tracking.
  int64 a_latency_ns = 10;
  int64 b_latency_ns = 11;
  int64 c_latency_ns = 12;
  int64 total_latency_ns = 13;

  string classifier_version = 14;
  string fingerprint_version = 15;
  string prompt_class_fingerprint_used = 16;

  // POST_GA_07: echoed request prediction policy so API consumers can
  // reconstruct policy behavior without joining audit rows.
  string prediction_policy_used = 17;
}
```

### 2.2 Deployment

集中 service（gRPC over mTLS internal transport per Sidecar §5）。Sidecar 每 decision 一次 call。

`output_predictor` 自己對 cache lookup 用 connection pool；對 plugin call 用 per-(tenant) circuit breaker（per `output-predictor-plugin-contract-v1alpha1.md` §6）。

POST_GA_07 adds a process-local per-tenant Predict RPC token bucket
before cache, database, or plugin work. Defaults are
`predict_rate_limit_per_tenant_per_second = 1000` per pod and
`predict_rate_limit_tenant_capacity = 4096` retained tenant buckets per
pod; setting the rate to `0` disables throttling for emergency rollback.
A tenant overrun returns gRPC `RESOURCE_EXHAUSTED`, logs the tenant id in
structured logs, and increments the no-label monotonic counter
`spendguard_output_predictor_rate_limited_total`. In multi-replica
deployments, effective service-wide tenant capacity is approximately
`per_pod_limit * ready_replicas` unless the deployment adds sticky
tenant routing or an external shared limiter.

POST_GA_09 bounds Strategy C plugin-bound identifiers before cache,
database, or plugin work: `decision_id <= 128` bytes and
`prompt_class_fingerprint <= 128` bytes. Over-limit requests fail with
gRPC `INVALID_ARGUMENT`; SpendGuard does not truncate these fields
because they are audit/join identifiers.

### 2.3 Hot path 並行模式

```rust
// Pseudocode for handler

async fn predict(req: PredictRequest) -> PredictResponse {
    let (a, b, c) = tokio::join!(
        compute_a(&req),          // sync, < 100us
        compute_b(&req),           // async; cache lookup + cold-start chain
        compute_c(&req),           // async; plugin call (50ms hard cap)
    );

    let (reserved_strategy, prediction_strategy_used) =
        select_strategy(&req.prediction_policy, &a, &b, &c);

    PredictResponse {
        predicted_a_tokens: a,
        predicted_b_tokens: b.map(|x| x.value),
        predicted_c_tokens: c.map(|x| x.value),
        reserved_strategy: reserved_strategy.to_string(),
        prediction_strategy_used: prediction_strategy_used.to_string(),
        prediction_policy_used: req.prediction_policy.clone(),
        confidence: c.as_ref().or(b.as_ref()).map(|x| x.confidence),
        sample_size: c.as_ref().or(b.as_ref()).and_then(|x| x.sample_size),
        cold_start_layer_used: b.as_ref().and_then(|x| x.layer.clone()),
        // ...
    }
}
```

A / B / C 並行算；selector 在三值齊備後決定。

---

## §3. Strategy A — max_tokens-based ceiling

### 3.1 算式

```
predicted_a_tokens = min(
    max_tokens_requested if max_tokens_requested > 0 else INFINITY,
    model_context_window - input_tokens
)
```

純 in-memory 算式；no I/O；< 100us。

### 3.2 `model_context_window` table

新增 lookup table（or hardcoded constants per `output_predictor` build）：

```toml
# services/output_predictor/data/model_context_window.toml
[[entries]]
model = "gpt-4o"
context_window = 128000

[[entries]]
model = "gpt-4o-mini"
context_window = 128000

[[entries]]
model = "claude-3-5-sonnet-20240620"
context_window = 200000

# ... etc ...
```

Asset bundled with binary（per same pattern as `model_default_distribution.toml`）。Refresh 同 cadence。

### 3.3 Unknown model 行為

若 `model_context_window` 對 model 無 entry：

- 用 conservative default `8000` tokens（per OpenAI's old gpt-3 standard；夠用於未知 small model）
- Emit metric `output_predictor_unknown_context_window{ model="..." }`
- 仍能算 A（永遠 succeed）

### 3.4 A 的 invariant

A **永遠** > 0 且 ≤ `model_context_window`（per default）。無 fail case。Reservation 的 safety floor 來自 A 永遠 callable。

---

## §4. Strategy B — SQL P95 lookup

### 4.1 主路徑

```
1. Lookup output_distribution_cache for (tenant, model, agent_id, prompt_class)
2. IF row exists AND sample_size_30d >= 30:
   → return PredictionB(p95_30d, confidence=derive(sample_size), layer=None /* L4 */)
3. ELSE:
   → enter cold-start chain (§7)
```

### 4.2 Cache lookup 實作

`output_predictor` 持 read-only connection pool 對 ledger Postgres（per `stats-aggregator-spec-v1alpha1.md` §5 cache table location）。Lookup query：

```sql
SELECT p50_30d, p95_30d, p99_30d, sample_size_30d, computed_at
FROM output_distribution_cache
WHERE tenant_id = $1
  AND model = $2
  AND agent_id = $3
  AND prompt_class = $4
  AND computed_at > now() - interval '2 hours';   -- staleness gate
```

Staleness > 2h → 視為 cache miss（fall to cold-start）。

### 4.3 In-memory cache

`output_predictor` 自己對 cache row 在 RAM keep `~5min` TTL（防止 hot key 對 Postgres 灼壓）。Key = bucket tuple；value = full row。

### 4.4 Confidence derivation

```
confidence = min(1.0, sample_size_30d / 200.0)
// 200 samples = full confidence
// 30 samples = 0.15 baseline confidence
// 100 samples = 0.5 confidence
```

Heuristic；calibration-report 後可 tune。

### 4.5 B 的 invariant

B 是 nullable —— cache miss + cold-start L1 → B null。`output_predictor` 寫 `optional int64 predicted_b_tokens = null` 給 sidecar。

---

## §5. Strategy C — delegated plugin

### 5.1 Entry condition

`output_predictor` lookup `(tenant_id → plugin_endpoint, mTLS cert_id)` from control plane cache. Hit → call plugin per `output-predictor-plugin-contract-v1alpha1.md` proto. Miss → C = null（tenant 沒配 plugin）。

POST_GA_09 endpoint-cache resilience:

- Cache miss/stale reload uses tenant-scoped singleflight, so many
  concurrent requests for the same tenant collapse into one control
  plane DB lookup. True misses and DB-error stale serves are shared for
  a 1s reload-result backoff so queued callers do not take turns
  re-hitting the DB. Different tenants use different locks.
- If the control plane DB lookup fails, an enabled cached endpoint may
  be served stale for at most 300s. Older stale entries fall back to B.
- `enabled = FALSE` remains a kill switch even during DB errors; stale
  disabled entries return C null rather than calling the plugin.

### 5.2 Call mechanics

```rust
// Pseudocode
async fn compute_c(req: &PredictRequest) -> Option<PredictionC> {
    let endpoint = endpoint_cache.get(&req.tenant_id)?;
    if circuit_breaker.is_open(&req.tenant_id) {
        emit_metric("plugin_circuit_open");
        return None;
    }

    let plugin_req = build_plugin_request(req);
    let result = timeout(Duration::from_millis(50), endpoint.client.predict(plugin_req)).await;

    match result {
        Ok(Ok(resp)) => {
            if !validate_response(&resp, req.model_context_window) {
                circuit_breaker.record_failure(&req.tenant_id);
                emit_metric("plugin_invalid_response");
                return None;
            }
            circuit_breaker.record_success(&req.tenant_id);
            Some(PredictionC {
                value: resp.predicted_output_tokens,
                confidence: resp.confidence,
                sample_size: Some(resp.sample_size),
            })
        }
        Ok(Err(grpc_err)) => {
            circuit_breaker.record_failure(&req.tenant_id);
            emit_metric_with_reason("plugin_grpc_error", grpc_err.code());
            None
        }
        Err(_timeout) => {
            circuit_breaker.record_failure(&req.tenant_id);
            emit_metric("plugin_timeout");
            None
        }
    }
}
```

### 5.3 Validation rules

per `output-predictor-plugin-contract-v1alpha1.md` §5.1 全部錯誤情境視為 None；no fallback to A（B 已經有 fallback chain；C 沒命中就 None）。

### 5.4 Plugin 失敗 vs C 不可達

兩者都 → C null。Sidecar / audit 看 `predicted_c_tokens` null + `prediction_strategy_used != 'C'` 即可區分（無需另外 expose plugin error reason in audit row v1alpha1 schema）。

---

## §6. Strategy selector

### 6.1 演算法

```rust
fn select_strategy(
    policy: &str,
    a: i64,
    b: Option<i64>,
    c: Option<i64>,
) -> (reserved: Strategy, prediction_used: Strategy) {
    match policy {
        "STRICT_CEILING" => (Strategy::A, prefer_c_then_b_then_a(c, b)),
        "EMPIRICAL_RUN_CEILING" => {
            let chosen = prefer_c_then_b_then_a(c, b);
            (Strategy::A, chosen)
        }
        "ADAPTIVE_CEILING" => {
            // Reservation switches based on budget remaining (caller has that context)
            // For our purposes: reservation = B/C if available else A
            let chosen = prefer_c_then_b_then_a(c, b);
            (chosen, chosen)
        }
        "SHADOW_ONLY" => (Strategy::A, Strategy::A),
        _ => (Strategy::A, Strategy::A),  // default conservative
    }
}

fn prefer_c_then_b_then_a(c: Option<i64>, b: Option<i64>) -> Strategy {
    if c.is_some() { Strategy::C }
    else if b.is_some() { Strategy::B }
    else { Strategy::A }
}
```

### 6.2 Reserved vs prediction_used 為什麼不同欄位

在 `STRICT_CEILING` 下：

- `reserved_strategy = 'A'`（policy 強制）
- `prediction_strategy_used = 'B'` 或 `'C'`（如果有命中）—— 紀錄「我們 *會* 用 B/C 預測 if policy 允許」

calibration-report 後可 backtest 「如果客戶 switched to EMPIRICAL_RUN_CEILING 會節省多少 budget overhead」—— 兩個欄位讓這個分析 可能。

### 6.3 Audit row 寫法

per `audit-chain-prediction-extension-v1alpha1.md` §2.1：

- `predicted_a_tokens = a` 永遠寫
- `predicted_b_tokens = b` （null 允許）
- `predicted_c_tokens = c` （null 允許）
- `reserved_strategy` = selector 第一個值
- `prediction_strategy_used` = selector 第二個值
- `prediction_policy_used` = `req.prediction_policy`

---

## §7. Cold-start chain L4 → L3 → L2 → L1

per `cold-start-baseline-spec-v1alpha1.md` §2.5 lookup algorithm：

```rust
async fn compute_b(req: &PredictRequest) -> Option<PredictionB> {
    // L4: cache row with sufficient samples
    if let Some(row) = cache_lookup(req).await {
        if row.sample_size_30d >= 30 {
            return Some(PredictionB::from_cache_row(row, Layer::L4));
        }
    }

    // L3: federated aggregate (deferred build; returns None for now)
    if L3_ENABLED {
        if let Some(agg) = federated_lookup(&req.model, &req.prompt_class).await {
            if agg.contributing_customers >= 5 {
                return Some(PredictionB::from_federated(agg, Layer::L3));
            }
        }
    }

    // L2: model_default_distribution.toml
    if let Some(entry) = MODEL_DEFAULT_DIST.get(&req.model, &req.prompt_class) {
        return Some(PredictionB::from_toml_entry(entry, Layer::L2));
    }

    // L1: hard fallback — B returns None; selector falls to A
    None  // caller writes cold_start_layer_used = 'L1' in audit
}
```

### 7.1 Audit `cold_start_layer_used` 寫法

| B 結果 | `cold_start_layer_used` |
|---|---|
| `Some` from L4 | NULL |
| `Some` from L3 | `'L3'` |
| `Some` from L2 | `'L2'` |
| `None` (L1) | `'L1'` |

---

## §8. Prompt class classifier + fingerprinting

### 8.1 Classifier rules (full, per `cold-start-baseline-spec-v1alpha1.md` §3.3 摘要)

```rust
fn classify(messages: &[Message], request: &Request) -> &'static str {
    // 1. Vision (highest priority)
    if request.has_image_content() { return "vision"; }

    // 2. Tool calling
    if !request.tool_definitions.is_empty() { return "tool_calling"; }

    // 3. Summarization (large input, small output cap)
    let input_tokens = estimate_input_tokens(messages);
    if input_tokens > 8000 && request.max_tokens.map_or(false, |m| m < 1000) {
        return "summarization";
    }

    // 4. Code generation
    if contains_code_markers(messages) { return "code_gen"; }

    // 5. RAG
    if contains_retrieval_markers(messages) { return "rag"; }

    // 6. Long chat
    if input_tokens > 1500 || messages.len() > 4 { return "chat_long"; }

    // 7. Short chat
    "chat_short"
}

fn contains_code_markers(messages: &[Message]) -> bool {
    for m in messages {
        if m.content.contains("```") || regex!(r"\b(def|function|class)\s+\w+").is_match(&m.content) {
            return true;
        }
    }
    false
}

fn contains_retrieval_markers(messages: &[Message]) -> bool {
    for m in messages {
        if regex!(r"(Document \d+:|Source: |^\[\d+\] )").is_match(&m.content) {
            return true;
        }
    }
    false
}
```

### 8.2 Fingerprint hash

```rust
fn prompt_class_fingerprint(messages: &[Message], model: &str, class: &str) -> String {
    // Hash over canonicalized template structure (NOT content)
    let canonical = format!("v1:{}|{}|{}", class, model, messages.len());
    format!("v1:{:x}", sha256(canonical.as_bytes()))
}
```

Fingerprint 永遠 prefix `v1:`，便於後續 v2 classifier upgrade 識別。Aggregator key 用 `class` 本身（7 enum），不用 fingerprint —— fingerprint 是 audit identifier，class 是 aggregation bucket。

### 8.3 Classifier 由 sidecar 跑

Sidecar 在 decision pre-stage 跑 classifier，把 class string 放入 `PredictRequest.prompt_class`. `output_predictor` 不重跑（避免 hot-path duplicate work）。

### 8.4 Classifier upgrade 路徑

v1alpha1 是 rule-based；v2 可能引入 ML classifier。Upgrade 走 v1beta1 contract：classifier version 寫進 audit row 的 `prompt_class_fingerprint` 前綴；舊 row 仍可被 query。

---

## §9. Failure modes

| 場景 | 行為 |
|---|---|
| stats_aggregator cache unreachable | B → fall to cold-start chain (L2/L1) |
| Cache row stale > 2h | B → fall to cold-start chain |
| Plugin endpoint不可達 / circuit breaker open | C = null；B 仍算 |
| Plugin returns illegal value | C = null + circuit breaker count failure |
| `model_default_distribution.toml` lookup miss | L2 → L1（B null） |
| `model_context_window` lookup miss | A 用 8000 default + emit metric |
| Tenant exceeds Predict RPC rate limit | Return gRPC `RESOURCE_EXHAUSTED`, log tenant id in structured logs, and increment no-label `spendguard_output_predictor_rate_limited_total` |
| Classifier mis-classify | 仍走流程；calibration-report 後驗 |
| All B and C fail | A 仍永遠算成 → selector 落到 A |
| Predict RPC timeout from sidecar | sidecar 走 conservative fallback（per `sidecar-architecture-spec-v1alpha1.md` §7 fail-safe path）—— typically A only |

---

## §10. Audit chain impact

per `audit-chain-prediction-extension-v1alpha1.md` §2.1，每 Predict response 對應 audit row 寫入：

- `predicted_a_tokens` ← `response.predicted_a_tokens`（always）
- `predicted_b_tokens` ← `response.predicted_b_tokens`（nullable）
- `predicted_c_tokens` ← `response.predicted_c_tokens`（nullable）
- `reserved_strategy` ← `response.reserved_strategy`
- `prediction_strategy_used` ← `response.prediction_strategy_used`
- `prediction_policy_used` ← `response.prediction_policy_used`（must equal the active contract policy for the decision）
- `prediction_confidence` ← `response.confidence`
- `prediction_sample_size` ← `response.sample_size`
- `cold_start_layer_used` ← `response.cold_start_layer_used`

CloudEvent proto mirror 對應 tags 300-310 per audit-chain extension §3.2。

---

## §11. SLO

### 11.1 Predict p99 budget

| 情境 | p99 budget |
|---|---|
| A only (B + C both unconfigured) | ≤ 1 ms |
| A + B (no plugin) | ≤ 5 ms |
| A + B + C (plugin enabled) | ≤ 15 ms |

預算分配：A < 1ms；B cache lookup < 3ms；C plugin call < 50ms (hard cap)；selector + serialize < 1ms。並行算讓總時間 ~= max(individual)。

### 11.2 從 sidecar 角度

Sidecar Contract §14 50ms total budget：

- tokenizer < 1ms
- output_predictor < 15ms
- run_cost_projector < 5ms
- contract evaluator + ledger reserve + audit write < 29ms
- 合計 < 50ms

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

1. SLICE 06 PR：output_predictor skeleton + Strategy A + B with cold-start L4/L2/L1 + classifier + initial `model_context_window.toml`
2. SLICE 07 PR：Strategy C delegated mode + plugin contract + per-tenant circuit breaker + control plane endpoint registration
3. SLICE 08 PR：`model_default_distribution.toml` populated; cold-start L2 entries; classifier validation drill
4. Production rollout per design partner;  calibration-report consumed continuously

---

*Document version: output-predictor-service-spec-v1alpha1 (DRAFT) | Drafted: 2026-05-29 | Critical surface: §2.3 parallel A/B/C computation;  §6 selector;  §7 cold-start chain;  §8 classifier rules | Hot-path budget: A+B+C ≤ 15ms p99 (full); A+B ≤ 5ms p99; A only ≤ 1ms p99 | Branch: `design/predictor-upgrade`*
