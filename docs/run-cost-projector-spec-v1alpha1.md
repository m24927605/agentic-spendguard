# Run Cost Projector Specification — v1alpha1 (DRAFT)

> 📝 **Status: DRAFT** (writing in design phase on branch `design/predictor-upgrade`)
> **DRAFT → LOCKED criteria**: locks together with the predictor-upgrade spec set per `predictor-architecture-spec-v1alpha1.md` §0.2; additionally requires (a) `RUN_BUDGET_PROJECTION_EXCEEDED` precision ≥ 90% in staged loop benchmark across OpenAI Agents / LangGraph / Pydantic-AI, (b) Project p99 ≤ 5ms, (c) state cache eviction is bounded (no run leaks memory).
> **Companion specs (this set)**: `predictor-architecture-spec-v1alpha1.md` (umbrella; Q3 reasoning), `output-predictor-service-spec-v1alpha1.md` (per-call cost feed), `stats-aggregator-spec-v1alpha1.md` (run-length distribution provider), `contract-dsl-spec-v1alpha2.md` (new RUN_* codes + run_projection_action enum activator), `audit-chain-prediction-extension-v1alpha1.md` (run-level audit columns).
> **Pre-existing LOCKED dependencies**: `contract-dsl-spec-v1alpha1.md` (§7 reservation 兩相 invariant), `trace-schema-spec-v1alpha1.md` (`run_id` UUID v7 in §3.2), `sidecar-architecture-spec-v1alpha1.md` (§5 mTLS internal transport).
> **Compatibility policy**: alpha — proto3 additive evolution; Signal 1/2/3 layering rules versioned per spec bump; state cache TTL configurable per tenant.

---

## §0. Lock status & prerequisites

### 0.1 範圍

本 spec 定義 **run_cost_projector service**：

1. gRPC `Project` API — sidecar 每 decision 一次 call，回 per-run projection
2. Signal 1（induced from history）算式
3. Signal 2（per-step dynamic re-projection）機制
4. Signal 3（explicit `with_run_plan` decorator）wire 路徑
5. Signal layering + override 規則
6. Per-`run_id` state cache + TTL + eviction
7. 三新 decision codes 觸發條件
8. `run_projection_action` policy 行為
9. Failure modes + SLO

**不在本 spec 範圍**：

- 三新 decision codes 在 contract DSL 的定義（推給 `contract-dsl-spec-v1alpha2.md` §3）
- Run-length distribution 如何 aggregate（推給 `stats-aggregator-spec-v1alpha1.md` §6）
- Per-call prediction（推給 `output-predictor-service-spec-v1alpha1.md`）
- 三個 RUN_* audit columns（推給 `audit-chain-prediction-extension-v1alpha1.md` §2.2）

### 0.2 DRAFT → LOCKED criteria

進入 LOCKED 之前下列 5 項必達成：

1. SLICE 09 PR merged：projector service + 三 signals + RUN_* code emission + state cache
2. `RUN_BUDGET_PROJECTION_EXCEEDED` precision ≥ 90% 在 staged loop benchmark
3. `Project` p99 ≤ 5ms（含 cache lookup + signal computation）
4. State cache eviction 對 10K concurrent runs 無 memory leak（72h endurance test）
5. SDK Signal 3 decorator 在 5 framework 通過 integration test（LangChain / LangGraph / Pydantic-AI / OpenAI Agents / AGT）

### 0.3 GA prerequisites

於 `predictor-architecture-spec-v1alpha1.md` §0.3 列出。本 spec 額外要求：

1. 3 frameworks × 30 日 production traffic 證實 RUN_* code 觸發合理（< 5% false positive）
2. State cache 10K+ concurrent runs scaled horizontally (sharded by run_id hash)
3. `with_run_plan` decorator 在 SDK 全部 integration 文檔化 + example

### 0.4 何時可能需要 v2

- 新增 Signal 4
- 改變 Signal layering rule（極不可能）
- State cache 改為分散式（per Phase 2+ horizontal scale）

---

## §1. Context (self-contained)

### 1.1 為什麼有這份 spec

per `predictor-architecture-spec-v1alpha1.md` §3.3 Q3 reasoning：

1. **真正 differentiation moat** —— LiteLLM 是 per-key cumulative；SpendGuard 既有是 per-call atomic；per-run projection 無人做
2. **能 stop 第 11 個 stuck-loop call 而非第 47 個** budget-exhaustion call
3. **Substrate 已在** —— `run_id` UUID v7 已在所有 SDK integration 寫進 audit row
4. **Universal coverage without framework cooperation** —— Signal 1 對 vanilla agents 也生效

沒有 run_cost_projector：

- SpendGuard 退回成「每 call atomic enforcement」級競品（仍勝 race condition 但失去 moat）
- Stuck-loop 仍要燒到 budget 用盡才停 → audit row 多 30+ 個 reservation 才看到 BUDGET_EXHAUSTED
- 客戶無法看「這個 run 預估還會花多少」projected_total_cost surfaceable in dashboard

### 1.2 在系統中的位置

```
sidecar (per decision)
  ↓
output_predictor.Predict() returns per-call A/B/C
  ↓
run_cost_projector.Project()                  ← 本 spec
  ↓
emits RUN_* decision codes if thresholds crossed
  ↓
contract DSL evaluator (per contract-dsl-v1alpha2 §3)
  ↓
final DecisionResponse
```

### 1.3 v1alpha1 核心哲學

> **Signal 1 是 universal default**；無需 framework 合作；對 vanilla agent loop 也生效。
>
> **Signal 2 是每 call 重算**；historical projection 不可信，每 call 都要 update。
>
> **Signal 3 是 power-user opt-in**；frameworks 自己知道 plan 可以宣告；不強制；override Signal 1。
>
> **Run state cache 是 in-memory**；no persistence；崩潰 = run state 重建（從 audit chain replay）；對 hot path 不阻塞。
>
> **三 RUN_* codes 在 hot path 觸發**；不等 batch；不等 hourly aggregation。

---

## §2. Service surface

### 2.1 gRPC proto

新檔案：`proto/spendguard/run_cost_projector/v1/projector.proto`

```protobuf
syntax = "proto3";
package spendguard.run_cost_projector.v1;
import "google/protobuf/timestamp.proto";

service RunCostProjector {
  // Hot-path: project remaining cost for a run, decide if RUN_* codes
  // should be emitted. Synchronous; called per decision; p99 ≤ 5ms.
  rpc Project(ProjectRequest) returns (ProjectResponse);

  // Signal that a run has terminated (run.end event). Clean cache.
  rpc TerminateRun(TerminateRunRequest) returns (TerminateRunResponse);
}

message ProjectRequest {
  string tenant_id = 1;
  string run_id = 2;
  string agent_id = 3;
  string step_id = 4;
  string decision_id = 5;

  // Per-call cost just computed by output_predictor.
  // This is the cost we'd reserve for THIS call (Strategy A / B / C
  // per policy).
  int64 this_call_reservation_atomic = 6;
  string unit_id = 7;

  // Budget remaining (post this call hypothetically).
  int64 budget_remaining_atomic = 8;

  // Signal 3 hint (optional).
  optional int32 planned_steps_hint = 9;
  optional int32 planned_tools_hint = 10;
}

message ProjectResponse {
  // Projection at decision time.
  int64 run_projection_at_decision_atomic = 1;
  int32 run_predicted_remaining_steps = 2;
  int32 run_steps_completed_so_far = 3;

  // Which signal(s) drove the projection.
  string signals_used = 4;  // "1" | "1,2" | "1,2,3" | "2,3" etc.

  // If any RUN_* code should be emitted, populated here.
  string emitted_code = 5;  // "RUN_BUDGET_PROJECTION_EXCEEDED" | "RUN_DRIFT_DETECTED" | "RUN_STEPS_EXCEEDED" | ""

  // Confidence + diagnostic.
  float projection_confidence = 6;
  string projection_diagnostic = 7;  // free-form for ops debugging
}

message TerminateRunRequest {
  string tenant_id = 1;
  string run_id = 2;
  string reason = 3;  // "completed" | "aborted" | "error" | "timeout"
}

message TerminateRunResponse {
  bool removed_from_cache = 1;
}
```

### 2.2 Deployment

集中 service（gRPC mTLS）+ in-memory state cache + Postgres advisory lock for HA leader election if multi-replica.

Phase 1: single replica per tenant shard. Phase 2: shard by `hash(run_id) mod replica_count`.

---

## §3. Signal 1 — induced from history

### 3.1 算式

```
predicted_remaining_steps_signal1 =
    max(0, run_length_p95(tenant_id, agent_id) - steps_completed_so_far)

predicted_remaining_cost_signal1 =
    predicted_remaining_steps_signal1 × strategy_b_per_call(tenant_id, model, agent_id, class)
```

`run_length_p95` 從 `stats-aggregator-spec-v1alpha1.md` §6 cache 讀。`strategy_b_per_call` 從 `output-predictor-service-spec-v1alpha1.md` Strategy B 讀（透過 caller pass through）。

### 3.2 Cold start

`(tenant, agent_id)` bucket 樣本不足 → fall back to global default:

- `run_length_p95_default = 10`（per industry convention; tune-able per release）
- emit metric `run_length_cold_start{ tenant, agent_id }`

不 fall through 到 L2 / L3 等 layered 系統 —— run length 量少時用 global default 簡單；之後 stats_aggregator 累積樣本後自動覆蓋。

### 3.3 Universal coverage

Signal 1 對任何 agent 都生效，**無需 framework 合作**：

- Vanilla `openai.chat.completions.create()` loop → SDK wrapper 寫 `run_id` per session → audit chain captures → stats_aggregator aggregates → Signal 1 命中
- 無需 SDK 知道 run 結構；無需 user 宣告 plan

---

## §4. Signal 2 — per-step dynamic re-projection

### 4.1 機制

每次 Project call 都重算 projection，不靠 cache stale value。Pseudo：

```
At step N:
  cumulative_cost = sum of all prior step costs (read from state cache)
  this_call_reservation = req.this_call_reservation_atomic
  predicted_remaining = predicted_remaining_steps_signal1
  predicted_remaining_cost = predicted_remaining × strategy_b_per_call

  projection = cumulative_cost + this_call_reservation + predicted_remaining_cost

  IF projection > req.budget_remaining_atomic:
    emit RUN_BUDGET_PROJECTION_EXCEEDED
```

### 4.2 Drift detection

Signal 2 同 step 偵測 drift：

```
predicted_remaining_cost_now = (Signal 1 + 2 computation)
predicted_remaining_cost_prior_step = state_cache[run_id].last_predicted_remaining_cost

IF |predicted_remaining_cost_now - predicted_remaining_cost_prior_step| / prior > 2σ_threshold
   AND happened 3+ consecutive steps (configurable N):
    emit RUN_DRIFT_DETECTED
```

`2σ_threshold` 從 stats_aggregator 的 mean/stddev 算（per `stats-aggregator-spec-v1alpha1.md` §7）。

### 4.3 與 prediction_drift_alert 區別

per `contract-dsl-spec-v1alpha2.md` §3.2：

- `prediction_drift_alert`（bucket-level，hourly aggregator）
- `RUN_DRIFT_DETECTED`（run-instance-level，hot path; 本 spec §4.2）

兩者各有 audit row + 各自 metric。

---

## §5. Signal 3 — explicit hint via SDK decorator

### 5.1 SDK 表面

```python
from spendguard import with_run_plan

@with_run_plan(planned_calls=8, planned_tools=2)
async def my_agent_function(...):
    # agent runs N LLM calls + M tool calls
    ...
```

Decorator 把 hint 塞進 SDK 的 `request_decision` payload metadata；sidecar 在 `RequestDecision` 收到 `planned_steps_hint = N + M` + `planned_tools_hint = M`。Sidecar 傳給 projector 的 `ProjectRequest.planned_steps_hint`.

### 5.2 Override semantics

Signal 3 active（`planned_steps_hint > 0`）→ override Signal 1：

```
IF req.planned_steps_hint > 0:
    predicted_remaining_steps = max(0, req.planned_steps_hint - steps_completed_so_far)
ELSE:
    predicted_remaining_steps = signal1_value
```

Signal 2 仍 always-on（每 step 重算 cumulative）。

### 5.3 RUN_STEPS_EXCEEDED 觸發

只有 Signal 3 active 時可能觸發：

```
IF req.planned_steps_hint > 0 AND steps_completed_so_far > req.planned_steps_hint:
    emit RUN_STEPS_EXCEEDED
```

Default vanilla agent 無 hint → 不可能觸發 RUN_STEPS_EXCEEDED。

---

## §6. Signal layering + override 規則

```
At each Project call:

1. Compute Signal 1: predicted_remaining_steps_signal1
   (uses stats_aggregator run-length P95 or cold-start default 10)

2. If Signal 3 active: override Signal 1
   predicted_remaining_steps = signal3_value
   ELSE: predicted_remaining_steps = signal1_value

3. Signal 2 always-on:
   predicted_remaining_cost = predicted_remaining_steps × strategy_b_per_call
   (Signal 2 dynamic recompute via stats_aggregator's latest B)

4. Compute projection:
   projection = cumulative_cost + this_call_reservation + predicted_remaining_cost

5. Compare to budget:
   if projection > budget_remaining → RUN_BUDGET_PROJECTION_EXCEEDED
   if drift detected (per §4.2) → RUN_DRIFT_DETECTED
   if signal3 and steps > hint → RUN_STEPS_EXCEEDED

6. Emit one or zero codes (precedence: BUDGET > STEPS > DRIFT)
```

### 6.1 Code precedence justification

- BUDGET > STEPS：budget exhaustion 比 steps 超預期更嚴重（直接 financial impact）
- STEPS > DRIFT：steps超預期已暗示 drift；emit 一個 sufficient
- 同時觸發 → 只 emit 最高優先；reason_codes 仍含全部

---

## §7. Run state cache

### 7.1 Schema

```rust
struct RunState {
    run_id: Uuid,
    tenant_id: Uuid,
    agent_id: String,
    started_at: Instant,
    last_activity_at: Instant,
    steps_completed: u32,
    cumulative_cost_atomic: i64,
    cost_per_step: Vec<i64>,                   // for drift detection
    last_predicted_remaining_cost: Option<i64>,
    drift_consecutive_count: u32,
    signal3_hint_planned_steps: Option<u32>,
}
```

### 7.2 Eviction

| Trigger | Action |
|---|---|
| `TerminateRun` RPC received | Remove from cache |
| `last_activity_at > now() - TTL`（default 30 min） | Remove + emit metric |
| Process restart | All states lost; replay from audit chain for live runs |
| Memory pressure（cache > 80% allocated memory） | LRU evict |

### 7.3 TTL configurability

- Default 30 min per (tenant, agent_id)
- Tenant-overridable via control plane（min 5 min, max 4 hours）
- 反映客戶 agent 平均 run length

### 7.4 Eviction safety

被 evict 的 run 後續若有新 decision 進來 → 重建 state from audit chain replay（per Sidecar §11 recovery）；無資料遺失。

---

## §8. Decision codes（per contract-dsl-v1alpha2 §3.1-§3.3）

### 8.1 RUN_BUDGET_PROJECTION_EXCEEDED

per `contract-dsl-spec-v1alpha2.md` §3.1：當 `projection > budget_remaining`。

projector emit code → sidecar pass to DSL evaluator → policy（`run_projection_action`）決定 final decision。

### 8.2 RUN_DRIFT_DETECTED

per §4.2 +`contract-dsl-spec-v1alpha2.md` §3.2 distinction。

### 8.3 RUN_STEPS_EXCEEDED

per §5.3 + `contract-dsl-spec-v1alpha2.md` §3.3。

---

## §9. `run_projection_action` policy（per contract-dsl-v1alpha2 §5）

projector 不執行 action（projector 只 emit code）；action 由 contract DSL evaluator + sidecar 決定。

對 projector 來說：

- emit `code` + `metadata` 給 sidecar
- sidecar 對 active `run_projection_action` decide：
  - `BLOCK_NEXT_CALL` → return STOP
  - `REQUIRE_APPROVAL` → return REQUIRE_APPROVAL
  - `ALERT_ONLY` → return CONTINUE + audit row

---

## §10. Failure modes

| 場景 | 行為 |
|---|---|
| stats_aggregator cache unreachable | Signal 1 fall to cold-start default（10 steps）；continue |
| State cache memory full | LRU evict；continue with rebuild |
| State cache lookup miss（cold run）| Initialize new state at step 0；continue |
| projector RPC unreachable from sidecar | Sidecar conservative fall-through：no RUN_* emitted；reservation 仍正確（用 A）；emit metric `projector_unreachable` |
| `TerminateRun` RPC fail | State remains; gets TTL evict later; no data loss |
| Audit chain replay fail | Sidecar fall to per-call decision only；run-level metadata loss |

---

## §11. Audit chain impact

per `audit-chain-prediction-extension-v1alpha1.md` §2.2，每次 Project response 寫進 audit row：

- `run_projection_at_decision_atomic` ← `response.run_projection_at_decision_atomic`
- `run_predicted_remaining_steps` ← `response.run_predicted_remaining_steps`（or -1 sentinel if projector unreachable）
- `run_steps_completed_so_far` ← `response.run_steps_completed_so_far`

CloudEvent proto mirror tags 311-313 per audit-chain extension §3.2。

---

## §12. SLO

### 12.1 Project p99 budget

- Cold cache miss (cache rebuild) → p99 ≤ 10ms
- Warm cache hit → p99 ≤ 5ms
- Hot path total 占 sidecar 50ms budget < 10%

### 12.2 RUN_* code precision

- `RUN_BUDGET_PROJECTION_EXCEEDED` precision ≥ 90% on staged loop benchmark
- `RUN_DRIFT_DETECTED` precision ≥ 80%（drift inherent variance higher）
- `RUN_STEPS_EXCEEDED` precision = 100%（deterministic comparison）

### 12.3 Cache hit rate

- Warm cache hit rate target ≥ 95%（per-tenant for long-lived agents）
- Cold cache rebuild from audit chain ≤ 30ms

---

## §13. GA prerequisites

於 `§0.3` 列出。本 spec 不重複。

---

## §14. Adoption history

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder — filled during Codex / panel adversarial review rounds per HANDOFF §9) |

---

## §15. Lock 後的下一步

1. SLICE 09 PR：projector service + 三 signals + RUN_* code emission + state cache + sidecar integration
2. SLICE 09 acceptance：staged loop benchmark precision ≥ 90% across 3 frameworks
3. SDK Signal 3 decorator 推到 SLICE 12 並文檔化
4. 客戶 dashboard surface for run projection（separate dashboard slice; post-launch）

---

*Document version: run-cost-projector-spec-v1alpha1 (DRAFT) | Drafted: 2026-05-29 | Critical surface: §3-§5 Signal 1/2/3; §6 layering precedence; §7 state cache; §8 RUN_* codes | SLO: Project p99 ≤ 5ms warm; RUN_BUDGET_PROJECTION_EXCEEDED precision ≥ 90% | Branch: `design/predictor-upgrade`*
