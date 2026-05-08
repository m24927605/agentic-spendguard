# Contract DSL Specification — v1alpha1 (LOCKED)

> 🔒 **Status: LOCKED implementation spec**  
> **Lock date**: 2026-05-06  
> **Lock judgment basis**: Codex round-4 minimal verification — 「Lock v3 as implementation spec after the v3.1 clarification patch. Do not create v4 unless POC breaks one of the transaction invariants.」  
> **Adoption history**: Round 1/2/3/4 採納率 100%/100%/100%/100%（4 輪零實質反駁）  
> **Companion**: `agent-runtime-spend-guardrails-complete.md` (v1.3 strategy)  
> **Compatibility policy**: alpha — bundle pinning + audit schema immutable + breaking changes require migration tool + old/new evaluator dual-run + contract diff report

---

## 0. Lock status & GA prerequisites

### 0.1 What this spec covers

完整的 Contract DSL 設計，可進入 reference implementation POC + first customer design partner onboarding。

### 0.2 GA 前置條件（Codex round-4 規定）

進入 GA 路徑前，下列 5 項必達成：

1. **Durable stage-output persistence** before any runtime mutation（§14 chaos test 依賴）
2. **Shadow and enforce ledgers physically separate namespaces**（§3 modeSemantics 強制）
3. **mTLS/SPIFFE workload transport auth** for grant retrieval（§8 補入）
4. **Run §14 7 chaos tests + 3 GA-gating tests**（multi_tenant_noisy_neighbor / clock_skew_window_boundary / jwt_exp_nbf_skew）
5. **Measure actual warm/cold latency**；replace `cold_start_p99_ms: 150` with observed SLO before GA（§13）

### 0.3 何時可能需要 v2

只有以下情況才開啟 v2 spec 修正：
- POC 在 chaos test 下 break decision transaction invariants
- 發現第 4 個（v0→v3 已找出 3 個）新 high-irreversibility gap
- Production data 揭示 schema 重大缺陷

正常情況下，v1alpha1 spec 直接演進至 v1beta1（無 break change）→ v1（GA）。

---

## 1. Context（self-contained）

### 1.1 產品

**Agent Runtime Spend Guardrails** — 在 agent step / tool call / reasoning spend 邊界做 budget decision、policy enforcement、approval、rollback、audit 的 runtime 安全層。

### 1.2 三支柱閉環

Predict (Risk Band) + Control (Decision-at-Boundary) + Optimize (Candidate Generator)。**主動排除 Continuous Learning**（合規難度創造 ceiling 而非 moat；見策略文件 §22.4）。

### 1.3 Contract DSL 在 T→L→C→D→E→P 中的角色

```
T (Trace) → L (Ledger) → C (Contract DSL) → D (Decision) → E (Evidence) → P (Proof)
                              ↑ 本 SPEC
```

### 1.4 核心哲學

> **Decision 必須是完整、可重播、idempotent 的 transaction**，且 audit 在 effect publish **之前**。  
> **Mode、multi-budget、provider commit** 三個語意必須顯式定義。  
> **Trust boundary** 用業界標準（OAuth/JWT、SPIFFE、mTLS），不自製。  
> **Reservation 是 authorization** 而非 forecast；overrun 拆 pre-call 與 post-commit 兩相。

---

## 2. Irreversibility ranking（最終版）

| # | 決策 | spec 位置 |
|---|---|---|
| 1 | Decision transaction state machine | §6 |
| 2 | Reservation as authorization + overrun phasing | §7 |
| 3 | Mode semantics（shadow vs enforce 副作用） | §3 |
| 4 | Multi-budget atomic reservation | §4 |
| 5 | Provider commit uncertainty | §5 |
| 6 | Inputs schema（money / time / non-monetary unit） | §12 |
| 7 | Effect lattice + same-type merge | §10 |
| 8 | Effect schema（含 idempotency / mutation patch constraints） | §11 |
| 9 | Sub-agent trust boundary（JWT + SPIFFE + mTLS） | §8 |
| 10 | Model capability matrix versioning + compatibility channels | §9 |
| 11 | Trigger points | §15 |
| 12 | Language paradigm（YAML + CEL） | §16 |
| 13 | Audit format（含 hmac salt） | §13 |
| 14 | Pure vs Stateful | pure (locked) |

---

## 3. Mode Semantics

```yaml
spec:
  mode: shadow                                    # shadow | enforce
  
  modeSemantics:
    shadow:
      applyEffect: false
      ledger: virtual_reservation_only
      audit: shadow_event
      visibility: separated
      crossModeBudgetIsolation: required          # ⚠ NOT tenant-overridable
      exposureReporting:                           # v3.1 patch
        enabled: true
        source: virtual_ledger
        neverCountsTowardEnforceBudget: true
    
    enforce:
      applyEffect: true
      ledger: real_reservation_and_commit
      audit: production_event
      visibility: production
  
  modeMigration:
    fromShadow:
      requireMinObservationTraces: 1000
      requireCalibrationErrorBelow: 0.05
      requireExplicitApprover: tenant-admin
    canaryEnforce:
      percentTraffic: [1, 10, 50, 100]
      autoRollbackOnAnomaly: true
  
  # === v1.1: Ledger capability requirement per mode ===
  ledgerCapabilityRequirement:
    enforce_mode:
      requires: [strong_global, single_writer_per_budget]   # per Ledger §18 / Sidecar §12.5
      forbidden: [eventual]
      rationale: |
        Hard enforcement 須 strong consistency；eventual ledger 跨 region 雙花
        會破壞 budget 不變式
    shadow_mode:
      tolerates: [strong_global, single_writer_per_budget, eventual]
      rationale: shadow 不影響 production budget；可用 eventual ledger
    
    sidecar_capability_filter:
      reference: Sidecar §12.5 hard_enforcement_filter
      action_on_mismatch: refuse_to_load_contract + audit_event(capability_mismatch)
```

### 3.1 為什麼 `crossModeBudgetIsolation` 不可 tenant override

Compliance 想看 exposure → 用 `exposureReporting` virtual ledger projection（不污染 production budget）。允許 override = shadow event 計入 enforce budget = 「我以為我在 dry-run」幻覺破裂 = 真實預算被假事件吃掉。

### 3.2 Shadow / Enforce ledger 物理分離

Two namespaces：
- `ledger.shadow.{tenant}.{budget}` — virtual reservations, never charges
- `ledger.enforce.{tenant}.{budget}` — real spend tracking

GA prerequisite #2：實作層必須是物理分離的儲存空間，不只是邏輯欄位區分。

---

## 4. Multi-Budget Atomic Reservation

```yaml
spec:
  reservationSet:
    strategy: all_or_nothing                      # all_or_nothing | best_effort_with_compensation
    
    budgets:
      - tenant_daily_usd
      - org_global_usd
      - parent_sub_agent_grant
    
    orderingMatters: false
    
    partialFailure:
      action: release_all_and_fail_closed
      compensatingActions:
        - reverse_all_partial_reservations
        - emit_partial_failure_audit_event
        - retry_after_backoff_if_idempotent
    
    deadlock_avoidance:
      strategy: lexicographic_lock_ordering
      timeout_ms: 50
      onTimeout: release_all_and_fail_closed
  
  crossBudgetConflict:
    most_restrictive_wins: true
    audit_records_blocking_budget: true
```

任一 budget exhausted = stop。Partial reserve 不允許 — 防「幽靈預算」（已扣未授權）。

---

## 5. Provider Commit Uncertainty

```yaml
spec:
  commitStateMachine:
    states:
      - unknown                                   # llm.call.post 未收到回應
      - estimated                                  # 用 risk.p50/p90 估算先記
      - provider_reported                          # provider response header 含 usage
      - invoice_reconciled                         # 月底與 provider invoice 對齊
    
    transitions:
      unknown_to_estimated:
        condition: timeout_after_call_pre_ms_exceeded
        action: 
          commit_amount: risk.p90                  # 保守估高
          mark_state: estimated
      
      estimated_to_provider_reported:
        condition: out_of_band_provider_response_received
        action:
          adjust_commit_amount: provider_reported_value
          delta_to_audit: true
      
      any_to_invoice_reconciled:
        condition: monthly_invoice_received
        action:
          final_adjustment: invoice_value
          tolerance_micros: 10000                  # $0.01 容忍
          large_delta_alert: true
    
    estimatedCommitPolicy:
      timeout_after_call_pre_ms: 30000
      conservative_estimation: use_p90
      audit_state_in_event: required
  
  reconciliationStrategy:
    schedule: monthly
    delta_handling:
      under_tolerance: silent_adjust
      over_tolerance: audit_event + tenant_notify
      systematic_delta: calibrate_pricing_version (full freeze: pricing_version + price_snapshot_hash + fx_rate_version + unit_conversion_version, see Ledger §13)
```

對齊 Stripe PaymentIntent / idempotency pattern 的 billing-style eventual reconciliation。

### 5.1a Refund and Dispute Handling

除了 4 個 commit states，post-commit 仍可能發生 provider 退款或客戶 dispute。對應 Ledger §10 operation_kinds（`refund_credit` / `dispute_adjustment` / `compensating`）：

```yaml
refund_policy:
  trigger: provider_issues_credit_for_prior_charge
  effect:
    emit_ledger_operation: refund_credit
    direction: credits adjustment account → increases available_budget
  audit:
    event_type: spendguard.refund.credit_received
    captured_fields: [provider_credit_id, credit_amount_atomic, currency, credited_at, original_reservation_id]
  cross_reference: Ledger §10 refund_credit operation

dispute_policy:
  trigger: customer_disputes_charge
  flow:
    case_open:
      emit_ledger_operation: dispute_adjustment
      direction: temporary debit on adjustment until resolved
      effect: amount conditionally held
    case_resolved_in_favor_of_customer:
      emit_ledger_operation: refund_credit
      result: budget restored
    case_resolved_against_customer:
      emit_ledger_operation: compensating (reverse the dispute_adjustment)
      result: original commit stands
    case_withdrawn:
      emit_ledger_operation: compensating
  audit:
    event_type: spendguard.dispute.{requested|granted|denied|withdrawn|resolved}
    captured_fields: [provider_dispute_id, case_state, resolved_at, resolution_amount_atomic]
  cross_reference: Ledger §5.2 ledger_transactions provider_dispute_id + case_state + resolved_at
```

**Contract DSL 不直接控制 refund / dispute** — 兩者由 provider lifecycle 觸發，sidecar 接收 provider webhook 後 emit 對應 ledger operation 與 audit event。Contract 可寫 policy 監測 dispute 狀態（如「dispute_adjustment > $X 須通知 tenant-admin」）。

---

## 6. Decision Transaction State Machine

```yaml
spec:
  decisionTransaction:
    idempotencyKey: trace.eventId
    
    stages:
      - id: snapshot
        captures: [event_time, evaluator_time, ledger_state, risk_band, contract_bundle_signature, pricing_version]
        atomicity: required
        output_persisted: snapshot_hash
      
      - id: evaluate
        runs: predicate evaluation against snapshot
        timeout_ms: 5
        output_persisted: matched_rules_hash
      
      - id: prepare_effect
        runs: compute effect (mutation patch, decision) — pure, no runtime side effect
        deterministic: required
        output_persisted: effect_hash
      
      - id: reserve
        runs: ledger atomic reservation (with reservationSet semantics)
        timeout_ms: 20
        output_persisted: reservation_id
      
      - id: audit_decision                         # ⚠ 在 publish 之前，不可 batch（語意），可 batch 傳輸/匯出
        runs: append-only event emit
        atomicity: required
        durability: durable_outbox_or_wal_sync
        output_persisted: audit_decision_event_id
      
      - id: publish_effect                         # 真實 mutate runtime / signal stop / queue approval
        side_effect: yes
        idempotency: required (via effect_hash)
      
      - id: commit_or_release
        commit: 在 llm.call.post 觸發
        release: timeout / runtime_error / run_aborted
        with_commit_state_machine: see §5
      
      - id: audit_outcome
        captures: [commit_state, commit_amount, delta, runtime_outcome]
    
    retry:
      strategy: idempotent_by_decision_id
      max_attempts: 3
      backoff_ms: [10, 50, 200]
      on_max_attempts_exhausted: fail_closed → stop + alert
    
    rollback:
      triggers: [reserve_failure, audit_decision_failure, publish_effect_failure]
      compensating_actions:
        - reverse_reservation_if_held
        - emit_rollback_audit_event
    
    recovery:
      strategy: resume_from_last_successful_stage
      requires_persisted_outputs_through: audit_decision
      on_resume_clock_skew: re_snapshot_if_drift_exceeded_ms: 1000
```

### 6.1 為什麼 audit_decision 必須在 publish_effect 前

- Publish 後 audit 失敗 = effect 已生效但無記錄 → 法務翻車
- Publish 前 audit 失敗 = transaction roll back，可重試
- 不變式：**「無 audit 則無 effect」**

### 6.2 Audit batching 規則（v3.1 補註）

- ❌ 語意 batch（多 decision 共用一 audit event）：禁止
- ✅ Transport batch / export batch：允許 — durable outbox / WAL 同步落盤後 async ship 至 warehouse

每個 decision 必須有 1:1 對應的 audit event；ship 至下游可 batch。

### 6.3 Stage Persistence Deployment Matrix（v1.1 補註）

`audit_decision` stage 的 durable store **依 deployment mode 決定**，由 Sidecar §6.2 `durability_mode_selection` matrix 強制對應：

| Deployment Mode | audit_decision durable store | 對應 storage class |
|---|---|---|
| `k8s_saas_managed` | `remote_decision_journal_ack`（preferred）| Trace §10.2 immutable_audit_log |
| `k8s_self_hosted` | `remote_decision_journal_ack` 或 `canonical_ingest_ack` | 同上 |
| `lambda` | `canonical_ingest_ack`（only allowed） | 同上 |
| `cloud_run_or_container_serverless` | `canonical_ingest_ack` 或 `remote_decision_journal_ack` | 同上 |
| `air_gapped` | `persistent_local_wal_with_replay_guarantee` | local immutable WAL |
| `vm_or_bare_metal` | 三選一（per matrix） | 對應 |

**不變式**：無論哪個 mode，`audit_decision` 必須在 `publish_effect` 之前 durable ack（per §6.1「無 audit 則無 effect」）。

**Stage ownership 完整對應**（與 Sidecar §12.1 對齊）：

| Stage | Owner | Storage |
|---|---|---|
| snapshot | local_sidecar | in-process（出 transaction WAL） |
| evaluate | local_sidecar | in-process |
| prepare_effect | local_sidecar | in-process（pure compute） |
| reserve | remote_ledger | Ledger atomic write |
| **audit_decision** | **remote_durable_store**（per §6.3 matrix） | per Sidecar §6.2 |
| publish_effect | in_process_adapter | runtime mutation |
| commit_or_release | remote_ledger | Ledger commit |
| audit_outcome | remote_durable_store | 同 audit_decision target |

**Cross-reference**：完整 deployment-specific 行為見 Sidecar §6.2 + Sidecar §12.1。

---

## 7. Reservation Authorization 兩相

```yaml
budgets:
  - id: tenant_daily_usd
    window: {type: calendar_day, timezone: UTC}
    limit: {amountMicros: 1000000000, currency: USD}
    
    reservation:
      amountSource: risk.p95
      requireHardCapWhenAvailable: true
      capUnavailablePolicy: soft_authorization_with_overrun_policy
      ttl: PT10M
      releaseOn: [commit, timeout, runtime_error, run_aborted]
    
    # === Phase A: call 前估超出 reserved ===
    preCallTopUpPolicy:
      type: require_approval_above_reserved
      maxTopUpRatio: 0.25
      approver: tenant-admin
      ttl: PT5M
    
    # === Phase B: call 後實際超支 ===
    overrunPolicy:
      phase: post_commit
      type: record_violation_and_debt
      debt:
        ledger: tenant_daily_usd
        absorb_in_next_window: true
        max_debt_ratio: 0.10
      onDebtRatioExceeded: stop_and_escalate
      violation:
        emit_event: true
        notify: tenant-admin
        block_subsequent_until_resolved: true
    
    commit:
      on: llm.call.post
      reconciliation:
        with: provider_invoice
        tolerance_micros: 10000
    
    pricing_version: focus_v1_2_2026_05
```

`top_up_with_approval` 只能用在 call 前；call 後只能記 debt/violation。

---

## 8. Sub-Agent Trust Boundary（v3.1 補入 mTLS）

```yaml
spec:
  subAgentBudgetPropagation:
    transport: budget_grant
    
    grant:
      tokenFormat: jwt_access_token_profile        # RFC 9068
      algorithm: ed25519
      claims:
        # === Standard JWT claims ===
        iss, sub, aud, exp, nbf, iat, jti
        # === OAuth Token Exchange (RFC 8693) ===
        act: { sub: parent_agent_id }
        # === Custom claims ===
        tenant_id, scope, max_amount, reservation_id, parent_run_id
    
    workloadIdentity:                              # v3.1 補強
      spiffeId: optional
      spiffeFormat: x509_svid
      # SPIFFE 用於 workload-level identity
    
    transportSecurity:                             # v3.1 NEW
      workloadToWorkload:
        mtls: required
        identitySource: spiffe_x509_svid | platform_mtls
        # SPIFFE X.509-SVID 適合 workload mTLS
        # JWT grant 是 budget authorization，不是 transport auth
    
    freshness:
      ledgerSnapshotHash: optional
      freshUntilMs: 1000
      finalReserveStillRequiresLedger: true
    
    propagation:
      via: openTelemetry_baggage
      baggage_keys:
        - trace_id
        - parent_run_id
        - budget_grant_jti                         # 引用，不攜帶 token 本體
      grant_retrieval:
        from: secure_grant_store
        transport: mtls_required                   # v3.1
    
    verification:
      child_must_verify_jwt_signature: true
      child_must_verify_jti_not_revoked: true
      child_must_recheck_ledger_for_final_reserve: true
    
    revocation:
      parent_can_revoke: true
      revocation_propagates_to_in_flight_children: true
      revocation_list: distributed_cache
```

**三層信任**：
1. **mTLS（transport）** — 確認 caller 是合法 workload（SPIFFE X.509-SVID）
2. **JWT（authorization）** — 攜帶 budget grant 內容
3. **Ledger recheck（final authorization）** — 不信賴 cached snapshot 的 max_amount

---

## 9. Model Capability Matrix（v3.1 補強 compatibilityClass）

```yaml
apiVersion: spendguard.ai/v1alpha1
kind: ModelCapabilityMatrix

metadata:
  name: low_cost_research
  version: 2.4.0

spec:
  alias: low_cost_research                         # logical alias（K8s Service-style）
  
  compatibilityClass:                              # v3.1 NEW
    id: low_cost_research.v2.research_grade
    requires:
      capabilityNonRegression: true
      qualityClassNonRegression: true
      costProxyIncreaseMaxRatio: 0                 # 不可漲價
      hardCapSupportNonRegression: true
      dataResidencyCompatible: true
      providerAllowlistCompatible: true
  
  reviewPolicy:                                    # v3.1 NEW
    reviewOnCostDecreaseAboveRatio: 0.50           # 大幅降價也應 review
    reviewReason: major_cost_proxy_change
    # 因大幅降價可能代表模型/供應商切換，品質與 compliance 風險仍需 review
  
  compatibilityChannels:
    - name: stable
      versionRange: "2.x"
      breakingChangePolicy: require_review
    - name: pinned
      versionRange: "=2.4.0"
      breakingChangePolicy: blocked
    - name: latest
      versionRange: "*"
      breakingChangePolicy: opt_in_only
  
  changeKinds:
    breaking:                                      # 必須 contract review
      - capability_removed
      - max_input_tokens_decreased
      - cost_proxy_increased
      - quality_class_decreased
    non_breaking:                                  # 在 compatibility channel 內 auto-roll
      - new_candidate_added_with_equal_or_better_cost_quality
      - max_input_tokens_increased
      - cost_proxy_decreased_within_review_threshold
      - quality_class_unchanged
      - failover_chain_extended
  
  candidates:                                      # K8s EndpointSlice-style
    - provider: anthropic
      model: claude-haiku-4-6-20260301
      pricing_version: focus_v1_2_2026_05
      capabilities:
        max_input_tokens: 200000
        max_output_tokens: 8192
        supports_reasoning: false
        supports_tools: true
        supports_vision: true
      cost_proxy:
        input_per_mtok_micros: 250000
        output_per_mtok_micros: 1250000
      quality_class: research_grade
```

Contract 引用：

```yaml
rules:
  - effect:
      decision: degrade
      mutate:
        modelAlias:
          name: low_cost_research
          channel: stable                          # auto-roll within stable
          # alternative: version: "2.4.0" (pinned)
```

---

## 10. Effect Lattice + Same-Type Merge

```yaml
combining:
  algorithm: all_matches_effect_lattice
  precedence: [stop, require_approval, skip, degrade, continue]
  # ⚠ governance strictness, not pure cost strictness
  # approval gate 不應被 skip 繞過
  
  same_type_merge:
    stop:
      strategy: first_wins
      reason_aggregation: list_all
    
    require_approval:
      strategy: most_restrictive_role
      ttl_aggregation: min
      role_priority: [tenant-admin, platform-eng, team-lead]
    
    skip:
      strategy: union_of_skip_targets
    
    degrade:
      mutate_merge:
        modelAlias:
          default: reject_on_disagree
          allowPriorityWinsIf:
            sameCompatibilityClass: true           # 兩 alias 在同 compatibility class 內
            explicitlyConfigured: true
        maxOutputTokens: min_value
        reasoningEffort: min_level
        temperature: reject_on_disagree
    
    continue:
      strategy: noop
  
  conflicts:
    definition: same_type_mutations_on_same_target_path_that_are_not_commutative
    on_conflict: reject_bundle
```

---

## 11. Effect Schema & Mutation Constraints

```yaml
decisions:
  continue:
    terminal: false
    mutates_request: false
    requires_approval: false
    idempotent: true
    rollback: not_applicable
  
  degrade:
    terminal: false
    mutates_request: true
    mutation_kinds: [model_alias, max_output_tokens, reasoning_effort, temperature]
    requires_approval: false
    idempotent: true                               # via deterministic patch
    rollback: revert_mutation_record
  
  skip:
    terminal: false
    mutates_request: false
    skip_target: optional_step
    idempotent: true
    rollback: not_applicable
  
  stop:
    terminal: true
    mutates_request: false
    requires_approval: false
    idempotent: true
    rollback: not_applicable
  
  require_approval:
    terminal: false
    mutates_request: false
    requires_approval: true
    approval_ttl_max: PT24H
    fallback_on_timeout: [stop, continue]
    idempotent: true
    rollback: cancel_approval_request

# === Approval semantics (v2 → v3) ===
approval:
  resumePolicy: re_evaluate_before_resume
  holdReservationWhilePending: true
  budgetSnapshotPolicy: refresh_on_resume
  on_resume_decision_change:
    policy: defer_to_higher_priority_decision
  ux_framing: |
    "Approval approves an exception attempt, not guaranteed execution.
     Re-evaluation may still block the attempt."

# === Mutation patch constraints ===
mutate:
  allowed_targets: [modelAlias, maxOutputTokens, reasoningEffort, temperature, topP, stop_sequences]
  forbidden_targets: [tools, tool_choice, system_prompt, messages, response_format]
  
  patch_format: rfc6902_restricted
  allowed_operations: [test, add, replace]
  forbidden_operations: [move, copy, remove]
  array_index_patch: forbidden
  require_test_guard: true
  
  determinism:
    same_input_same_output: required
    side_effects: forbidden
  
  snapshot_relative: true
```

---

## 12. Money / Time / Unit Schema

### 12.1 Money type

```yaml
Money:
  oneOf:
    - type: monetary
      amountMicros: integer                        # amount × 10^6（避免浮點）
      currency: ISO4217                            # USD, EUR, JPY, ...
    - type: token
      count: integer
      token_kind: input | output | reasoning | cached_input | total
      model_family: string                         # 不同 model 的 token 不可直接比
    - type: credit
      amount: integer
      credit_program: string
    - type: non_monetary
      unit: string
      amount: integer

# Cross-unit 比較必須先 normalize（透過 pricing_version conversion table）
# 不可在 hot path 動態查詢
```

CEL helper：

```cel
money("50.00", "USD")  // → {amountMicros: 50000000, currency: USD}
// money("$1,000") 不允許 — 必須 explicit currency
```

### 12.2 Time window types

```yaml
window:
  oneOf:
    - {type: calendar_day, timezone: IANA_tz}
    - {type: rolling, period: ISO8601_duration}
    - {type: calendar_month, timezone: IANA_tz}
    - {type: billing_cycle, anchor_day: 1-28}
```

---

## 13. Audit Schema

```yaml
DecisionAuditEvent:
  required:
    - event_id, event_time, evaluator_time, tenant
    - contract_bundle_signature, contract_versions
    - matched_rules: [{contract, rule_id, priority, decision}]
    - selected_effect: {decision, reason_codes, mutation_patch}
    - reservation: {reservation_id, amount, source, ttl_expires_at}
    - decision_transaction: {idempotency_key, stages_completed, retry_count}
    - pricing_version, evaluator_version, runtime
    - input_snapshot:
        ledger_state, risk_band, trace_metadata
        prompt_hash: hmac_sha256_with_tenant_salt   # 防 cross-tenant 反推
        snapshot_hash: sha256
    - stage_outputs:                                # 持久化每階段
        snapshot_hash, matched_rules_hash, effect_hash
        reservation_id, audit_decision_event_id, publish_id
    - commit:
        state: unknown | estimated | provider_reported | invoice_reconciled
        estimated_at, reconciled_at
        commit_amount, actual_vs_reserved_delta_micros
        overrun_event_id
  
  immutable: true
  append_only: true
  signed: ed25519
  
  # === v3.1 batching note ===
  semantic_batching: forbidden                      # 1 decision = 1 event
  transport_batching: allowed                       # durable outbox + async ship
  durability: write_ahead_log_sync_before_publish_effect

# === v1.1 Refund / Dispute / Region Failover events (per §5.1a + Sidecar §10 + Ledger §10) ===
RefundEvent:
  event_type: spendguard.refund.credit_received
  required:
    - event_id, event_time, tenant
    - provider_credit_id, credit_amount_atomic, currency
    - credited_at
    - original_reservation_id, original_ledger_transaction_id
    - ledger_compensating_transaction_id
  immutable: true
  signed: ed25519

DisputeEvent:
  event_type: enum [
    spendguard.dispute.requested,
    spendguard.dispute.granted,
    spendguard.dispute.denied,
    spendguard.dispute.withdrawn,
    spendguard.dispute.resolved
  ]
  required:
    - event_id, event_time, tenant
    - provider_dispute_id
    - case_state: enum [open, under_review, resolved_in_favor, resolved_against, withdrawn]
    - resolved_at (when applicable)
    - resolution_amount_atomic, currency (when applicable)
    - original_reservation_id, original_ledger_transaction_id
  immutable: true
  signed: ed25519

RegionFailoverEvent:                                  # Phase 2+ (per Sidecar §10 / Ledger §12)
  event_type: spendguard.region_failover_promoted
  required:
    - event_id, event_time, tenant
    - old_region_id, new_region_id
    - revocation_evidence: {endpoint_catalog, network_acl, signing_key}
    - new_fencing_epoch
    - signed_promotion_certificate
  immutable: true
  signed: ed25519
```

---

## 14. Performance & Latency

> **v1.1 capability assumption**：50ms p99 latency budget achievable **only when**：
> - Ledger advertises `single_writer_per_budget` 或 `strong_global`（per Ledger §18）
> - Sidecar 與 ledger 同 region（per Sidecar §10 region_affinity）
> - Cache warm（per Sidecar §14 prewarmRequired）
> 
> **若 ledger advertise `eventual`**：enforce mode 拒絕載入此 contract（per §3 `ledgerCapabilityRequirement`）；shadow mode 可運行但無 latency SLA。

```yaml
evaluation:
  predicateLanguage: cel
  inputSchema: spendguard.inputs.v1
  snapshot: at_event_start
  ledgerConsistency: strong
  
  schemaValidation:
    unknownField: compile_error
  runtimeMissing:
    missingAtRuntime: eval_error
  
  helpers:
    has: kept_as_macro                              # CEL built-in
    exists: 'exists(string_path) -> bool'           # v3 新增
    get: 'get(string_path, default) -> any'         # v3 新增
    coalesce: 'coalesce(a, b, ...) -> any'
  
  performance:
    prewarmRequired: true
    
    timeouts:
      warm:
        expression_eval_ms: 5
        snapshot_capture_ms: 5
        reservation_ms: 20
        audit_emit_ms: 10
        decision_boundary_total_p99_ms: 50
        decision_boundary_total_p50_ms: 25
      
      cold_start:
        decision_boundary_total_p99_ms: 200
        cold_start_p99_ms: 150
        status: hypothesis_until_poc_measured       # v3.1: 不宣稱實證
        applies_when: tenant_evaluator_cache_miss
    
    prewarm_strategy:
      load_compiled_predicates_eagerly: true
      pre_resolve_capability_matrix: true
      pre_check_ledger_connectivity: true
  
  onError:
    strategy: fail_soft
    decision: require_approval
```

⚠ **GA 前置條件 #5**：實測 warm/cold latency；用 observed SLO 取代 hypothesis 數值。Cold miss 不應納入 warm p99。

---

## 15. Trigger Points（abstract names）

| Trigger | 觸發時機 | 評估 policy？ |
|---|---|---|
| `run.pre` | agent run 開始前 | ✅ |
| `agent.step.pre` | 每個 agent decision step 前 | ✅ |
| `agent.step.post` | step 完成後（觀察用） | ❌ 僅 observe |
| `llm.call.pre` | LLM API call 發出前 | ✅ |
| `llm.call.post` | LLM API call 完成後（**僅供 commit**） | ❌ |
| `tool.call.pre` | external tool 執行前（副作用前最後關卡） | ✅ |
| `tool.call.post` | tool 完成後（觀察用） | ❌ |
| `run.post` | run 結束後（總結 evidence） | ❌ |

只有 `*.pre` triggers 評估 policy 並影響 decision。`*.post` 僅 commit / observe / emit evidence。

Runtime mapping：每 runtime adapter 將上述 abstract names 映射至自身機制（LangGraph super-step、Pydantic-AI tool selection 等）。

---

## 16. Ledger 一致性 & Sharding

### 16.1 Phase 1（first customer）

```
Single-region single-leader Postgres
- Row-level lock for reservation table
- WAL-based replication for read replicas
- Reservation TTL queue + background reaper
```

### 16.2 Phase 2+（規模化）

```
Sharded by (tenant_id, budget_id, window_start)
- Single leader per shard
- Cross-shard distributed transaction (rare; only for cross-tenant policies)
```

### 16.3 Hot-key handling（escrow formula）

```yaml
ledger:
  escrow:
    enabled_for_budget_kinds: [soft_org_global]    # ⚠ hard budget 禁 escrow
    
    formula:
      max_overalloc_micros: shard_count * escrow_size_per_shard
      max_overalloc_ratio: 0.02                     # 1-2% of org budget cap
    
    escrow_size_per_shard:
      strategy: dynamic
      base: org_budget_remaining / (shard_count * 10)
      max: org_budget_remaining * 0.005
    
    redemption:
      strategy: lazy_settlement
      schedule: per_minute
      on_close_window: full_settlement
    
    safety:
      hard_budget_cannot_use_escrow: required
      audit_overalloc_events: true
```

---

## 17. Bundle Signature

```yaml
bundleSignature:
  runtime: ed25519
  manifest: canonical JSON of all contracts + matrices
  
  verification:
    at_load: full_verification (cache result)
    at_eval: cheap_signature_compare_against_cached
  
  audit_event_stores: signature_digest_only (not full bundle)
  
  provenance:                                       # v3.1 補強
    sigstoreTransparencyLog: optional
    verifyAtLoadOnly: true
    # Sigstore/cosign 適合 provenance/transparency，不放 eval path
```

---

## 18. Quickstart Minimal Contract

新客戶第一個 contract — 不暴露完整 spec。

```yaml
apiVersion: spendguard.ai/v1alpha1
kind: Contract

metadata:
  name: quickstart-daily-budget
  tenant: example_tenant
  version: 1.0.0
  owner: platform-eng

spec:
  mode: shadow
  
  scope:
    routes: {include: ["*"]}
  
  budgets:
    - id: daily_usd
      window: {type: calendar_day, timezone: UTC}
      limit: {amountMicros: 100000000, currency: USD}
      reservation:
        amountSource: risk.p95
        requireHardCapWhenAvailable: true
        ttl: PT10M
      overrunPolicy:
        phase: post_commit
        type: record_violation_and_debt
  
  rules:
    - id: stop_when_exhausted
      priority: 1000
      condition: |
        budget("daily_usd").spent.amountMicros >= 
        budget("daily_usd").limit.amountMicros
      effect: {decision: stop}
      reasonCode: BUDGET_EXHAUSTED
    
    - id: degrade_when_low
      priority: 500
      condition: |
        budget("daily_usd").remaining.amountMicros < 
        money("20.00", "USD").amountMicros
      effect:
        decision: degrade
        mutate:
          modelAlias: {name: low_cost_research, channel: stable}
      reasonCode: BUDGET_LOW
  
  defaults: {decision: continue}
```

**Quickstart 設計原則**：
- 1 budget + 2 rules + shadow mode = 第一條 budget guard
- 不含 require_approval（第二個 starter template 才介紹）
- 90% 設定取 platform default（modeSemantics, decisionTransaction, evaluation, combining）

---

## 19. Reference Implementation POC

### 19.1 Chaos / Idempotency Tests（lock-required 7 個）

```yaml
chaos_test_scenarios:
  - name: snapshot_failure_then_retry
    inject_failure_at: snapshot
    expected: retry → fresh_snapshot → success
  
  - name: reserve_failure_partial_multi_budget
    inject_failure_at: reservationSet
    expected: release_all → fail_closed → no_phantom_reserve
  
  - name: audit_decision_failure_before_publish
    inject_failure_at: audit_decision
    expected: rollback_reservation → no_effect_published → retry
  
  - name: publish_effect_failure_after_audit
    inject_failure_at: publish_effect
    expected: idempotent_retry_via_effect_hash
  
  - name: provider_timeout_after_publish
    inject_failure_at: llm.call.post
    expected: commit_state=estimated → reconcile_at_invoice
  
  - name: hot_reload_during_in_flight
    inject: bundle_swap
    expected: in_flight_uses_pinned_old_bundle
  
  - name: child_agent_grant_revoked_mid_run
    inject: parent_revokes_grant
    expected: child_next_eval_fails_closed → run_aborted
```

### 19.2 GA-Gating Tests（Codex round-4 補入 3 個）

```yaml
ga_gating_test_scenarios:
  - name: multi_tenant_noisy_neighbor
    description: 高流量 tenant A 不應影響 tenant B 的 evaluator latency
  
  - name: clock_skew_window_boundary
    description: 跨 calendar window 邊界（00:00 UTC）+ NTP drift 下的 budget 計算正確性
  
  - name: jwt_exp_nbf_skew
    description: Sub-agent grant 的 exp / nbf 在 clock skew 下的處理（容忍 vs reject）
```

### 19.3 End-to-End Scenarios（5 個）

1. Quickstart shadow → enforce migration（含 1000 traces 觀察 + canary）
2. Multi-budget atomic reserve（3 budgets, 1 fails）
3. Provider timeout → estimated commit → invoice reconcile
4. Reservation overrun: pre_call_top_up vs post_commit debt
5. Sub-agent grant revocation mid-run

### 19.4 Stage Output Persistence Schema

```yaml
StageOutputs:
  partition_key: decision_id
  schema:
    snapshot: {hash, blob_ref, timestamp}
    matched_rules: {hash, rule_ids}
    effect: {hash, decision, mutation_patch}
    reservation: {reservation_id, reserved_at}
    audit_decision: {audit_event_id, committed_at}
    publish: {publish_id, side_effect_target}
    commit_or_release: {state, timestamp}
    audit_outcome: {audit_event_id}
  
  retention:
    hot: 30_days
    cold: 7_years                                   # SOX / audit retention
```

---

## 20. Companion Compatibility Policy（alpha）

v1alpha1 對 design partner 承諾：

| 承諾 | 細節 |
|---|---|
| **Bundle pinning** | 客戶可 pin specific bundle version；platform 升級不強制 migrate |
| **Audit schema immutable** | §13 schema 不會 break；只能 additive 擴充 |
| **Breaking changes require migration tool** | 從 v1alpha1 → v1beta1 提供 contract transformer |
| **Old/new evaluator dual-run** | Migration 期間舊 evaluator 與新 evaluator 並行；客戶可 diff |
| **Contract diff report** | Bundle 升級前自動產 diff report；客戶 review 後才升 |
| **Alpha SLA** | 99.5% availability（GA 為 99.9%）；明確標示 |

---

## 21. v1alpha1 → v1beta1 → v1（GA） 演進路徑

```
v1alpha1 (this spec, locked)
  ↓ (POC + first customer + 5 e2e scenarios + 7 chaos tests pass)
v1beta1
  ↓ (3 GA-gating tests pass + observed cold/warm latency replaces hypothesis + multi-customer validation)
v1 (GA)
```

正常情況下 v1alpha1 → v1beta1 無 break change（compatibility policy 保證）。Spec 的演進在 additive 範圍內。

---

## 22. Adoption History（簡記）

| Round | Codex 反饋 | 採納率 | 主要產出 |
|---|---|---|---|
| Round 1 | 8 個 partial / 反駁點 | 100% | irreversibility 重排；money/time schema；effect lattice；CRD 風格 YAML |
| Round 2 | 5 個 high-irreversibility gap（reservation overrun / decision transaction / approval resume / sub-agent trust / matrix versioning） | 100% | v2 補齊 5 gap；audit storm 防護；timeout 拆解 |
| Round 3 | 3 個新 high-irreversibility gap（mode semantics / multi-budget atomicity / provider commit uncertainty） | 100% | v3 補齊 3 新 gap；audit_decision 在 publish 前；JWT/SPIFFE；matrix compatibility channels |
| Round 4 | Minimal verification | 100% | v3.1 patch（mTLS + compatibilityClass + cold_start hypothesis 標記）→ **LOCK** |

---

## 23. Lock 後的下一步

1. **Reference impl POC 開工**（§19）— 與本 spec 平行展開
2. **Trace canonical schema RFC**（Stage 1B）— 開始下一個 RFC
3. **Sidecar architecture RFC**（Stage 1C）
4. **Ledger storage model RFC**（Stage 1D）
5. **First customer design partner onboarding**（v1alpha1 + alpha compatibility policy）

---

*Document version: contract-dsl-spec-v1alpha1 (LOCKED) | Generated: 2026-05-06 | Adoption: 100% across 4 Codex rounds | GA prerequisites listed §0.2 | Companion: agent-runtime-spend-guardrails-complete.md (v1.3)*
