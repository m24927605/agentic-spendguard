# Predictor Architecture Specification — v1alpha1 (DRAFT)

> 📝 **Status: DRAFT** (writing in design phase on branch `design/predictor-upgrade`)
> **DRAFT → LOCKED criteria**: this spec, plus all 9 sibling specs in this set, lock together after — (1) the full predictor-upgrade design merges to `main`, (2) the first 2 rounds of `predictor-review-checklist.md` adversarial review close clean per the spec set, and (3) at least one design partner POC runs the predictor service through `calibration-report` and surfaces stable B/C calibration ratios (P95 reserved/actual ≤ 1.30) for ≥7 consecutive days.
> **Companion specs (this set)**:
> - `tokenizer-service-spec-v1alpha1.md` (DRAFT)
> - `output-predictor-service-spec-v1alpha1.md` (DRAFT)
> - `output-predictor-plugin-contract-v1alpha1.md` (DRAFT)
> - `run-cost-projector-spec-v1alpha1.md` (DRAFT)
> - `cold-start-baseline-spec-v1alpha1.md` (DRAFT)
> - `contract-dsl-spec-v1alpha2.md` (DRAFT, additive over v1alpha1)
> - `stats-aggregator-spec-v1alpha1.md` (DRAFT)
> - `calibration-report-spec-v1alpha1.md` (DRAFT)
> - `audit-chain-prediction-extension-v1alpha1.md` (DRAFT)
>
> **Pre-existing LOCKED dependencies**: `contract-dsl-spec-v1alpha1.md`, `sidecar-architecture-spec-v1alpha1.md`, `trace-schema-spec-v1alpha1.md`, `ledger-storage-spec-v1alpha1.md`, `stage2-poc-topology-spec-v1alpha1.md`, `agent-runtime-spend-guardrails-complete.md` (v1.3 strategy).
> **Compatibility policy**: alpha — additive over existing capability levels L0–L3 (Sidecar §4) / Contract DSL v1alpha1 / audit chain canonical bytes (signing/verifier two-form: proto + ledger-JSON). Strategy A/B/C interface bumps follow per-component-spec versioning.

---

## §0. Lock status & prerequisites

### 0.1 範圍

本 spec 是 **predictor upgrade umbrella architecture**：定義整套 tokenizer + output_predictor + run_cost_projector + stats_aggregator + calibration_report + audit chain extension + customer plugin contract 的結構性決策，把 9 個 sibling specs 串成一個 coherent 系統。Component-level 實作細節推給各自的 sibling spec；本 spec 不重複它們的內容，只負責 cross-spec invariant 與架構性決策的存放與引用。

### 0.2 DRAFT → LOCKED criteria

進入 LOCKED 之前下列 5 項必達成：

1. 9 sibling specs 全部 merged to `main`（含 v1alpha2 Contract DSL additive 補丁）
2. `predictor-review-checklist.md` 對本 spec set 的前 2 輪 adversarial review 全 clean（per HANDOFF §9 round-pass rule：finding 清空才算過）
3. 至少 1 個 design partner POC 跑通 `calibration-report --tenant <id>` 並輸出穩定 B/C ratio P95 ≤ 1.30 ≥ 7 連續日
4. `verify-chain` regression 在新欄位寫入後仍綠（cross-ref `audit-chain-prediction-extension-v1alpha1.md` §11）
5. `Tier 3` heuristic hit rate < 0.1% over 10K production decisions（per HANDOFF §3.2 health invariant）

### 0.3 GA prerequisites

進入 GA 路徑前下列必達成（疊在 §0.2 之上）：

1. 5 個 production tenants 累積 30 日資料證實 Strategy B 在 7 個 prompt classes 各自的 calibration drift < 2σ
2. Tier 1 shadow 在所有支援的 provider 上 sample rate 1% × 30 日 ≥ 50K samples 證實 |T1 − T2| / T1 < 1%
3. Customer plugin contract 至少 1 個 design partner 整合並 24h health rate > 99.5%
4. Run cost projector 三 signal 在 OpenAI Agents / LangGraph / Pydantic-AI 三種 framework 下 `RUN_BUDGET_PROJECTION_EXCEEDED` precision ≥ 90%（在 staged loop benchmark 中）
5. Calibration report CLI 通過 audit / CFO / 第三方審計 reviewer 各一輪 walkthrough

### 0.4 何時可能需要 v2 architecture spec

只有以下情況才開啟 v2 architecture spec 修正：

- POC 揭示 A/B/C 三角色職責劃分有結構性錯誤（reservation 與 projection 應該合併或進一步切分）
- 發現第 5 個 §3 級 high-irreversibility decision 必須鎖
- 新增第 4 個 tokenizer tier 或第 4 個 output-projection strategy
- Contract DSL spec 升級至 v1beta1 時 predictor 對應 break

正常情況下 v1alpha1 → v1beta1 → v1（GA）為 additive 演進，**無 breaking changes**（compatibility policy 保證）。

---

## §1. Context (self-contained)

### 1.1 產品

**Agentic SpendGuard** 是 LLM agents 的 pre-call spend firewall：在每次 LLM call 出發前，atomic reserve 預算 + 簽 audit row + fail-closed on exhaustion。整套架構建在 Rust egress proxy（axum, port 9000）+ Rust sidecar（tonic over UDS）+ Postgres ledger，配 `audit_outbox` 不變式三角（immutability triggers + Ed25519/KMS-ECDSA-P256 signed CloudEvents + canonical_ingest verifier）。

### 1.2 載入決策的核心句

> 「企業使用我們的 agentic spend guard 重點是能不能有效阻止預算飆漲失控，預測必須精準 LLM token in & LLM token out。」

兩半語意決定本 spec set 的所有設計選擇：

- **(a) 「有效阻止預算飆漲失控」** → atomic per-call reservation 永遠 ≥ worst-case actual（reservation 是 authorization，不是 forecast）；reservation ≥ actual 是 hard invariant，永遠不被 Strategy B / C 違反在 default policy 下
- **(b) 「預測必須精準」** → token_in 必須 exact tokenizer（provider-native or canonical local BPE）、token_out 必須 calibrated probabilistic projection，且 operator-visible accuracy metrics 在 `calibration-report` CLI

### 1.3 Predictor 在 T → L → C → D → E → P 中的角色

```
T (Trace) → L (Ledger) → C (Contract DSL) → D (Decision) → E (Evidence) → P (Proof)
                                                  ↑
                                  本 spec set 在這層升級
                                  (Predict + Project; 不改 Control / Audit invariants)
```

Predictor upgrade 在 **D (Decision)** 階段嵌入：sidecar 在進入 `reserve` stage（Contract §6 stages[3]）前，先 query tokenizer service（hot path）+ output_predictor service（hot path or cached lookup）+ run_cost_projector（hot path），把結果寫進 `BudgetClaim` 並一路串進 `audit_outbox` 的 prediction columns。

不變式承諾：Contract §6 八階段 decision transaction 結構不變、§6.1 「無 audit 則無 effect」不變、§7 reservation 兩相不變、Sidecar §3 三層 architecture 不變、Trace §10.2 三層 storage class 不變。

### 1.4 v1alpha1 核心哲學

> **Strategy A 永遠是 reservation**；Strategy B / C 是 projection。在 default `STRICT_CEILING` policy 下，reservation 從不被 B / C 替代；reservation `≥` worst-case actual 是 hard invariant。
>
> **Tokenizer Tier 2 在 hot path**；Tier 1 是 async shadow（drift detection only），Tier 3 是 last-resort fallback（健康部署 < 0.1% hit）。
>
> **SpendGuard 不訓 ML model**；Strategy B 是純 SQL P95 aggregation，Strategy C 是 customer-trained gRPC plugin；SpendGuard 只提供 contract 與 reference template。
>
> **Per-run projection 是 differentiation moat**；競品最多做到 per-call atomic（LiteLLM / Portkey）或 per-key cumulative（Helicone / Langfuse），都沒做 per-`run_id` 的 multi-signal projection。
>
> **Audit chain 是 calibration 證據**；所有 prediction 欄位（11 prediction + 3 run-level + 4 commit-side = 18 新欄位 —— 含 `cold_start_layer_used` first-class promotion per audit-chain-prediction-extension §2.4）都進 audit chain，被簽章、被 replicated、被 `verify-chain` replay。

---

## §2. Problem statement

### 2.1 The competitive finding

「Pre-call enforcement」 已是 table-stakes LLM-gateway feature，SpendGuard 原本的 wedge 不再成立：

| Competitor | Stars | Pre-call enforcement? | Signed tamper-evident audit chain? |
|---|---:|---|---|
| LiteLLM | 48.6k | **Yes** — `ExceededTokenBudget` synchronous pre-forward | No (Prometheus metrics only) |
| Portkey | 11.9k | Virtual-key expiry (enterprise) | No |
| Microsoft AGT | 3.2k | **Yes** built-in (Tutorial 24 / 51 + ADR-0012) | Partial (audit logging) |
| Helicone | 5.8k | No — observability only | No |
| Langfuse | 28.2k | No — post-call cost only | No |
| agentbudget | 0.1k | Post-call (+8% overshoot in `benchmarks/runaway-loop/`) | No |
| AgentGuard | 0.2k | Post-call (+1700% overshoot on self-hosted base_url) | No |

**真正可防守的差異化**（前提：本 spec set 實作完成）：

(a) **Atomic per-call reservation under concurrent burst** — race-free vs LiteLLM eventually-consistent counter（field evidence: LiteLLM Issue #27480 tag-budget enforcement silently skipped on header path）

(b) **Per-run projection** — 無競品實作

(c) **Cryptographically verifiable audit chain of every reservation / commit / release** — 無競品實作（既有 SpendGuard 強項，本 upgrade 延伸到 prediction metadata 整套）

(d) **Calibration-grade prediction metadata in every audit row** — 本 spec set 帶來的新差異化

### 2.2 The code reality

當前 production code 在預測這塊**沒有支撐產品承諾**（verified 2026-05-29，code exact match HANDOFF §2.2）：

| 路徑 | 行為 | 失效模式 |
|---|---|---|
| `services/egress_proxy/src/decision.rs:277-295` — 17 行 `estimate_tokens` | `chars/4 × 2` heuristic | CJK 2–3× under-estimate；`max_tokens` 不讀（200-char prompt + `max_tokens=4096` 估 100 tokens 實際可達 4096 → under-reserve ~40×）；tool_calls / vision / system metadata 不計；單一公式跨所有 provider |
| `services/sidecar/src/decision/transaction.rs:562-577` — `build_budget_claims` | 完全不估，要求 caller-supplied `projected_claims` non-empty | 所有 SDK integration（litellm / langchain / pydantic_ai / openai_agents / agt）的 `claim_estimator: Callable` 都讓 user 自帶；docstring 例子是同樣的 `chars // 4` heuristic |

**結論**：「accurate token in/out prediction」當前是 aspirational claim。本 spec set 把它變成 code-backed reality。

### 2.3 The architecture pivot

| 維度 | Pre-upgrade pitch | Post-upgrade pitch |
|---|---|---|
| 主張 | 「Pre-call enforcement is the moat.」 | 「Atomic reservation + tokenizer-precise input + calibrated output projection + per-run projection + cryptographic calibration audit. Customer ML plugs in via stable contract. No competitor ships any of this.」 |
| Wedge | Pre-call vs post-call | Calibration evidence + race-free atomic + per-run beating per-call |
| Operator-visible differentiator surface | None | `spendguard calibration-report --tenant <id>` CLI |
| Verifiable claim | 「我們 block 了」（log line） | 「我們在 (model, strategy) 各 bucket 的 reserved/actual ratio P50/P95/P99，可被 `verify-chain` replay」 |

本 upgrade **additive over existing architecture**：Contract DSL / L0-L3 / audit chain semantics / sidecar fencing+drain 全部保留。新增 tokenizer service、output_predictor service、predictor plugin contract、run cost projector、calibration-grade audit columns、multi-provider routing、calibration report CLI、customer BYO predictor template。

---

## §3. The four locked decisions (cross-ref HANDOFF §5)

每條決策已在 design conversation 鎖定（HANDOFF §5 Q1-Q4）；以下是 condensed restatement，detail 推給 sibling specs。

### 3.1 Q1 — SpendGuard 不訓 ML model

**決策**：Strategy B 是純 SQL P95 aggregation（`stats_aggregator` service）。Strategy C 是 gRPC plugin contract；客戶自訓自託管。

**為什麼**：

1. **Multi-tenant ML 跨租戶 leak 風險高** — 共用模型訓練讓 customer A 的 prompt pattern 滲入 customer B 的預測
2. **客戶 ML team 更懂自己 agent** — customer-support agent vs code-review agent 的 output token 分佈本質不同，generic model 無法贏 customer-owned model
3. **ML lifecycle 是另一個產品** — model registry / A/B framework / drift retraining / rollback 屬於 ML platform，混進來稀釋 SpendGuard 定位
4. **Deterministic enforcement 是採購優勢** — EU AI Act / FedRAMP / FINRA 採購問「decision path 是否 AI?」的標準答案：「enforcement decisions are deterministic; an optional customer-trained predictor informs projection but never overrides the safety floor」

**Detail**：`stats-aggregator-spec-v1alpha1.md` + `output-predictor-plugin-contract-v1alpha1.md`。

### 3.2 Q2 — Tier 2 是 hot path，Tier 1 是 async shadow

**決策**：local exact tokenization（`tiktoken-rs` for OpenAI、vendored BPE for Anthropic / Gemini）是 reservation source of truth。Tier 1（provider `count_tokens` API）以 1% 預設 sampling rate async 跑，只做 drift detection。

**為什麼**：

1. **Latency** — Tier 1 加 50–80ms 摧毀 Contract §14 50ms p99 SLO + 每 burst benchmark 輸給 LiteLLM / Portkey
2. **Reliability** — Tier 1 dependency 在 provider 上會 cascade 出 reservation failure；Tier 2 fully self-contained
3. **Drift 仍然偵測得到** — 1% sample + alert 觸發後 100% cool-down window
4. **OpenAI tokenizer 是公開的** — tiktoken byte-exact，不存在 drift；Tier 1 對 OpenAI 完全不需要

**Detail**：`tokenizer-service-spec-v1alpha1.md`。

### 3.3 Q3 — Per-run projection（三 signal layered）

**決策**：完整建 three-signal projector。Default 行為對任何 agent（無 framework 合作）都生效。

**為什麼**：

1. **真正 differentiation moat** — LiteLLM 是 per-key cumulative；SpendGuard 既有是 per-call atomic；per-run projection 無人做。能 stop 第 11 個 stuck-loop call 而非第 47 個 budget-exhaustion call
2. **Substrate 已在** — `run_id` 已在所有 SDK integration 寫進 `transaction.rs:562`、historical commits per run queryable from canonical_events
3. **Universal coverage without framework cooperation** — Signal 1（induced from history）讓 vanilla `openai.chat.completions.create()` loop 也有 per-run projection；Signal 3 是 opt-in power-user 路徑不影響 default

**Detail**：`run-cost-projector-spec-v1alpha1.md` + `contract-dsl-spec-v1alpha2.md` §3 三新 decision codes + §4-§5 `run_projection_action` + `prediction_policy` enums。

### 3.4 Q4 — Cold-start 四層 fallback (L1/L2/L3/L4)

**決策**：L1 hard fallback（`model.context_window − input_tokens`）+ L2 public-benchmark-derived `model_default_distribution.toml`（shipped）+ L3 federated cross-customer aggregate（design now, deferred build until ≥10 prod tenants opt-in）+ L4 customer's own B distribution。Most-to-least specific：L4 → L3 → L2 → L1。

**為什麼**：

1. **Cold start 是 real failure mode** — 新 (tenant, agent_id, prompt_class) bucket 零樣本時 collapse 到 A 會 starve concurrent calls + 給 first-impression 災難
2. **Public benchmarks 對 L2 足夠** — MT-Bench / HumanEval / MBPP / LongBench / MMLU 都公開 completion length distribution，hand-curated table 直接組
3. **自跑 internal benchmarks 低 ROI** — dogfooding 不代表 customer workload；curate public data 比自己跑 benchmark CP 值高
4. **Federated L3 長期贏** — real customers 多了後 aggregate 比個別 tenant 小樣本準；但需要客戶體量 seed，deferred 到 ≥10 prod tenants

**Detail**：`cold-start-baseline-spec-v1alpha1.md`。

---

## §4. Component architecture

### 4.1 系統圖（複製自 HANDOFF §5.5）

```
                ┌─────────────────────────────────────┐
                │   stats_aggregator                  │
                │   (pure SQL, no ML, no GPU)         │
                │   • per-class P50/P95/P99 per       │
                │     (tenant, model, agent, class)   │
                │   • per-(tenant, agent) run-length  │
                │   • drift detection (2σ shifts)     │
                └────────────┬────────────────────────┘
                             │  reads canonical_events,
                             │  writes output_distribution_cache
                             ▼
                ┌──────────────────────────────────────┐
                │ output_predictor (service)           │
                │   in:  (tenant, model, agent, class) │
                │   out: A, B, C predictions +         │
                │        strategy_used + confidence    │
                │                                       │
                │ • Strategy A: max_tokens-based       │
                │ • Strategy B: SQL P95 lookup         │
                │ • Strategy C: delegated to           │
                │     customer plugin endpoint         │
                │     (fallback B on any error)        │
                │ • Cold-start: L1→L2→L3→L4 fallback   │
                └────────────┬─────────────────────────┘
                             │
       ┌─────────────────────┼─────────────────────┐
       ▼                     ▼                     ▼
┌──────────────┐    ┌──────────────────┐   ┌──────────────────────┐
│ tokenizer    │    │ run_cost_         │   │ customer C plugin    │
│ service      │    │ projector         │   │ (gRPC, customer-     │
│              │    │                   │   │  hosted, optional)   │
│ Tier 2 hot   │    │ • Signal 1: hist  │   │                      │
│ (tiktoken-rs │    │ • Signal 2: per-  │   │ Customer trains and  │
│  vendored    │    │   step re-proj    │   │ deploys; SpendGuard  │
│  BPE)        │    │ • Signal 3: hint  │   │ calls; 50ms timeout; │
│              │    │                   │   │ fallback B on error  │
│ Tier 1 async │    │ emits decision    │   └──────────────────────┘
│ shadow (1%)  │    │ codes RUN_*       │
│              │    │                   │
│ emits        │    └────────┬──────────┘
│ drift_alert  │             │
└──────┬───────┘             │
       │                     │
       └──────┬──────────────┘
              ▼
   ┌─────────────────────────────────────┐
   │ egress_proxy decision.rs (rewritten) │
   │ + multi-provider routing (forward.rs)│
   └────────┬────────────────────────────┘
            │ DecisionRequest enriched with
            │   prediction metadata (10 cols)
            │   + run projection (3 cols)
            ▼
       ┌─────────┐         ┌──────────────────┐
       │ sidecar │ ──────▶ │ ledger /         │
       │         │ atomic  │ audit_outbox     │
       │         │ reserve │ (signed; trigger │
       └─────────┘         │  refuses U/D)    │
                           └────────┬─────────┘
                                    │
                                    ▼
                       outbox_forwarder
                                    │
                                    ▼
                          canonical_events
                          (carries all new
                           prediction columns)
                                    │
                                    ▼
                ┌──────────────────────────────────┐
                │ calibration-report CLI            │
                │ (operator-facing truth surface)   │
                └──────────────────────────────────┘
```

### 4.2 Service responsibility 對照

| Service | 主要責任 | 部署單位 | Hot path? | Spec |
|---|---|---|:---:|---|
| `tokenizer` | Tier 2 exact tokenize（OpenAI / Anthropic / Gemini）；Tier 1 async shadow；Tier 3 fallback；emit drift_alert | 集中 service（gRPC, internal mTLS） | ✅ (Tier 2 only) | `tokenizer-service-spec-v1alpha1.md` |
| `output_predictor` | 計算 A/B/C；selector；cold-start fallback；delegated mode to customer plugin | 集中 service（gRPC） | ✅ | `output-predictor-service-spec-v1alpha1.md` |
| `stats_aggregator` | 純 SQL aggregation；feed `output_distribution_cache` + run-length distribution；emit `prediction_drift_alert` | 定期 job（hourly default，per-tenant override） | ❌ | `stats-aggregator-spec-v1alpha1.md` |
| `run_cost_projector` | Signal 1 / 2 / 3 layered projection；emit `RUN_*` decision codes | 集中 service（gRPC，sidecar 每 decision 一次 call） | ✅ | `run-cost-projector-spec-v1alpha1.md` |
| `egress_proxy` (重寫) | `decision.rs::estimate_tokens` 全部替換；multi-provider routing；NetworkPolicy gate | 既有 service 升級 | ✅ | (cross-spec; SLICE 10/11) |
| `calibration_report` CLI | 對 `canonical_events` read-only aggregation；text / JSON / markdown 輸出 | 一次性 binary | ❌ | `calibration-report-spec-v1alpha1.md` |
| Customer plugin (Strategy C) | C 預測；mTLS gRPC；50ms hard cap | customer-owned, customer-hosted | ✅ (when enabled) | `output-predictor-plugin-contract-v1alpha1.md` |

### 4.3 跨服務依賴矩陣

| 上游 → 下游 | 何時呼叫 | 下游失敗時上游的 fallback |
|---|---|---|
| egress_proxy → tokenizer | 每個 request-time tokenize | Tier 2 panic = fail-closed reservation；Tier 3 emit metric；circuit breaker open 時降到 Tier 3 |
| egress_proxy → output_predictor | 每個 decision 取 A/B/C 三值 | A 必算成（純 lookup）；B null fallback cold-start；C null fallback B |
| sidecar → run_cost_projector | 每個 decision check RUN_* | projector unreachable = conservative pass-through，不阻擋下個 call，emit metric |
| output_predictor → stats_aggregator | read `output_distribution_cache` | cache stale > threshold → fall to cold-start chain |
| output_predictor → customer plugin | C path only | 任何 error / timeout / illegal projection（negative / > context_window）→ fall to B + emit `customer_predictor_error` |
| stats_aggregator → canonical_events | aggregation cycle | 連線失敗 → skip cycle，下次再跑；不阻擋 hot path |
| calibration_report → canonical_events | CLI 一次性 read | 連線失敗 → CLI 報錯；不影響 hot path |

---

## §5. Policy matrix（reservation vs projection 的職責分離）

每 contract 可宣告一個 `prediction_policy`（新 enum, see `contract-dsl-spec-v1alpha2.md` §4），決定 reservation 與 projection 的策略選擇。Default 為 `STRICT_CEILING`，operator 必須 explicit opt-in 其他。

| Policy | Per-call reservation | Per-run projection | 適用 | Reservation ≥ actual 保證 |
|---|---|---|---|---|
| **`STRICT_CEILING`** (default) | A | A (conservative) | 規範 / 合規 / zero-tolerance | **永遠** |
| `EMPIRICAL_RUN_CEILING` | A | B 或 C | 一般 SaaS | **永遠**（仍以 A reserve；B/C 只影響 per-run projection 與 calibration report） |
| `ADAPTIVE_CEILING` | B 為主；`remaining_budget < 2 × A` 時自動切回 A | B 或 C | High-throughput SaaS；接受罕見 commit overrun 換 budget utilization | 一般情況 violated；保留 Contract §7 phase B overrun_debt 機制兜底 |
| `SHADOW_ONLY` | A | A，但 audit row 仍寫 B/C for backtest | 評估新 tenant 的 B/C calibration 期 | **永遠** |

**核心 invariant**：

1. A 永遠被寫進 audit row 的 `predicted_a_tokens` column，無論 policy 為何
2. Reservation 在 `STRICT_CEILING` / `EMPIRICAL_RUN_CEILING` / `SHADOW_ONLY` 三 policy 下永遠 ≥ A，因此 ≥ worst-case actual
3. `ADAPTIVE_CEILING` 是 advanced opt-in；operator 必須明白簽下接受 commit overrun 風險換 budget utilization
4. `audit_outbox.prediction_policy_used` column 紀錄當下 active policy，calibration report 可 group by policy 比較 calibration profile

**Default 為 `STRICT_CEILING` 的理由**：規範性業務（healthcare / finance / government）採購 SpendGuard 時不能有「typical case 預估」滲入 enforcement decision；regulated environment audit 只能接受「reservation 是 ceiling」的純語意。Optional opt-in 給其他客戶選擇 utilization vs safety tradeoff，但 default 必須是 safety floor。

---

## §6. Audit chain summary

整套 upgrade 對 audit chain 的影響：`audit_outbox` + `canonical_events` 增加 **11 prediction columns + 3 run-level columns + 4 commit-side columns = 18 個 new columns**（含 `cold_start_layer_used` first-class promotion per audit-chain-prediction-extension §2.4），全部 additive nullable，全部進 canonical bytes derivation，全部被 `verify_cloudevent` replay。

新欄位的 column-level immutability 必須同步更新 `services/ledger/migrations/0011_immutability_triggers.sql` 的 `reject_audit_outbox_immutable_columns` 函式以避免被 forwarder UPDATE path 違反 audit immutability（此風險在 SLICE 01 已 identify，per HANDOFF Step 4 discrepancy #4）。

**Detail 全部推給** `audit-chain-prediction-extension-v1alpha1.md`。本 spec 不重複該 spec 的 schema 內容，只在 §5 / §8 cross-ref。

---

## §7. Cross-spec dependency map

15 個 implementation slices 對 10 個 specs + 2 個 review-standards 的引用關係。

| Slice | 主要實作 | 次要引用 |
|---|---|---|
| SLICE_01 canonical_events migration | `audit-chain-prediction-extension` §2-§5 | `contract-dsl-v1alpha2`（columns 對齊 codes） |
| SLICE_02 Contract DSL v1alpha2 additive | `contract-dsl-v1alpha2` 全本 | `run-cost-projector` §8（codes pass-through） |
| SLICE_03 tokenizer skeleton (OpenAI) | `tokenizer-service` §3 §6 §8 | `audit-chain-prediction-extension` §2（`tokenizer_tier` / `tokenizer_version_id`） |
| SLICE_04 Tier 2 expansion (Anthropic + Gemini) | `tokenizer-service` §3 §7 | 同上 |
| SLICE_05 Tier 1 shadow + drift | `tokenizer-service` §4 §8 | `stats-aggregator`（drift event emission） |
| SLICE_06 output_predictor A+B + stats_aggregator | `output-predictor-service` §3 §4 §6；`stats-aggregator` 全本 | `cold-start-baseline` §6（promotion threshold）；`audit-chain-prediction-extension` §2 |
| SLICE_07 plugin contract + delegated C | `output-predictor-plugin-contract` 全本；`output-predictor-service` §5 | `audit-chain-prediction-extension` §2（`predicted_c_tokens` nullable rules） |
| SLICE_08 cold-start TOML + loader | `cold-start-baseline` §4 §7 | `output-predictor-service` §7 |
| SLICE_09 run_cost_projector + RUN_* | `run-cost-projector` 全本 | `contract-dsl-v1alpha2` §3-§5（codes + policy 活化）；`audit-chain-prediction-extension` §3 |
| SLICE_10 egress_proxy decision.rs rewrite | (cross-component) | `tokenizer-service` §2；`output-predictor-service` §2；`audit-chain-prediction-extension` §2-§3 |
| SLICE_11 multi-provider routing | (cross-component) | `tokenizer-service` §3（per-provider dispatch） |
| SLICE_12 SDK default_estimator | (cross-component; Python SDK paths) | `tokenizer-service` §3；`run-cost-projector` Signal 3 hint |
| SLICE_13 calibration-report CLI | `calibration-report` 全本 | 全 spec set（CLI 讀所有 prediction columns） |
| SLICE_14 customer template | `output-predictor-plugin-contract` §10 | — |
| SLICE_15 E2E + benchmark | (cross-component test slice) | 全 spec set |

Review-standards 對應：

- `predictor-review-checklist.md` — universal checks，每 slice 第一輪必跑
- `staff-panel-arbitration-process.md` — round-5 fail 才觸發（per HANDOFF §8.6）

---

## §8. GA prerequisites（整套上線前必達成）

於 §0.3 列出。以下重複幾條因其為 spec-set-wide 不變式：

1. **Calibration accuracy**：5 production tenants × 30 日 × 7 prompt classes 全部 drift < 2σ
2. **Tier 1 / 2 / 3 health invariant**：Tier 3 hit rate < 0.1%；Tier 1 shadow |T1 − T2|/T1 < 1% on ≥50K samples
3. **Plugin contract maturity**：≥1 design partner integrated 並 24h health rate > 99.5%
4. **Run projection precision**：`RUN_BUDGET_PROJECTION_EXCEEDED` precision ≥ 90% in staged loop benchmark across OpenAI Agents / LangGraph / Pydantic-AI
5. **Audit chain regression-free**：`verify-chain` 對所有既有 demo 與新 prediction 欄位 rows 全綠
6. **Calibration-report walkthrough**：通過 audit / CFO / 第三方審計 各一輪 walkthrough

---

## §9. Adoption history

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder — filled during Codex / panel adversarial review rounds per HANDOFF §9) |

---

## §10. Lock 後的下一步

1. 9 sibling specs 並行寫作（per `design/predictor-upgrade` branch order）→ 各自走 maintainer review + adversarial review → merge to `main`
2. 15 slice docs 並行寫作 → 各自 maintainer review → merge to `main`
3. SLICE_01 schema migration 啟動（per HANDOFF §13.7 implementation phase 流程）
4. 每 slice 走 `ait run --adapter claude-code --review adversarial --review-budget deep` 至多 5 輪；round-5 fail 啟用 Staff+ panel per `staff-panel-arbitration-process.md`
5. 所有 15 slices merged 後跑 SLICE_15 E2E benchmark；calibration-report CLI 對 design partner POC tenant 連續輸出 7 日 → 本 spec set 整套 LOCKED

---

*Document version: predictor-architecture-spec-v1alpha1 (DRAFT) | Drafted: 2026-05-29 | Companion: 9 sibling specs in this set; pre-existing LOCKED dependencies listed in header | GA prerequisites listed §0.3 + §8 | Branch: `design/predictor-upgrade`*
