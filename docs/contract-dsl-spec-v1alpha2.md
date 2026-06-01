# Contract DSL Specification — v1alpha2 (DRAFT, additive over v1alpha1)

> 📝 **Status: DRAFT** (writing in design phase on branch `design/predictor-upgrade`)
> **DRAFT → LOCKED criteria**: locks together with the predictor-upgrade spec set per `predictor-architecture-spec-v1alpha1.md` §0.2; additionally requires (a) the 3 new decision codes (`RUN_BUDGET_PROJECTION_EXCEEDED` / `RUN_DRIFT_DETECTED` / `RUN_STEPS_EXCEEDED`) are accepted by the sidecar DSL evaluator without crashing in the initial SLICE_02 implementation (`d5c5434`), then activated by SLICE_09/10 projector wiring (`6407648` / `c649196`) with unsupported wire clients failing closed, (b) `prediction_policy` enum default is `STRICT_CEILING` confirmed by Codex round 2 adversarial review, and (c) all 8+ existing demo modes (`make demo-up DEMO_MODE=...`) running v1alpha1 contracts continue to produce identical decision outcomes after the v1alpha2 evaluator upgrade.
> **Pre-existing LOCKED dependency**: `contract-dsl-spec-v1alpha1.md` — this spec is a strictly additive bump over v1alpha1; **no v1alpha1 semantics changes, no field removals, no breaking enum renumbering**.
> **Companion specs in this set**: `predictor-architecture-spec-v1alpha1.md` (umbrella, defines policy matrix consumed by §4), `run-cost-projector-spec-v1alpha1.md` (defines decision-code emission semantics consumed by §3), `audit-chain-prediction-extension-v1alpha1.md` (defines audit columns that record which policy was active).
> **Compatibility policy**: alpha — strictly additive. v1alpha1 wire format remains byte-compatible; proto3 additive evolution gives new enum values new tags without renumbering existing ones; v1alpha1 contracts continue to load and evaluate identically on a v1alpha2 evaluator.

---

## §0. Lock status & prerequisites

### 0.1 範圍

本 spec 對 `contract-dsl-spec-v1alpha1.md` 的**最小可能 additive 補丁**：

1. 3 個新 decision codes（`RUN_BUDGET_PROJECTION_EXCEEDED` / `RUN_DRIFT_DETECTED` / `RUN_STEPS_EXCEEDED`），語意定義 + DSL 評估器 acceptance rules（SLICE_02 merge `d5c5434`）and SLICE_09/10 activation/fail-closed behavior（`6407648` / `c649196`）
2. 1 個新 policy enum `prediction_policy`（`STRICT_CEILING` | `EMPIRICAL_RUN_CEILING` | `ADAPTIVE_CEILING` | `SHADOW_ONLY`）+ default 設計
3. 1 個新 policy enum `run_projection_action`（`BLOCK_NEXT_CALL` | `REQUIRE_APPROVAL` | `ALERT_ONLY`）+ default 設計
4. 對 `proto/spendguard/sidecar_adapter/v1/adapter.proto` 的 `DecisionResponse.Decision` enum 與 budget claim schema 的 additive field 補丁
5. v1alpha1 contracts encountering v1alpha2 codes 的 compatibility 行為：contract bundle loading is capability-gated, and unknown response enum values must fail closed rather than allowing a provider call（egress_proxy commit `3035b54`, Python SDK HARDEN_03 commit `307eed4`）

**不在本 spec 範圍**：

- v1alpha1 既有任何 invariant 的調整（不允許 —— additive only；違者轉成 v2 spec 而非 v1alpha2 patch）
- Decision codes 的 *觸發邏輯* 細節（推給 `run-cost-projector-spec-v1alpha1.md`）
- Policy enum 的 *evaluation 邏輯* 細節（推給 `output-predictor-service-spec-v1alpha1.md` §6 與 `predictor-architecture-spec-v1alpha1.md` §5）
- Audit chain 新 column 設計（推給 `audit-chain-prediction-extension-v1alpha1.md`）

### 0.2 DRAFT → LOCKED criteria

進入 LOCKED 之前下列 4 項必達成：

1. SLICE 02 實作 proto additive 補丁通過 prost / tonic codegen + 全部 service 編譯不 break
2. SLICE 02 DSL evaluator 對 3 個新 codes 採 non-crashing acceptance 實作（`d5c5434`），且 SLICE_09/10 activation後 unsupported response enum consumers fail closed（`6407648` / `c649196`）
3. 既有 v1alpha1 contracts 在 v1alpha2 evaluator 下產生 byte-identical decision audit rows（regression test 在 8+ demo modes 全綠）
4. 對「v1alpha2 contract 含新 codes 但 sidecar 為 v1alpha1 版本」的 rollback 情境驗證 sidecar fail-closed（per `sidecar-architecture-spec-v1alpha1.md` §3.3 capability_required mismatch）

### 0.3 GA prerequisites

於 `predictor-architecture-spec-v1alpha1.md` §0.3 列出。本 spec 額外要求：

1. `RUN_BUDGET_PROJECTION_EXCEEDED` precision ≥ 90% 在 staged loop benchmark（across OpenAI Agents / LangGraph / Pydantic-AI）
2. `RUN_DRIFT_DETECTED` 與 `prediction_drift_alert`（stats_aggregator 發的）區分清楚，無重複觸發
3. `prediction_policy` 4 個值各自至少 1 個 production tenant 跑 30 日驗證行為符合 spec
4. `run_projection_action` 3 個值各自被測試覆蓋

### 0.4 何時可能需要 v1alpha3 或 v2

只有以下情況開啟下一輪 bump：

- 出現第 4 個 RUN_* 級別 decision code（建議：能 derive 自三者就不開新 code）
- `prediction_policy` 需要 mode parameters（例如 `ADAPTIVE_CEILING` 的 `2 × A` 切換閾值 tenant-overridable）—— 此時做 v1alpha3 additive
- v1alpha1 既有 invariant 需要 break（罕見；觸發 v2 而非 v1alpha3）

---

## §1. Context (self-contained)

### 1.1 為什麼 bump 但不 break

整套 predictor upgrade 引入 per-run projection + 多策略 prediction + multi-tier tokenizer，這些**對既有 contract DSL 既有 invariants 都不衝突**：

- decision transaction state machine（v1alpha1 §6）不變
- reservation 兩相（v1alpha1 §7）不變
- mode semantics shadow vs enforce（v1alpha1 §3）不變
- multi-budget atomic reservation（v1alpha1 §4）不變
- audit invariant 「無 audit 則無 effect」（v1alpha1 §6.1）不變

唯一不夠的是 **per-run 級別的決策碼語意**。現有 decision lattice（`continue` / `degrade` / `skip` / `stop` / `require_approval`）是 per-call 語意；per-run projection 需要的「stop next call in this run」與 per-call stop 在 audit / control plane / approval flow 都該分開歸類。

v1alpha2 用 **additive enum values**（per proto3 additive evolution）加 3 個新 RUN_* codes。舊 evaluator 不會載入 `apiVersion: spendguard.ai/v1alpha2` bundle（§8.1 fail-closed），支援 v1alpha2 的 evaluator 對 RUN_* codes 採對應 `run_projection_action` policy 處理。舊 client 若無法辨識新增的 response enum value，不得放行 provider call：Python SDK HARDEN_03 commit `307eed4` maps `STOP_RUN_PROJECTION` to `DecisionStopped`, and egress_proxy SLICE_02 commit `3035b54` treats unknown/unspecified decision variants as fail-closed sidecar errors.

### 1.2 在 T → L → C → D → E → P 中的位置

本 spec 完全在 **C (Contract DSL)** 層：

```
T → L → C → D → E → P
        ↑
    本 spec 在這層 additive bump
    (3 codes + 2 enums; 不改既有 v1alpha1 任何 semantics)
```

### 1.3 v1alpha2 核心哲學

> **v1alpha2 必須是 additive-only over v1alpha1**：任何 invariant 改動都不被允許在 v1alpha2 patch 內；觸發者升 v2。
>
> **新 codes 是 per-run 級別**：與 v1alpha1 的 per-call lattice 平行 namespace；不影響 v1alpha1 effect lattice 的 precedence。
>
> **`STRICT_CEILING` 是 default**：規範性業務的 safety floor，operator 必須 explicit opt-in 其他 policy（per `predictor-architecture-spec-v1alpha1.md` §5）。
>
> **Fail-closed 是 wire fallback；pass-through 只描述內部 evaluator compatibility**：v1alpha2 evaluator 對 v1alpha1 contracts 主動套用 default policy；SLICE_02 的 pass-through 是 parser/evaluator 內部 compatibility path（merge `d5c5434`），不是 old client 放行 provider call 的 fallback。Unsupported response enum consumers must fail closed per egress_proxy commit `3035b54` and Python SDK commit `307eed4`.

---

## §2. Compatibility with v1alpha1 (preserved invariants — re-asserted)

本節 verbatim 重申 v1alpha1 哪些不變量在 v1alpha2 evaluator 下保留。任何 SLICE 02 PR 違反此節必 fail review。

| v1alpha1 invariant | v1alpha2 保留？ | 機制 |
|---|---|---|
| §3 modeSemantics — shadow / enforce 物理分離 | ✅ | v1alpha2 evaluator 不觸碰 mode logic |
| §3 crossModeBudgetIsolation 不可 tenant override | ✅ | 同上 |
| §3 ledgerCapabilityRequirement — enforce_mode requires strong consistency | ✅ | 同上 |
| §4 reservationSet all_or_nothing | ✅ | 新 codes 不引入 partial reserve |
| §5 commit state machine 4 states | ✅ | 新 codes 不引入新 commit state |
| §5.1a refund / dispute policies | ✅ | 不變 |
| §6 decision transaction 8 stages | ✅ | 新 codes 在 stage `audit_decision` 寫入 audit row；不改 stage 結構 |
| §6.1 audit_decision before publish_effect | ✅ | 同上 |
| §6.2 audit batching rules | ✅ | 新 codes per-decision 1:1 不變 |
| §7 reservation 兩相 (pre-call top-up + post-commit overrun) | ✅ | 新 codes 不參與 reservation；參與 policy enum 對 reservation 策略選擇 |
| §8 sub-agent budget grant | ✅ | 不變 |
| §9 model capability matrix versioning | ✅ | 不變 |
| §10 effect lattice precedence: stop > require_approval > skip > degrade > continue | ✅ | 新 RUN_* codes 不參與 lattice（per §3.4） |
| §11 mutation patch constraints (RFC 6902 restricted) | ✅ | 新 codes 不引入 mutation |
| §12 Money / Time / Unit schema | ✅ | 不變 |
| §13 audit schema — signed, immutable, append-only | ✅ | 新 audit columns 同樣 signed（per `audit-chain-prediction-extension-v1alpha1.md`） |
| §13 semantic batching forbidden | ✅ | 不變 |
| §14 50ms p99 latency budget | ⚠ 條件保留 | 新 evaluator 對 run_projection_action 評估 +<1ms（per §7.2） |
| §15 trigger points — only *.pre evaluates policy | ✅ | 新 codes 在 `llm.call.pre` 觸發；不改 trigger map |
| §16 ledger consistency requirements | ✅ | 不變 |
| §17 bundle signature | ✅ | 新 evaluator 的 bundle hash 變化 → 既有 customer 需 rotate bundle_id（per `audit-chain-prediction-extension §9` schema_bundle rotation） |
| §18 Quickstart minimal contract | ✅ | v1alpha1 quickstart 在 v1alpha2 evaluator 下 100% 正確 |

---

## §3. New decision codes (3 new; additive)

### 3.1 `RUN_BUDGET_PROJECTION_EXCEEDED`

**語意**：本 call 評估 reservation 時，per-run projection（per `run-cost-projector-spec-v1alpha1.md` §3-§5 signal 1/2/3 layered）顯示 `cumulative_cost + this_call_reservation + predicted_remaining_cost > budget_remaining`。

**何時觸發**：每個 `llm.call.pre` decision boundary；由 `run_cost_projector` service 計算後通報 sidecar；sidecar 依 `run_projection_action` policy 決定該如何處理（`BLOCK_NEXT_CALL` / `REQUIRE_APPROVAL` / `ALERT_ONLY` —— per §5）。

**與 `BUDGET_EXHAUSTED`（v1alpha1）的區別**：

- `BUDGET_EXHAUSTED` 在 ledger reservation 階段觸發（hard cap）—— budget 已耗盡
- `RUN_BUDGET_PROJECTION_EXCEEDED` 在 projection 階段觸發（soft cap）—— budget 還沒耗盡但這個 run 不太可能跑完不超
- 兩個 code 可能同時 match；effect lattice 仍以 `STOP > REQUIRE_APPROVAL` precedence 處理

**Audit row 影響**：

- `decision_id` 與 normal stop 一樣 mint
- ledger `audit_outbox.cloudevent_payload` and downstream `canonical_events.payload_json` 的 `reason_codes` array 含 `"RUN_BUDGET_PROJECTION_EXCEEDED"`（schema lineage: commit `ca83792`; aggregator mirror columns were added by commit `8436cd4`）
- `run_projection_at_decision_atomic`（per `audit-chain-prediction-extension-v1alpha1.md` §2.2）必填

### 3.2 `RUN_DRIFT_DETECTED`

**語意**：本 run 的 per-call actual cost 顯著高於該 run 早期 calls 的預測（per `run-cost-projector-spec-v1alpha1.md` Signal 2 dynamic re-projection 連續多次往上修），暗示 agent 進入 stuck-loop / pathological prompt growth state。

**何時觸發**：`run_cost_projector` 在 update 後若發現 `predicted_remaining_cost` 連續 N steps（default N=3，配置在 projector）以 > 2σ 速率上升 → emit `RUN_DRIFT_DETECTED`。

**與 `prediction_drift_alert`（stats_aggregator）的區別**：

- `prediction_drift_alert` 是 **bucket-level** 漂移（per `(tenant, model, agent_id, prompt_class)` actual/predicted ratio 跨 period 變 > 2σ）—— 由 stats_aggregator 在 hourly aggregation 時 emit；不是 hot-path
- `RUN_DRIFT_DETECTED` 是 **run-instance-level** 漂移（per `run_id` 內 per-step cost 上升）—— 由 run_cost_projector 在 hot path emit；針對 stuck-loop / pathological scenario

兩者各有 audit row，互不衝突。calibration-report 同時聚合 stats_aggregator 與 projector 的 drift 數據。

**Audit row 影響**：與 §3.1 同；額外 `run_predicted_remaining_steps` + `run_steps_completed_so_far` 必填（per audit-chain extension §2.2）。

### 3.3 `RUN_STEPS_EXCEEDED`

**語意**：本 run 已執行 step 數超過 SDK `with_run_plan(planned_calls=N, planned_tools=M)` decorator 宣告的計畫（per HANDOFF §3.3 Signal 3）。

**何時觸發**：每個 `llm.call.pre` decision；當 `run_steps_completed_so_far > planned_steps_hint`（hint 來自 Signal 3）→ emit `RUN_STEPS_EXCEEDED`。

**只在 Signal 3 active 時觸發**。Default vanilla agents（無 `with_run_plan` 宣告）不會收到此 code。

**Audit row 影響**：與 §3.1 同；`run_steps_completed_so_far` 必填。

### 3.4 三新 codes 在 v1alpha1 effect lattice 的位置

v1alpha1 §10 effect lattice precedence：`stop > require_approval > skip > degrade > continue`。

**v1alpha2 新 codes 不修改 v1alpha1 lattice**。新 codes 對應到 `run_projection_action` enum（§5）後 *projects* 到 v1alpha1 lattice：

| RUN_* code | run_projection_action | 對應 v1alpha1 lattice decision |
|---|---|---|
| `RUN_BUDGET_PROJECTION_EXCEEDED` | `BLOCK_NEXT_CALL` | `stop` |
| `RUN_BUDGET_PROJECTION_EXCEEDED` | `REQUIRE_APPROVAL` | `require_approval` |
| `RUN_BUDGET_PROJECTION_EXCEEDED` | `ALERT_ONLY` | `continue`（但 audit row 仍 emit alert event） |
| `RUN_DRIFT_DETECTED` | (same 3 paths) | (same projection) |
| `RUN_STEPS_EXCEEDED` | (same 3 paths) | (same projection) |

**核心 invariant**：v1alpha1 effect lattice 與 audit row decision field 仍是 v1alpha1 的 5 個值；新 RUN_* codes 只出現在 `reason_codes` array 與 `audit_outbox` extension columns。下游 consumer（dashboards / SIEM / approval workflows）不需 update 來理解新 codes —— old consumer 看到 v1alpha2 audit row 仍能解讀 decision field。

---

## §4. New policy enum: `prediction_policy`

```yaml
prediction_policy:
  enum: [STRICT_CEILING, EMPIRICAL_RUN_CEILING, ADAPTIVE_CEILING, SHADOW_ONLY]
  default: STRICT_CEILING
  scope: per-contract-rule (覆蓋整個 contract 的所有 budgets)
```

詳細語意對應 `predictor-architecture-spec-v1alpha1.md` §5 policy matrix（4 行 × reservation/projection/適用/保證）；不在此重複。

### 4.1 Default 為 `STRICT_CEILING` 的理由（rationale captured in spec）

- 規範性業務（healthcare / finance / government）採購 SpendGuard 時不能有「typical case 預估」滲入 enforcement decision
- 升級 v1alpha1 contracts 自動繼承 `STRICT_CEILING`，行為與 v1alpha1 相同（A 是 reservation）—— 真正 backwards-compat default
- Opt-in 其他 policy 必須 contract author 明確簽（per §4.2 audit 簽章機制）

### 4.2 Policy change 必經 audit

對既有 contract 改 `prediction_policy`（v1alpha2 contract bundle 之間升級）必須觸發：

- 新 bundle ed25519 簽章
- audit event `spendguard.contract.policy_changed`（new event type, per Trace §7.5 CloudEvents 1.0 type list）
- bundle hash 不同 → schema_bundle_id rotation（per audit-chain extension §9）

不允許 runtime 動態切 policy（必須 bundle 重 deploy）。理由：runtime override 會給「我以為 audit log 是 STRICT 結果其實是 ADAPTIVE」的幻覺，破壞 calibration 證據鏈。

---

## §5. New policy enum: `run_projection_action`

```yaml
run_projection_action:
  enum: [BLOCK_NEXT_CALL, REQUIRE_APPROVAL, ALERT_ONLY]
  default: BLOCK_NEXT_CALL
  scope: per-contract-rule (per RUN_* code)
  audit_event_on_action: required
```

### 5.1 `BLOCK_NEXT_CALL`（default）

當 RUN_* code 觸發 → 下個 `llm.call.pre` decision return `stop`（v1alpha1 lattice）+ `reason_codes` 含對應 RUN_* code + audit row emit。

對 sidecar caller 行為：sidecar's `DecisionResponse.decision = STOP`；caller 拋 `DecisionStopped` exception（per SDK `spendguard.exceptions`）。Run 結束。

### 5.2 `REQUIRE_APPROVAL`

當 RUN_* code 觸發 → return `require_approval`；走 v1alpha1 §11 approval flow；approver tier 由 contract 在 rule 上 specify（per v1alpha1 §10 same_type_merge.require_approval）。

Approval grant 後 sidecar 走 `ResumeAfterApproval` RPC（per `proto/spendguard/sidecar_adapter/v1/adapter.proto` line 97）—— reservation 重新評估包括 projection；可能再次觸發 RUN_* 此次仍要 user approval。

### 5.3 `ALERT_ONLY`

當 RUN_* code 觸發 → return `continue`（不阻擋）+ audit row 仍 emit + dashboard alert + per-tenant Slack / webhook notification（per Operator setup）。

**警告**：`ALERT_ONLY` 在 `STRICT_CEILING` policy 下會被 evaluator 拒絕（contract bundle load 時 fail）—— 規範性業務不允許「警告但不擋」的 budget 策略。Allowed pairs：

| `prediction_policy` | Allowed `run_projection_action` values |
|---|---|
| `STRICT_CEILING` | `BLOCK_NEXT_CALL` only |
| `EMPIRICAL_RUN_CEILING` | `BLOCK_NEXT_CALL`, `REQUIRE_APPROVAL`, `ALERT_ONLY` |
| `ADAPTIVE_CEILING` | `BLOCK_NEXT_CALL`, `REQUIRE_APPROVAL`, `ALERT_ONLY` |
| `SHADOW_ONLY` | `ALERT_ONLY` only（shadow 不阻擋，僅紀錄） |

DSL evaluator 在 bundle load 時驗證該對應；違反 → audit event `bundle_validation_failed` + refuse_to_load。

---

## §6. Proto schema additive diff

### 6.1 `proto/spendguard/sidecar_adapter/v1/adapter.proto` `DecisionResponse.Decision` enum

```protobuf
// === v1alpha2 additive change ===
// Existing values 1-5 unchanged. No renumbering.
enum Decision {
  DECISION_UNSPECIFIED = 0;
  CONTINUE = 1;
  DEGRADE = 2;
  SKIP = 3;
  STOP = 4;
  REQUIRE_APPROVAL = 5;
  // NEW (v1alpha2): explicit decision when stop is driven by run projection,
  // distinct from per-call STOP. Effect lattice still maps this to the same
  // terminal STOP effect; old or unsupported consumers must fail closed rather
  // than gracefully continuing. Implemented by SLICE_02 commit `c50b911`
  // (sidecar exhaustive StopRunProjection match), egress_proxy commit `3035b54`
  // (StopRunProjection blocked; unknown decision → fail-closed sidecar error),
  // and SLICE_09 commit `cc20cb4` (projector activation).
  // New field reason_codes carries the RUN_* code explicitly.
  // STOP semantics 完全等同 v1alpha1 STOP; 此值僅供 dashboard / SIEM
  // 細分顯示用，effect lattice precedence 不變.
  STOP_RUN_PROJECTION = 6;
}
```

> Reviewer 注意：考慮過不加 `STOP_RUN_PROJECTION`、純 reuse `STOP` + `reason_codes`。**選擇加** 是因為：(a) dashboard UX 需要 first-class enum 分類；(b) 對 SIEM 過濾規則「RUN_* triggered stop vs per-call stop」更直接；(c) `reason_codes` 仍存在所以 backward-compat 不破。代價：DecisionResponse 多 1 個 enum value。若您 prefer 不擴 enum，告訴我，改為純 reason_codes 走法。

### 6.2 `proto/spendguard/sidecar_adapter/v1/adapter.proto` `DecisionResponse` 新欄位

```protobuf
// === v1alpha2 additive change ===
message DecisionResponse {
  // ... existing fields 1-15 unchanged ...

  // NEW (v1alpha2): which RUN_* code triggered this decision. Empty if
  // decision is per-call (no RUN_* match).
  // Tag 16 chosen as next available; v1alpha1 reserved tags 1-15 for
  // hot-path fields per common.proto §field-number convention.
  string run_code_triggered = 16;  // "RUN_BUDGET_PROJECTION_EXCEEDED" | "RUN_DRIFT_DETECTED" | "RUN_STEPS_EXCEEDED" | ""
}
```

### 6.3 Contract bundle wire schema (YAML; per v1alpha1 §3)

#### 6.3.0 Wedge surface (SLICE_02)

SLICE_02 shipped the additive YAML fields and the declarative
`when.claim_amount_atomic_gt` / `when.claim_amount_atomic_gte` predicate
surface only. This is the only condition surface honored by the
SLICE_02 hot-path evaluator; it preserves the §2 row-18 invariant that
v1alpha1 quickstart bundles still parse and evaluate correctly under a
v1alpha2-capable sidecar.

Supported SLICE_02 shape:

```yaml
apiVersion: spendguard.ai/v1alpha2  # bump from v1alpha1
kind: Contract

spec:
  # ... existing v1alpha1 fields unchanged ...

  # NEW (v1alpha2):
  prediction_policy: STRICT_CEILING  # default; one of STRICT/EMPIRICAL/ADAPTIVE/SHADOW

  # RUN_* rules are not authored as claim-amount threshold rules. SLICE_09+
  # wires run_cost_projector output directly into the evaluator; see §6.3 for
  # the non-authoritative shape and §8.4 for CEL upgrade guidance.
  rules: []
```

SLICE_02 ships the CEL helper structs (`RunProjection`,
`PredictionContext`) and `into_cel_context` in
`services/sidecar/src/contract/cel_helpers.rs` with unit-test coverage.
The hot-path evaluator does NOT yet invoke `Program::execute` against
contract YAML — the v1alpha2 declarative form
(`when.claim_amount_atomic_gt` / `when.claim_amount_atomic_gte`) remains
the only honored condition surface in SLICE_02.

v1alpha2 contract authors writing `condition: <cel-expr>` in v1alpha2
bundles BEFORE SLICE_09 lands would silently get no enforcement — every
rule with a CEL condition would be ignored at evaluation time and the
operator would observe the contract as a no-op without any audit
breadcrumb. To prevent the silent-ignore foot-gun, **the SLICE_02
parser asymmetrically handles the `condition:` field**:

**On v1alpha1 contracts**, `condition:` fields are LEGACY (per
v1alpha1 spec §18 quickstart, which documents the `condition: |` CEL
form in rule bodies) and the wedge evaluator falls back to declarative
`when:` form — a `tracing::warn!` is emitted on parse but the contract
loads successfully. This preserves the §2 row-18 invariant
("v1alpha1 quickstart 100% 正確") and is consistent with M1's
forward-compat-hint pattern (parse.rs:247-258).

**On v1alpha2 contracts**, `condition:` fields are REJECTED with
`bundle_validation_failed` because v1alpha2 explicitly opts into the
predictor-aware surface; SLICE_09 will wire the CEL accessor surface
listed above (`run_projection.*`, `prediction.*`). The rejection error
string:

```
bundle_validation_failed: rule '<rule-id>' uses CEL `condition:`
field; CEL conditions wired in SLICE_09 — use claim_amount_atomic_gt /
claim_amount_atomic_gte under `when:` in SLICE_02. See
contract-dsl-spec-v1alpha2.md §6.3 SLICE_02-vs-SLICE_09 wiring
boundary.
```

Cross-reference: enforcement in
`services/sidecar/src/contract/parse.rs` (rule-iteration block); test
coverage in `parse.rs::tests::rejects_v1alpha2_contract_with_cel_condition_field`
and `v1alpha1_contract_with_cel_condition_field_parses_with_warn`.

#### 6.3.1 Post-SLICE_09 CEL accessor capability

SLICE_09 wires CEL-condition support (`condition: "<cel-expr>"` form in
rule body) after `run_cost_projector` populates `RunProjection` /
`PredictionContext` per decision.

CEL helpers新增（per v1alpha1 §14 evaluation.helpers）：

```cel
run_projection.at_decision_micros  // 等於 audit_outbox.run_projection_at_decision_atomic / pricing.scale
run_projection.predicted_remaining_steps
run_projection.steps_completed_so_far
prediction.tier   // "T1" | "T2" | "T3"
prediction.strategy_chosen  // "A" | "B" | "C"
prediction.confidence  // 0.0-1.0
```

Post-SLICE_09 non-authoritative pseudo-shape. This illustrates the predicate
that run_cost_projector evaluates internally; do not paste this into a
contract bundle because v1alpha2 sidecars fail closed on `condition:` per §8.4.

```yaml
rules:
  - id: stop_when_projection_exceeded
    priority: 1100
    condition: |
      run_projection.at_decision_micros >
      budget("daily_usd").remaining.amountMicros
    effect:
      decision: stop
      reasonCode: RUN_BUDGET_PROJECTION_EXCEEDED
    run_projection_action: BLOCK_NEXT_CALL
```

### 6.4 v1alpha1 contracts 在 v1alpha2 evaluator 下的 default 填充

當 v1alpha2 evaluator 載入 `apiVersion: spendguard.ai/v1alpha1` contract：

- `prediction_policy` 自動填 `STRICT_CEILING`（per §4 default）
- 任何 rule 沒寫 `run_projection_action` → 自動填 `BLOCK_NEXT_CALL`
- 任何 rule 沒有 `RUN_*` code reference → 不主動套用 RUN_* logic（v1alpha1 不知道 run-level projection）

行為效果：v1alpha1 contract 在 v1alpha2 evaluator 下 produce 與 v1alpha1 evaluator 100% byte-identical audit row（除了新欄位 NULL）。

**Round-1 fix M1 — observability for ignored forward-compat hints**:

When a v1alpha1 contract specifies `spec.prediction_policy` or any rule
sets `run_projection_action`, the parser default-fills as above AND
emits a `tracing::warn!` event identifying:

- the source apiVersion (so the operator sees which bundle is on the wire)
- the ignored hint value
- the contract id (top-level field) or rule id (per-rule)
- guidance to bump apiVersion to `spendguard.ai/v1alpha2` to activate the
  declared value

Behavior is unchanged (v1alpha1 still resolves to
`STRICT_CEILING + BLOCK_NEXT_CALL` regardless of hint), but the
observability gap is closed — pre-round-1 the value was silently
discarded with no log breadcrumb.

---

## §7. DSL evaluator extension

`services/sidecar/src/contract/evaluate.rs` 增量點：

### 7.1 SLICE 02 階段（internal compatibility path）

```rust
// Pseudo-code; actual SLICE 02 PR will follow Rust style of evaluate.rs

fn handle_run_code(code: RunCode, action: RunProjectionAction, ctx: &EvalContext) -> Effect {
    match action {
        RunProjectionAction::BlockNextCall => Effect::Stop {
            reason_codes: vec![code.as_str().into()],
            terminal: true,
        },
        RunProjectionAction::RequireApproval => Effect::RequireApproval {
            reason_codes: vec![code.as_str().into()],
            ttl: ctx.contract.approval_ttl,
            approver: ctx.contract.approver_tier,
        },
        RunProjectionAction::AlertOnly => Effect::Continue {
            reason_codes: vec![code.as_str().into()],
            alert: true,  // dashboard / webhook 由 audit row 觸發
        },
    }
}
```

SLICE_02 merge `d5c5434` did not implement RUN_* code trigger logic（no run_cost_projector yet）—— 三 codes 進 evaluator 後 route through above function for internal compatibility. Actual emission was activated by SLICE_09 merge `6407648` / commit `cc20cb4`; unsupported or unknown response enum handling is fail-closed, not provider-call pass-through, per egress_proxy commit `3035b54`.

### 7.2 Latency 預算

v1alpha1 §14 50ms p99 latency budget 包含 evaluation 5ms。v1alpha2 evaluator 對 `handle_run_code` 路徑 + RUN_* code condition evaluation 額外 < 1ms（純 enum dispatch + reason_codes vec push）—— 不打破 §14 SLO。

---

## §8. Migration: v1alpha1 contracts encountering v1alpha2 codes 的行為

### 8.1 Forward path：v1alpha1 evaluator + v1alpha2 contract bundle

v1alpha1 sidecar binary 收到 v1alpha2 contract bundle（`apiVersion: spendguard.ai/v1alpha2`）：

- 走 v1alpha1 §3.3 `capability_required` mismatch path（sidecar bundle loader 認 `apiVersion` 不在白名單）
- emit audit event `bundle_validation_failed`（v1alpha1 既有 event）
- **refuse to load**（per sidecar §3.3 invariant）

這是 v1alpha1 既有保護 —— v1alpha2 contract 不會誤跑在 v1alpha1 sidecar 上。

### 8.2 Backward path：v1alpha2 evaluator + v1alpha1 contract bundle

per §6.4，v1alpha2 evaluator 對 v1alpha1 bundle：

- `apiVersion: spendguard.ai/v1alpha1` 識別後走 v1alpha1 defaults 填充
- 行為 100% byte-identical with v1alpha1 evaluator
- SLICE 02 acceptance 必含 byte-identical regression test 對 8+ demo modes

### 8.3 Rolling upgrade 順序

| Step | Order | Why |
|---|---|---|
| 1 | Deploy v1alpha2 sidecar binary（仍跑 v1alpha1 contracts） | sidecar 必須先升好，才能處理新 contract |
| 2 | 驗證 v1alpha1 contracts byte-identical 行為 | 8+ demo modes regression |
| 3 | Deploy 第一個 v1alpha2 contract bundle（with policy + RUN_* rules） | 客戶 opt-in 才會走新邏輯 |
| 4 | 觀察 audit chain 新欄位 + RUN_* codes 行為 | 7 日 trial period |
| 5 | 推廣至更多 tenants | |

不允許「先 deploy v1alpha2 contract，後升 sidecar」順序 —— 由 §8.1 mismatch refuse 機制強制。

### 8.4 SLICE_02 upgrade path: `condition:` validation surface

Operators upgrading from pre-SLICE_02 sidecars MUST scan contract
bundles before rolling the new sidecar binary:

```sh
grep -RInE '(^|[[:space:]])condition[[:space:]]*:' /path/to/contract-bundles
```

If a bundle is `apiVersion: spendguard.ai/v1alpha2`, replace
`condition:` rules with the SLICE_02 declarative `when:` form before
deploying. A v1alpha2 bundle that still carries `condition:` fails to
load with:

```text
bundle_validation_failed: rule '<rule-id>' uses CEL `condition:`
field; CEL conditions wired in SLICE_09 — use claim_amount_atomic_gt /
claim_amount_atomic_gte under `when:` in SLICE_02.
```

If a bundle is `apiVersion: spendguard.ai/v1alpha1`, `condition:` is
treated as a legacy quickstart field: the bundle loads, the wedge
evaluator uses the declarative `when:` fallback, and the parser emits a
warning so operators can clean up before moving the bundle to
v1alpha2.

---

## §9. GA prerequisites

於 §0.3 列出。本 spec 不重複。

---

## §10. Adoption history

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder — filled during Codex / panel adversarial review rounds per HANDOFF §9) |

---

## §11. Lock 後的下一步

1. SLICE_02 implementation（merge `d5c5434`）：proto additive 補丁 + DSL evaluator extension（internal compatibility path）+ 既有 8+ demo modes regression; production emission/blocked-call behavior was activated by SLICE_09/10 (`6407648` / `c649196`)
2. SLICE 02 acceptance：v1alpha1 contract 在 v1alpha2 evaluator 下 byte-identical audit row
3. SLICE 09 PR：run_cost_projector 接入 + RUN_* codes 真正觸發
4. 客戶 v1alpha2 quickstart template（per v1alpha1 §18 風格）—— 加 `prediction_policy: STRICT_CEILING` + 一個 sample RUN_BUDGET_PROJECTION_EXCEEDED rule

---

*Document version: contract-dsl-spec-v1alpha2 (DRAFT) | Drafted: 2026-05-29 | Strictly additive over v1alpha1; preserves all v1alpha1 invariants per §2 | Critical surface: §3 new codes; §4-§5 policy enums; §6 proto additive diff; §8 migration | Branch: `design/predictor-upgrade`*
