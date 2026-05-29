# Output Predictor Plugin Contract — v1alpha1 (DRAFT)

> 📝 **Status: DRAFT** (writing in design phase on branch `design/predictor-upgrade`)
> **DRAFT → LOCKED criteria**: locks together with the predictor-upgrade spec set per `predictor-architecture-spec-v1alpha1.md` §0.2; additionally requires (a) at least 1 design partner integrates a Strategy C plugin and operates 24h health rate > 99.5% for 30 consecutive days, (b) all documented failure modes have unit-test fallbacks to Strategy B verified, (c) `contrib/output_predictor_template/` reference plugin compiles + runs in a Docker sandbox in CI.
> **Companion specs (this set)**: `predictor-architecture-spec-v1alpha1.md` (umbrella; Q1 reasoning that defines this contract's purpose), `output-predictor-service-spec-v1alpha1.md` (calls plugin in delegated mode), `audit-chain-prediction-extension-v1alpha1.md` (audit `predicted_c_tokens` + nullable rules).
> **Pre-existing LOCKED dependencies**: `sidecar-architecture-spec-v1alpha1.md` (§5 mTLS internal transport).
> **Critical invariant**: Plugin failure must **never** propagate to enforcement decision. Falls back to Strategy B silently (with metric). This is the core safety contract.
> **Compatibility policy**: alpha — proto3 additive evolution; plugin maintainers receive 12-month deprecation notice for any breaking change; SpendGuard's central service guarantees v1alpha1 contract surface stable through v1beta1.

---

## §0. Lock status & prerequisites

### 0.1 範圍

本 spec 定義 **客戶自訓自託管 Strategy C predictor plugin 的 gRPC contract**：

1. Proto contract（`Predict` + `HealthCheck`）
2. Authentication（mTLS）+ endpoint registration（control plane API）
3. Hot-path timeout（50ms hard cap）+ error semantics + circuit breaker
4. Multi-tenant isolation 規則
5. Customer template（`contrib/output_predictor_template/`）的承諾

**不在本 spec 範圍**：

- SpendGuard 側 plugin 呼叫實作（推給 `output-predictor-service-spec-v1alpha1.md` §5）
- ML 模型訓練（永遠不做；per Q1）
- 客戶 plugin 的 internal implementation（客戶自由選擇 Python / Rust / Go / etc.）

### 0.2 DRAFT → LOCKED criteria

進入 LOCKED 之前下列 5 項必達成：

1. SLICE 07 + SLICE 14 PR merged：contract proto + delegated C mode in output_predictor + customer template repo
2. 至少 1 個 design partner 整合（i.e., 在 staging 跑 ≥ 7 日；audit 含 `predicted_c_tokens` 非 null rows ≥ 10K）
3. 5 failure modes（per §5）都有 unit test 確認 fallback 到 Strategy B
4. Template plugin Docker sandbox 通過 conformance test suite
5. mTLS rotation drill：客戶在不重啟 plugin 情況下完成 cert rotation

### 0.3 GA prerequisites

於 `predictor-architecture-spec-v1alpha1.md` §0.3 列出。本 spec 額外要求：

1. 24h health rate > 99.5% 對 first design partner（per `§9` observability）
2. Customer template 至少有 2 個 reference reimplementation（Python + Go）
3. Plugin v1alpha1 → v1beta1 migration playbook 文檔化

### 0.4 何時可能需要 v2

- 引入第 4 個 Strategy（極不可能）
- mTLS 改為其他 auth（不太可能；OAuth / JWT 雖然標準但 hot path 太重）
- 引入 batch prediction（Hardcoded singular request 改 batch）

---

## §1. Context (self-contained)

### 1.1 為什麼客戶自訓自託管（重申 Q1 reasoning）

per `predictor-architecture-spec-v1alpha1.md` §3.1 鎖定四條 reasoning：

1. **Multi-tenant ML 跨租戶 leak 風險高** —— SpendGuard hosted ML 無法 isolate
2. **客戶的 ML team 更懂自己 agent** —— 客戶 agent 行為 detail 只有客戶端有
3. **ML lifecycle 是另一個產品** —— model registry / A/B / drift retraining / rollback 屬 ML platform，不該混進 spend firewall
4. **Deterministic enforcement 是採購優勢** —— 「decision path 是否 AI?」答「enforcement deterministic; optional customer predictor informs projection」

因此 Strategy C 是 **contract，不是 service**。客戶決定要不要 build；要 build 客戶決定 stack；SpendGuard 只規範 wire surface。

### 1.2 在系統中的位置

```
output_predictor.Predict(req)
  ├─ Strategy A: max_tokens lookup           [always computed]
  ├─ Strategy B: SQL P95 lookup              [always tried; null on cold start]
  └─ Strategy C: gRPC call to customer plugin [only when tenant has endpoint configured]
                  ↓ ANY failure (timeout/error/illegal projection)
                  fall to B (silent; metric emitted)
```

**Plugin 失敗永遠不阻擋 reservation**。這是核心 invariant。

### 1.3 v1alpha1 核心哲學

> **客戶 plugin 失敗 = SpendGuard 退到 Strategy B**；reservation 仍然正確；customer impact 是「prediction confidence 略降」而非「budget hard block」。
>
> **50ms hard cap is non-negotiable**；客戶 plugin 必須在 50ms 內回，否則被視為 timeout。
>
> **Plugin endpoint per-tenant**；不允許 multi-tenant fan-in；每個 endpoint 只為一個 tenant 服務。
>
> **mTLS only**；不接受其他 auth；簡化 trust chain。
>
> **SpendGuard 不偷看 plugin internals**；只 call wire；client side observability only。

---

## §2. Plugin gRPC contract

### 2.1 Proto definition

新檔案：`proto/spendguard/output_predictor_plugin/v1/plugin.proto`

```protobuf
// SpendGuard customer-trained output predictor plugin contract.
//
// This file is shipped to customers and is the public ABI. Treat it as
// frozen except via proto3 additive evolution and 12-month deprecation
// notice for any breaking change.
//
// Compatibility: proto3, additive evolution; new fields appended at
// next available tag.

syntax = "proto3";
package spendguard.output_predictor_plugin.v1;
import "google/protobuf/timestamp.proto";

service OutputPredictorPlugin {
  // Hot-path: predict output tokens for a single decision.
  // SpendGuard's output_predictor calls this synchronously inside the
  // 50ms p99 budget. Plugin MUST respond within 50ms; any later =
  // timeout = fall to Strategy B.
  rpc Predict(PredictRequest) returns (PredictResponse);

  // Health check. SpendGuard calls this every 30s for circuit-breaker
  // state machine; not on hot path.
  rpc HealthCheck(HealthCheckRequest) returns (HealthCheckResponse);
}

message PredictRequest {
  // Plugin instance identity (SpendGuard-assigned; used for telemetry).
  string spendguard_call_id = 1;  // UUIDv7; per-call unique
  string tenant_id = 2;            // pre-validated by SpendGuard to match
                                   // the endpoint's configured tenant
  string model = 3;
  string agent_id = 4;
  string prompt_class = 5;         // one of the 7 enum values

  int64 input_tokens = 6;          // exact, from tokenizer service
  int64 max_tokens_requested = 7;  // request.max_tokens if specified, else 0

  // Optional context features SpendGuard surfaces to help the plugin.
  // Plugin may ignore.
  ContextFeatures features = 8;

  message ContextFeatures {
    int32 conversation_depth = 1;    // messages.length proxy
    bool has_tool_calls = 2;
    bool has_system_message = 3;
    int32 num_tool_definitions = 4;
    string user_role_hint = 5;        // "first" | "continuation" | "tool_response"
    google.protobuf.Timestamp request_time = 6;
  }
}

message PredictResponse {
  // Predicted output tokens. MUST be > 0 and <= model context window.
  // SpendGuard validates; out-of-range = treated as plugin error.
  int64 predicted_output_tokens = 1;

  // Plugin's self-reported confidence.
  // 0.0 - 1.0; SpendGuard writes to audit `prediction_confidence` column.
  float confidence = 2;

  // Sample size the plugin used to derive this prediction.
  // Written to audit `prediction_sample_size`.
  int32 sample_size = 3;

  // Plugin internal version (for audit/diagnostics).
  string plugin_version = 4;

  // Plugin feature hash (so SpendGuard can correlate audit drift with
  // plugin training/version changes).
  string feature_hash = 5;
}

message HealthCheckRequest {
  // Empty.
}

message HealthCheckResponse {
  enum Status {
    STATUS_UNSPECIFIED = 0;
    SERVING = 1;        // healthy; can answer Predict
    DEGRADED = 2;       // can answer but quality degraded (e.g., model retrain underway)
    NOT_SERVING = 3;    // do not call Predict; SpendGuard skips C
  }
  Status status = 1;

  // Plugin self-reported version (for control plane diagnostics).
  string plugin_version = 2;
  google.protobuf.Timestamp checked_at = 3;
}
```

### 2.2 Wire format invariants

- All fields proto3 additive；新欄位永遠 append at next tag
- 12-month deprecation for any breaking change（per compatibility policy）
- Plugin authors SHOULD log unknown fields received（forward-compat with future SpendGuard additions）

---

## §3. Authentication — mTLS

### 3.1 必要條件

- 客戶 plugin endpoint 必須提供 valid TLS cert（PEM-encoded; full chain）
- SpendGuard side 用 client cert（per-tenant issued by SpendGuard control plane）
- Verify client cert subject = `spiffe://spendguard.platform/predictor-client/<tenant_id>`

### 3.2 Cert lifecycle

- 客戶在 control plane 註冊 endpoint 時 upload server cert public key fingerprint（pinning）
- SpendGuard 每 30 days rotate client cert；轉換窗口 12 hours dual-validity
- 客戶可在 control plane 觸發「force re-fetch SpendGuard 的 trust roots」（incident response）

### 3.3 不接受其他 auth

OAuth / JWT / shared secret / IP allowlist 全部不支援。理由：

- Hot path 50ms 不能負擔 OAuth token refresh
- Shared secret rotation cumbersome
- IP allowlist 在 K8s ephemeral pod 不可靠
- mTLS 是 SpendGuard 內部 transport 一致選擇（per `sidecar-architecture-spec-v1alpha1.md` §5），降低 customer integration 心智 cost

---

## §4. Timeout semantics

### 4.1 50ms hard cap

```
SpendGuard 的 output_predictor 對 plugin Predict RPC 設 50ms deadline.
Plugin 必須在 50ms 內回 PredictResponse.
deadline_exceeded = plugin error = fallback to Strategy B + emit metric.
```

### 4.2 Tenant 不可 override

50ms 不是 configurable；理由：

- 整個 sidecar decision 50ms p99 budget 包含 plugin call；放寬 plugin = 違反 Contract §14 SLO
- 客戶被誘惑「我多開 200ms 給 plugin 更精準」會破壞整體 latency invariant
- 若 plugin 真的需要 > 50ms compute，正確 architecture 是 plugin internal cache / pre-compute；不是放寬 deadline

### 4.3 Connection setup deadline

- mTLS handshake：500ms hard cap；超時 = plugin unreachable
- Connection pool：SpendGuard 維持 per-(tenant, endpoint) 連線池，避免每 call handshake

---

## §5. Error semantics + fallback to Strategy B

### 5.1 哪些情境視為 plugin error → fall to B

| 情境 | 處理 |
|---|---|
| Plugin RPC timeout (50ms) | fall to B + metric `customer_predictor_timeout` |
| Plugin returns gRPC error status | fall to B + metric `customer_predictor_grpc_error` |
| Plugin returns `predicted_output_tokens <= 0` | fall to B + metric `customer_predictor_invalid_zero_or_negative` |
| Plugin returns `predicted_output_tokens > model.context_window` | fall to B + metric `customer_predictor_invalid_overflow` |
| Plugin returns `confidence < 0` or `confidence > 1` | fall to B + metric `customer_predictor_invalid_confidence` |
| Plugin RPC throws deserialization error | fall to B + metric `customer_predictor_deserialization_error` |
| Plugin endpoint TLS handshake failure | fall to B + metric `customer_predictor_tls_error` |
| `HealthCheck` returns `NOT_SERVING` | skip Predict for this period（per circuit-breaker §6） |

### 5.2 Audit row 行為

當 plugin error 發生：

- `predicted_c_tokens = NULL`（per audit-chain extension §2.1）
- `prediction_strategy_used = 'B'`（fell to B）
- `reserved_strategy = 'A'`（per default policy）
- CloudEvent metadata 含 `plugin_error_reason` 欄位 for diagnostics（per audit-chain extension proto tag TBD —— 推上 v1beta1 補；v1alpha1 skip）

### 5.3 Plugin 永遠不能阻擋 reservation

這是 spec 最強的 invariant：**任何 plugin failure 都不該變 hard cap**。Plugin only informs projection；reservation 始終由 A（或 policy 允許下 B）算。

SLICE 07 acceptance 必含「plugin malicious return」test：plugin 故意 return 巨大值 / negative / illegal —— SpendGuard reservation 仍正確（用 B 或 A）。

---

## §6. Circuit breaker

### 6.1 狀態機

```
[Closed]  -- 10 consecutive failures --> [Open]
[Open]    -- 5 min elapsed             --> [Half-Open]
[Half-Open] -- probe success           --> [Closed]
[Half-Open] -- probe fail              --> [Open] (reset 5 min)
```

### 6.2 Per-tenant scope

每 tenant 自己的 circuit breaker（不 cross-tenant aggregate）。一個 tenant 的 plugin 壞掉不影響其他 tenants 的 plugin call。

### 6.3 Open state 行為

- SpendGuard skip plugin call；fall to B
- 每 30s HealthCheck（per §2.1）；若 HealthCheck SERVING → half-open probe Predict；成功則 close
- Operator 可在 control plane API force-reset circuit breaker（incident response）

### 6.4 Half-open probe

- 1 個 Predict call（純測試）
- 結果若 success → close；fail → re-open
- Probe 用 dummy request（標記 `spendguard_call_id` 以 `probe-` prefix 區分）

---

## §7. Multi-tenant isolation

### 7.1 Endpoint per tenant

每個 plugin endpoint **只能服務一個 tenant**。Control plane register endpoint 時必綁 tenant_id。

不允許 customer 跨 tenant 共用 plugin：

- A tenant 看不到 B tenant 的 prompt patterns → 無法 train cross-tenant
- 即便兩個 tenant 屬同個 customer organization，仍需 separate endpoints
- 例外（design partner）需 explicit signed audit event approval

### 7.2 mTLS cert 識別

SpendGuard 對 plugin 出示的 client cert 含 `tenant_id` 在 SVID subject。Plugin 收到 cert 應驗證該 tenant_id matches its configured 預期 tenant；若不 match → reject + emit warning。

### 7.3 SpendGuard 側 enforcement

`output_predictor` 在 call plugin 前 lookup `(tenant_id → endpoint_url, client_cert_id)`，由 control plane 維護。Lookup result tenant_id 必 match request 中 tenant_id；否則 hard refuse（not even fall to B —— 視為 config error）。

---

## §8. Control plane API for endpoint registration

```
POST   /api/v1/predictor-plugins
  body: { tenant_id, endpoint_url, server_cert_fingerprint, ... }
  returns: { plugin_endpoint_id, client_cert_chain_pem }

PUT    /api/v1/predictor-plugins/{id}
  body: 同上（修改 endpoint）

DELETE /api/v1/predictor-plugins/{id}
  effect: 之後 output_predictor 跳過該 tenant 的 C path

GET    /api/v1/predictor-plugins/{id}/health
  returns: { last_health_check, status, error_rate_24h, ... }

POST   /api/v1/predictor-plugins/{id}/force-reset-circuit-breaker
  emit audit event
```

每個 API 操作 emit 對應 CloudEvent `spendguard.plugin.{registered, updated, deleted, force_reset}`，signed + immutable。

---

## §9. Observability requirements

### 9.1 Metrics SpendGuard 自動 emit

- `customer_predictor_call_total{ tenant, outcome }` — counter
- `customer_predictor_latency_seconds{ tenant }` — histogram
- `customer_predictor_health_rate_24h{ tenant }` — gauge (computed rolling)
- `customer_predictor_circuit_breaker_state{ tenant }` — gauge (0/1/2)

### 9.2 Plugin 端 SHOULD emit（不強制；推薦）

- Predict QPS / latency / error rate (per plugin)
- Model training freshness（plugin internal）
- Feature distribution drift（plugin internal）

### 9.3 Dashboard surface

Control plane dashboard 展示 per-(tenant, plugin) 的 24h health rate + latency P50/P95/P99 + circuit breaker history。

---

## §10. Customer template guarantees

### 10.1 `contrib/output_predictor_template/`

SpendGuard 提供 reference Python 實作（per SLICE 14）：

```
contrib/output_predictor_template/
├── predictor_server.py        # gRPC server skeleton
├── feature_extractor.py       # ContextFeatures parsing
├── model_predictor_stub.py    # sklearn-style model stub（客戶替換）
├── backtest_harness.py        # offline validation
├── Dockerfile                 # contained run env
├── mtls_setup.md              # cert setup walkthrough
├── conformance_test.py        # SpendGuard's conformance corpus
└── README.md
```

### 10.2 SpendGuard 對 template 的承諾

- 12-month 不 break template 的 wire 行為
- Conformance test corpus 持續 update（新 model / class 時補）
- Reference Python 維護到 SpendGuard v1beta1 結束（之後可能改維護 Go version）

### 10.3 客戶選擇權

客戶 free to:

- 改 stack（不用 Python，可 Go / Rust / TypeScript）
- 改 model（sklearn / XGBoost / LightGBM / transformer）
- 改 deployment（K8s / Lambda / VM）
- 改 feature engineering（ContextFeatures 多少用）

唯一不能改：proto contract（per §2）。

---

## §11. Versioning (v1alpha1 → v1beta1 migration)

### 11.1 Wire 升級路徑

- v1alpha1 → v1beta1：strictly additive；客戶 plugin 不修可繼續 work
- v1beta1 之內：additive 持續；新 features 新 tags

### 11.2 Breaking change 規則

- 任何 breaking change → 提前 12-month deprecation notice via control plane API + email
- Notice 期間 dual-version support（v1alpha1 + v1alpha2 endpoints coexist）
- Customer 自選 migration timing

### 11.3 Plugin 端版本宣告

```
HealthCheckResponse.plugin_version
PredictResponse.plugin_version
```

兩處宣告版本給 SpendGuard 用於 audit（`predicted_c_plugin_version` 推 v1beta1 audit-chain extension 補；v1alpha1 仍 skip）。

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

1. SLICE 07 PR：plugin proto + delegated C mode in output_predictor + control plane API + circuit breaker + per-tenant isolation enforcement
2. SLICE 14 PR：`contrib/output_predictor_template/` reference Python implementation + Docker sandbox + conformance test
3. First design partner integration drill：mTLS setup + first Predict call audit row verify
4. Optional Go reference template post-launch

---

*Document version: output-predictor-plugin-contract-v1alpha1 (DRAFT) | Drafted: 2026-05-29 | Critical invariant: plugin failure NEVER blocks reservation, always falls to Strategy B silently (per §5.3) | Hard caps: 50ms RPC timeout (non-overridable); mTLS only auth | Branch: `design/predictor-upgrade`*
