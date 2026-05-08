# Trace Canonical Schema Specification — v1alpha1 (LOCKED)

> 🔒 **Status: LOCKED implementation spec**  
> **Lock date**: 2026-05-06  
> **Lock judgment basis**: Codex round-3 minimal verification — 「可以 lock，但建議做 v2 → spec 時加入 minor clarification patch；不需要 v3 / round-4。沒有發現新的 §7 級 high-irreversibility gap。」  
> **Adoption history**: Round 1/2/3 採納率 100%/100%/100%（3 輪零實質反駁）  
> **Companion**: `agent-runtime-spend-guardrails-complete.md` (v1.3 strategy) + `contract-dsl-spec-v1alpha1.md` (LOCKED)  
> **Compatibility policy**: alpha — schema bundle pinning + golden corpus + canonical event immutability + breaking changes require migration tool + dual decoder period

---

## 0. Lock status & POC prerequisites

### 0.1 範圍

完整的 Trace Canonical Schema 設計，可進入 reference implementation POC + first customer design partner。

### 0.2 POC 前置條件（Codex round-3 規定）

進入 reference implementation POC 前下列必須到位：

1. **Independent canonical ingest endpoint** — 不走 OTel sampling path（§10.1 強制）
2. **三個 storage class 與 RTBF tombstone flow** 實作（§10.2 強制）
3. **Schema bundle validation + mapping profile conformance tests**（§12 強制）
4. **Golden corpus 建立**：partial span、late arrival、cross-region、RTBF tombstone、CloudEvents audit 範例（§10.6）
5. **Pydantic-AI / LangGraph adapter 通過**：idempotency fallback 測試 + span links 測試

### 0.3 GA 前置條件

POC 通過後，GA 路徑前下列必達成：

1. Canonical ingest endpoint **SLO 定義並驗證**（POC 階段為 best-effort + backpressure；GA 必須有定義的可用性目標）
2. Producer **event signature** 強制（POC 階段 optional，GA required）
3. Conformance test suite 覆蓋所有 mapping profiles
4. 7-year backward compatibility decoder 通過 golden corpus
5. Cross-region 部署驗證 ingest_position determinism

### 0.4 何時可能需要 v2 spec

只有以下情況才開啟 v2 spec 修正：
- POC 揭示 schema 重大缺陷
- 發現新的 §7 級 high-irreversibility gap
- Contract DSL spec 升級 v1beta1 時 trace schema 對應 break

正常情況下 v1alpha1 → v1beta1 → v1（GA）為 additive 演進，**無 breaking changes**（compatibility policy 保證）。

---

## 1. Context（self-contained）

### 1.1 產品

**Agent Runtime Spend Guardrails** — 在 agent step / tool call / reasoning spend 邊界做 budget decision、policy enforcement、approval、rollback、audit 的 runtime 安全層。

### 1.2 Trace 在 T→L→C→D→E→P 中的角色

```
T (Trace) → L (Ledger) → C (Contract DSL) → D (Decision) → E (Evidence) → P (Proof)
   ↑ 本 SPEC
```

### 1.3 v1alpha1 核心哲學

> **spendguard.canonical 是真相**；OTel / OpenInference 是 mapping profile。  
> **Canonical 流不靠 observability 工具運送**；audit / decision / ledger 走獨立 durable ingest。  
> **Storage class 分層調和 GDPR 與 7 年 retention 矛盾**；不同 class 不同 deletion policy。  
> **Schema 與 Producer 都是被驗證的物件**：schema bundle + producer trust boundary 鎖定真實性。

---

## 2. Inverted Foundation Model

```
truth source: spendguard.canonical.v1alpha1
  ↓ (mapping profile)
  ├─ OTel Gen-AI semconv (Development status — 不可作 canonical)
  ├─ OpenInference (competing 標準)
  ├─ MLflow tracing (subset coverage)
  └─ runtime-native (per-runtime profile)
```

OTel 仍是 transport（W3C Trace Context propagation、SpanLink、SpanEvent、Baggage、Collector ingestion）；schema canonical truth 在 spendguard.canonical 命名空間。

### 2.1 Mapping profile 規範

```yaml
mapping_profiles:
  schema:
    profile_id: string
    profile_version: semver
    canonical_schema_version: spendguard.v1alpha1
    field_mappings:
      - {canonical, profile, lossy, loss_reason?}
    conformance_tests:
      required: true
      test_corpus: spendguard.conformance.v1alpha1
    lossiness_report:
      required: true
      published: true
```

### 2.2 Customer lossy mode

```yaml
customer_lossy_mode:
  options:
    lossy_allowed: default
    lossy_forbidden: 不合格 mapping 進 quarantine
  quarantine_behavior:
    on_lossy_event_when_lossy_forbidden: quarantine_with_audit_unverified_flag
```

---

## 3. Identity Split

### 3.1 OTel IDs（嚴格 W3C 格式）

```yaml
identity:
  otel:
    trace_id:
      format: w3c_trace_id_16_byte_lower_hex
      example: "0af7651916cd43dd8448eb211c80319c"
      required: true
    span_id:
      format: w3c_span_id_8_byte_lower_hex
      example: "00f067aa0ba902b7"
      required: true
    parent_span_id:
      format: w3c_span_id_8_byte_lower_hex
      required_when: not_root_span
```

### 3.2 spendguard.* IDs（UUID v7）

```yaml
identity:
  spendguard:
    run.id: uuid_v7                           # RFC 9562
    step.id: uuid_v7                           # required when span_kind == agent.step
    llm_call.id: uuid_v7                       # required when span_kind == llm.call
    tool_call.id: uuid_v7                      # required when span_kind == tool.call
    decision.id: uuid_v7                       # source: contract_decision_transaction
```

### 3.3 event_id

Producer 端生成穩定 UUID v7；不由 mutable attributes hash 派生；immutable。

### 3.4 idempotency_key（含 fallback）

```yaml
event_identity:
  idempotency_key:
    format: hmac_sha256
    primary_inputs:
      - tenant_id
      - producer_id
      - runtime_native_event_id
      - event_kind
      - occurrence_index
    fallback_when_runtime_native_event_id_missing:
      inputs:
        - tenant_id
        - producer_id
        - process_start_id                     # producer process 啟動時生成 UUID v7
        - otel.trace_id
        - otel.parent_span_id
        - event_kind
        - producer_sequence                    # monotonic per producer
      adapter_must_persist_mapping: true
      mapping_retention: minimum_30_days
    stable_across_retries: true
```

---

## 4. Tiered Required Fields（6 層）

### Tier 1: required_at_ingest

```yaml
- spendguard.canonical.tenant.id
- spendguard.canonical.run.id
- spendguard.canonical.runtime.kind
- spendguard.canonical.runtime.version
- spendguard.canonical.schema_version
- spendguard.canonical.event_time
- otel.trace_id
- otel.span_id
- event_id
```

### Tier 2: required_for_enforcement

```yaml
- spendguard.canonical.route
- spendguard.canonical.step.id
- spendguard.canonical.step.index_or_sequence
- spendguard.canonical.bundle.signature
- spendguard.canonical.bundle.manifest_ref
```

### Tier 3: required_at_llm_call_end

```yaml
- spendguard.canonical.llm_call.input_tokens
- spendguard.canonical.llm_call.output_tokens
- spendguard.canonical.llm_call.pricing.version
- spendguard.canonical.llm_call.commit_state
```

### Tier 4: required_for_ledger_attribution

```yaml
- spendguard.canonical.tenant.id
- spendguard.canonical.run.id
- spendguard.canonical.step.id
- spendguard.canonical.llm_call.id
- spendguard.canonical.pricing.version
- spendguard.canonical.ledger.reservation_id
```

### Tier 5: required_for_audit_replay

```yaml
- spendguard.canonical.snapshot.id
- spendguard.canonical.snapshot.hash
- spendguard.canonical.bundle.signature
- spendguard.canonical.bundle.manifest_ref
- spendguard.canonical.decision.id
- spendguard.canonical.decision.transaction_idempotency_key
```

### Tier 6: optional_profile_fields

```yaml
- runtime_native_payload
- mapping_profile_metadata
- reasoning_breakdown
- cache_hit_breakdown
```

---

## 5. Time Semantics

```yaml
time:
  event_time: rfc3339_nano                      # producer clock
  server_ingest_time: rfc3339_nano              # independent of event_time
  observed_precision: enum [ns, us, ms, s, unknown]   # ⚠ 不假設 ns 真實精度
  producer_sequence: int64                      # monotonic per producer
  monotonic_offset_ns: optional
  clock_skew_ms: optional
  
  ordering:
    primary: event_time
    tiebreaker_1: server_ingest_time
    tiebreaker_2: producer_sequence
    tiebreaker_3: uuid_v7_lexicographic
```

---

## 6. HMAC Tenant Salt（versioned, dual-read rotation）

```yaml
privacy:
  hash:
    algorithm: hmac_sha256
    hash_key_id: required
    hash_version: required
    redaction_version: required
    
    rotation:
      strategy: dual_read_no_rewrite
      cadence: yearly
      dual_period_months: 12
      old_hashes_NOT_recomputed: required
      
      # v1.1: keys validity during dual_read window
      both_old_and_new_keys_valid_during_dual_period: required
      sidecar_handshake_must_announce_active_epochs: required (per Sidecar §5)
      adapter_sdk_must_accept_either_epoch_during_dual_period: required
      cross_spec_alignment: aligned with Sidecar §15 producer signing key rotation
    
    storage:
      keys: hsm_or_kms_only
      cross_tenant_isolation: salt_per_tenant
  
  prompt_storage:
    default: hash_only
    redaction_layer:
      pii_detector: required
      replacement: "<REDACTED:KIND>"
  
  right_to_be_forgotten:
    deletion_scope: tenant_subset_or_user_id_hash
    cleartext_deletion: required
    hash_only_records: tombstone_required
    immutable_audit_log: forbidden_to_delete
    no_resurrection_via_dual_read: required
    subject_lookup_edge_deletion: required      # see §10.2 lookup_edges definition
```

---

## 7. Hierarchy: Span Tree + Links + Canonical Events Hybrid

### 7.1 三層模型

```yaml
hierarchy:
  span_tree: primary structure (parent_span_id)
  span_links: non-parent relationships (DAG / parallel / async)
  canonical_events: first-class events (CloudEvents 1.0 envelope)
```

### 7.2 Span hierarchy

```
agent.run                              (root span; W3C trace_id stable across run)
  ├── agent.step                       (UUID v7 step.id)
  │   ├── llm.call                     (UUID v7 llm_call.id)
  │   │   └── llm.reasoning            (optional; for thinking 模型)
  │   └── tool.call                    (UUID v7 tool_call.id)
  ├── agent.step                       (next; can be parallel via span_links)
  └── sub_agent.invocation             (link via budget_grant_jti)

canonical_events (parallel, CloudEvents envelope):
  ├── decision_event
  ├── audit_event (decision / outcome)
  ├── ledger.reservation_event
  ├── ledger.commit_event
  ├── approval lifecycle events
  ├── rollback_event
  └── tombstone_event (RTBF)
```

### 7.3 link_kind enum

```yaml
span_links:
  link_kind: enum [
    causal_dependency,
    branch_start,
    branch_join,
    retry_of,
    dag_dependency,
    async_completion,
  ]
```

### 7.4 Span lifecycle: append-only records

```yaml
span_lifecycle:
  model: append_only_records
  records: [span_start, span_delta, span_end]
  in_progress_updates: append_delta_only
  finalization_required_for: [token_usage, commit_amount, outcome, end_time]
  late_arrival_window: 1_hour
  after_window: backfill_with_late_arrival_marker
```

### 7.5 Canonical events 採 CloudEvents 1.0 envelope

```yaml
canonical_events:
  envelope: cloudevents_1_0_structured_mode
  required_attributes:
    specversion: "1.0"
    type: enum [
      spendguard.audit.decision,
      spendguard.audit.outcome,
      spendguard.decision,
      spendguard.ledger.reservation,
      spendguard.ledger.commit,
      spendguard.ledger.release,
      spendguard.approval.requested,
      spendguard.approval.granted,
      spendguard.approval.denied,
      spendguard.approval.expired,
      spendguard.rollback,
      spendguard.tombstone,
    ]
    source: producer_uri
    id: event_id (uuid_v7)
    time: rfc3339_nano
    datacontenttype: "application/json"
  
  spendguard_extension_attributes:
    tenantid, runid, decisionid (when applicable)
  
  span_relationship:
    primary_storage: independent_event_stream
    span_mirror: optional (OTel SpanEvent inline mirror)
    rule: |
      Truth 在 canonical event stream，不在 span attributes。
      Span 可 mirror audit summary，但不可作為 replay source。
```

### 7.6 Audit-Span 關係

```yaml
audit_span_relationship:
  audit_event_to_span: 1_to_1
  decision_event_to_span: 1_to_1
  ledger_reservation_event_to_span: 1_to_1
  ledger_commit_event_to_span: 1_to_1
  approval_lifecycle_to_span: 1_to_N
  rollback_event_to_span: 1_to_N
```

---

## 8. Namespace（dot-separated layers）

```
spendguard.
├── canonical.                          (truth schema)
│   ├── tenant.{id, region}
│   ├── run.{id, kind, route, ...}
│   ├── step.{id, index_or_sequence, kind, ...}
│   ├── llm_call.{id, provider, model, tokens, ...}
│   ├── tool_call.{id, name, side_effect, ...}
│   ├── reasoning.{class, budget, tokens, ...}
│   ├── sub_agent.{parent_run_id, child_run_id, grant_jti, ...}
│   ├── runtime.{kind, version}
│   ├── bundle.{signature, manifest_ref}
│   ├── pricing.{version}
│   ├── decision.{id, transaction_idempotency_key, ...}
│   ├── snapshot.{id, hash}
│   ├── ledger.{reservation_id, commit_id}
│   └── time.{event_time, observed_precision, producer_sequence, ...}
├── profile.{otel, openinference, mlflow, runtime_native}.*
└── meta.{schema_version, ingest_metadata, ...}
```

Style anti-patterns（禁用）：underscore（`spendguard.tenant_id`）、camelCase、uppercase。

---

## 9. Cross-Runtime Mapping

```yaml
cross_runtime_mapping:
  strategy: dual_layer
  
  canonical_layer:
    schema: spendguard.canonical.v1alpha1
    enforcement: required_at_ingestion
  
  per_runtime_profile:
    schema: spendguard.profile.<runtime_kind>.v1alpha1
    versioned: required
    lossiness_annotated: required
    conformance_tested: required
  
  adapters:
    pydantic_ai: L3 (per Contract §12)
    langgraph: L3 (special: dag_via_span_links)
    openai_agents: L1-L2
    crewai: L1-L2
    anthropic_claude_sdk: L0-L1
    autogen_msaf: L0
    smolagents: L0
    dspy: L0 (upstream data source)
```

---

## 10. High-Irreversibility 7 項

### 10.1 Sampling Pipeline Split

```yaml
sampling:
  pipelines:
    canonical_required:
      bypass_otel_sampling: true                # ⚠ 必須繞過 OTel sampling
      durability: append_only_before_ack
      independent_durable_ingest: required
      includes:
        - audit_event (any kind)
        - decision_event
        - ledger_reservation_event
        - ledger_commit_event
        - ledger_release_event
        - approval_event (any lifecycle)
        - cost_bearing_llm_call (Tier 3 fields complete)
        - rollback_event
        - tombstone_event
    
    observability_profile:
      may_use_otel_sampling: true
      sampleable:
        - runtime_native_payload
        - verbose_reasoning_intermediate
        - full_text_debug
        - tool_call_input_output_full_text
  
  enforcement:
    canonical_required_path:
      not_via: otel_collector_sampling_pipeline
      via: independent_canonical_ingest_endpoint

# === v2.1 patch ===
canonical_ingest:
  availability_target:
    poc: best_effort_with_backpressure
    ga: define_slo_before_ga
  
  failure_mode:
    enforcement_route:
      action: fail_closed_or_quarantine_no_enforce
      rationale: "ingest 不可達時 enforcement 不應通過"
    observability_route:
      action: buffer_then_retry
      rationale: "可暫存重試，不影響 enforcement"
```

### 10.2 Storage Classes 三層

```yaml
storage:
  classes:
    immutable_audit_log:
      retention: 7_years (SOX)
      deletion: forbidden
      contains_cleartext: forbidden
      append_only: required
      legal_hold_supported: required
      examples:
        - audit.decision events
        - audit.outcome events
        - tombstone events
    
    canonical_raw_log:
      retention: 7_years
      append_only: required
      contains: hashes_only
      cleartext_forbidden: required
      rtbf_delete: subject_lookup_edges_only      # 主資料留 hash；只刪 user→hash 對應
      examples:
        - hash-only span attributes
        - canonical event payloads (hash-only)
    
    profile_payload_blob:
      retention: tenant_policy
      cleartext_allowed_with_consent: true
      rtbf_delete: required
      examples:
        - runtime_native_payload
        - verbose reasoning intermediate text
        - tool call input/output full text
        - prompt cleartext (consent only)
        - completion cleartext (consent only)
  
  legal_hold:
    supported: required
    overrides_rtbf: required                       # legal hold 凍結期間 RTBF 排隊
    audit_event_emitted: required
    rtbf_resume_after_release: required
  
  tombstone:
    required_on_rtbf: true
    schema:
      type: spendguard.tombstone
      subject_hash: hmac_sha256
      deleted_at: rfc3339_nano
      retention_class_affected: array
      audit_event_id: uuid_v7

# === v2.1 patch: lookup_edges definition ===
storage:
  lookup_edges:
    definition: "PII-bearing or subject-identifying mapping from external subject identifiers to canonical hashes/events"
    examples:
      - user_id_to_hash
      - email_to_hash
      - session_id_to_user
      - customer_id_to_subject
    rtbf_action: delete_edge_emit_tombstone
    canonical_hash_record: retained_if_hash_only
    isolation: per_tenant_namespace
```

### 10.3 Cardinality Control

```yaml
cardinality:
  classes:
    low: < 1k
    medium: 1k-100k
    high: 100k-10M
    unbounded: > 10M or unbounded
  
  examples:
    low: [tenant.id (per scope), runtime.kind, commit_state, outcome]
    medium: [route, tool_call.name, llm_call.model]
    high: [prompt_template_hash, run.id]
    unbounded:
      - event_id
      - tool_call.input_hash
      - completion_hash
      - customer_id (when > 10M)
  
  policy:
    indexable: low + medium
    queryable_with_partition: high
    audit_only_no_index_or_partition_scan_only: unbounded
  
  customer_id_handling:
    threshold: 10M
    above_threshold:
      treat_as: unbounded
      operations_supported: [exact_lookup, partition_scan, materialized_aggregate]
      operations_forbidden: [secondary_index, cardinality_count_query, distinct_count_aggregate]
    below_threshold:
      treat_as: high (indexable with partition)
  
  customer_dashboard_guidance:
    distinct_customers_count: use_materialized_aggregate_or_approximate_sketch
    ad_hoc_distinct_query: forbidden_above_threshold
```

### 10.4 Cost Computation Timing（三 amount 對齊 Contract §5）

```yaml
cost_computation:
  ingest_time_pinned:
    # === Three-layer pricing freeze (per Ledger §13) ===
    - spendguard.canonical.llm_call.pricing.version
    - spendguard.canonical.llm_call.pricing.price_snapshot_hash
    - spendguard.canonical.llm_call.pricing.fx_rate_version
    - spendguard.canonical.llm_call.pricing.unit_conversion_version
    # === Commit state ===
    - spendguard.canonical.llm_call.commit_state
  
  amount_fields:
    estimated_amount:
      type: money
      optional: true
      populated_when: commit_state in [unknown, estimated]
      source: risk.p90 (conservative)
    
    provider_reported_amount:
      type: money
      optional: true
      populated_when: commit_state == provider_reported
      source: provider_response_usage_header
    
    invoice_reconciled_amount:
      type: money
      optional: true
      populated_when: commit_state == invoice_reconciled
      source: monthly_provider_invoice
    
    finalized_amount:
      type: derived
      derivation:
        priority_order:
          - invoice_reconciled_amount
          - provider_reported_amount
          - estimated_amount
      not_stored: true (computed at query time)
      query_layer_must_apply_priority: required
  
  recomputability:
    formula: pricing_version + token_counts → amount_micros
    deterministic: required
    audit_replay: must_yield_same_amount_with_same_pricing_version
  
  alignment_with_contract: contract-dsl-spec-v1alpha1.md §5 commitStateMachine
```

### 10.5 Cross-Region Ordering

```yaml
cross_region:
  ingest_position:
    fields:
      region_id: string
      ingest_shard_id: string
      ingest_log_offset: int64
    immutable: true
    ordering_scope: per_ingest_shard
    persistence: same_storage_class_as_event
    not_exposed_as: business_id
    
    # === v2.1 patch ===
    semantics: deterministic_replay_order_not_global_happens_before
    rationale: |
      ingest_position 提供 deterministic replay order，
      不是 cross-region global happens-before relation。
      Causal ordering 仍需 event_time + producer_sequence。
  
  global_ordering_strategy:
    within_region:
      primary: event_time
      tiebreaker_1: producer_sequence
    cross_region:
      primary: ingest_position (region_id, ingest_shard_id, ingest_log_offset)
      not_relying_on: time_alone
  
  data_residency:
    cross_region_join_in_query_layer: tenant_overridable
    default: forbidden
```

### 10.6 Reverse Migration

```yaml
reverse_migration:
  retention: 7_years
  
  decoder_support_window:
    rule: "decoder for schema vN must be supported until last record of schema vN passes retention end"
    not_deprecated_after_n_years: required
    minimum_dual_decode_period: 12_months_per_version_transition
  
  golden_corpus:
    purpose: 確保舊 raw event 在新 decoder 下可正確 decode
    contents:
      - sample_records_per_schema_version
      - edge_cases (clock skew, late arrival, partial records)
      - boundary_cases (cross-region, RTBF tombstone, CloudEvents audit)
      - producer_trust_edge_cases (key rotation, signature)
    update_frequency: per_schema_version_change
    test_required_before_decoder_release: true
  
  forward_compatibility:
    unknown_fields: preserved
    unknown_span_kinds: stored_as_generic_span
    unknown_canonical_event_types: stored_with_type_unknown_flag
  
  backward_compatibility:
    old_event_in_new_decoder: required
    new_decoder_must_pass_golden_corpus: required
  
  migration_tool: spendguard.migrate
```

### 10.7 Tenant Validation

```yaml
tenant_validation:
  modes:
    strict:
      missing_required_at_ingest: reject_with_error
      schema_violation: reject_with_error
      audit_gap: alert
    quarantine:
      missing_required_at_ingest: quarantine_with_audit_unverified_flag
      schema_violation: quarantine
      audit_gap: emit_gap_event
    accept_partial: forbidden
  
  per_route_override:
    allowed: true
    enforcement_routes:
      minimum_mode: strict
      rationale: enforcement 路由若 quarantine = audit gap = 預算守不住
    observability_routes:
      minimum_mode: quarantine
      strict_optional: true
    audit_overrides_required: true
```

---

## 11. Contract §13 Audit Integration

### 11.1 Trace Anchors for Decision Transaction

Contract §6 8 階段 anchor：

```yaml
decision_transaction_trace_anchors:
  snapshot:        spendguard.canonical.snapshot.id
  evaluate:        spendguard.canonical.evaluation.id
  prepare_effect:  spendguard.canonical.effect.prepared_id
  reserve:         spendguard.canonical.ledger.reservation_id
  audit_decision:  spendguard.canonical.audit.decision_event_id
  publish_effect:  spendguard.canonical.effect.published_id
  commit_or_release: spendguard.canonical.ledger.commit_id_or_release_id
  audit_outcome:   spendguard.canonical.audit.outcome_event_id

decision_transaction:
  idempotency_key: hmac_sha256(stable_inputs)
  trace_anchor_chain: linked_via_decision.id
```

### 11.2 Bundle Manifest Joinable

```yaml
bundle:
  signature: ed25519_signature
  manifest_ref:
    storage_uri: blob_store_ref
    manifest_hash: sha256
    contract_versions: [{name, version}]
    matrices_versions: [{name, version}]
    indexable: required
```

### 11.3 Snapshot 與 Trace Metadata

```yaml
snapshot:
  snapshot_hash: sha256
  blob_ref: object_store_ref
  
  trace_metadata:
    ledger_state: {budgets, as_of_event_time}
    risk_band: {per_route_model, as_of_event_time}
    run_metadata: {run_id, step_id, step_index, route, runtime}
    pricing_metadata: {pricing_version, model_capability_matrix_version}
```

### 11.4 Sub-Agent Grant Audit

```yaml
sub_agent_grant_audit:
  required_fields:
    parent_tenant_id: required
    child_tenant_id: required
    grant_jti: required (JWT jti)
    delegation_scope: {routes, runtimes, max_amount}
    audit_owner: enum [parent_tenant, child_tenant, both]
```

### 11.5 Audit as Canonical Event

Storage independent canonical event stream；CloudEvents envelope；不在 span attributes inline。Span 透過 SpanLink 連接到 audit canonical event；query pattern 是 join via event_id 或 decision_id。

### 11.6 Cross-Tenant Audit Dual Projection

```yaml
cross_tenant_audit:
  storage_pattern: dual_projection
  
  parent_tenant_view:
    sees:
      - grant_issuance
      - grant_revocation
      - child_completion_summary (aggregated)
    not_sees:
      - child_decisions_individually
      - child_prompt_hashes
  
  child_tenant_view:
    sees:
      - grant_consumption
      - own_decisions
      - own_commits
    not_sees:
      - parent_other_grants
      - parent_internal_decisions
  
  cross_visibility:
    forbidden_by_default: true
    allowed_when: explicit_consent_audit_event_emitted
```

---

## 12. Schema Distribution（v2.1 NEW）

```yaml
schema_distribution:
  schema_bundle_id: uuid_v7
  schema_bundle_hash: sha256
  canonical_schema_version: spendguard.v1alpha1
  mapping_profile_versions: map<profile_id, version>
  
  producer_must_emit_schema_bundle_id: true
  ingest_must_validate_against_bundle: true
  
  bundle_resolution:
    discovery_endpoint: spendguard.schema.bundle_registry
    cache_ttl: 1_hour
    cache_invalidation: on_bundle_hash_change
  
  unknown_bundle_id_at_ingest:
    action: reject_with_unknown_bundle_error
    fallback: register_bundle_with_audit_event
```

每個 producer emit 的事件必須帶 `schema_bundle_id` + `schema_bundle_hash`，ingest 端驗證並拒絕未知 bundle。

---

## 13. Producer Trust Boundary（v2.1 NEW）

```yaml
producer_trust:
  producer_id: required
  
  auth:
    workload_identity: mtls_or_spiffe          # 對齊 Contract §8
    key_rotation: required
    rotation_cadence: yearly
  
  event_signature:
    optional_for_poc: true
    required_for_ga: true
    algorithm: ed25519
    signs:
      - canonical_event_payload
      - schema_bundle_id
      - producer_id
      - producer_sequence
  
  clock_claims_untrusted_until_ingest: true
  rationale: |
    Producer 提供的 event_time 等時間欄位不可在 ingest 前信任；
    必須 server_ingest_time 獨立記錄（§5）；
    時鐘漂移以 clock_skew_ms 顯式標示。
  
  producer_revocation:
    parent_can_revoke: true
    revocation_distribution: distributed_cache
    in_flight_events_after_revocation: quarantine
```

---

## 14. Reference Implementation POC Plan

### 14.1 必達成的實作項

1. **Independent canonical ingest endpoint**（§10.1）
2. **Three storage classes**（§10.2）
3. **RTBF tombstone flow**（§6 + §10.2）
4. **Schema bundle validation**（§12）
5. **Mapping profile conformance tests**（§9）
6. **Golden corpus**（§10.6）
7. **Pydantic-AI adapter L3** with idempotency fallback + span links 測試
8. **LangGraph adapter L3** with DAG via span_links

### 14.2 End-to-End Scenarios（5 個）

1. Quickstart Pydantic-AI integration → canonical ingest → audit event 全鏈路
2. LangGraph DAG with parallel branches → span_links 正確
3. Sub-agent invocation → cross-tenant audit dual projection
4. RTBF tombstone → cleartext deletion + hash-only canonical retention
5. Cross-region ingest position determinism

### 14.3 Chaos Tests

```yaml
chaos_tests:
  - producer_restart_with_idempotency_fallback
  - canonical_ingest_endpoint_down_with_fail_closed
  - otel_collector_sampling_active_canonical_unaffected
  - schema_bundle_unknown_at_ingest
  - producer_signature_rotation_during_in_flight
  - cross_region_clock_skew_with_ingest_position_determinism
  - rtbf_during_legal_hold
```

---

## 15. v2.1 Patch Detail（記錄用）

進入 spec lock 前納入的 minor clarifications：

| Patch | 位置 | 內容 |
|---|---|---|
| schema_distribution | §12（NEW） | schema_bundle_id + schema_bundle_hash + producer must emit + ingest validates |
| producer_trust | §13（NEW） | producer_id + workload_identity + key_rotation + event_signature (POC opt / GA required) + clock_claims_untrusted |
| storage.lookup_edges | §10.2 | 明確定義 lookup edge：subject identifier → hash/event 映射；RTBF 刪 edge 留 hash |
| canonical_ingest 可用性 | §10.1 | POC best_effort_with_backpressure；GA SLO required；failure modes per route |
| cross_region.ingest_position 語意 | §10.5 | 補一行：deterministic_replay_order_not_global_happens_before |

---

## 16. Companion Compatibility Policy（alpha）

| 承諾 | 細節 |
|---|---|
| **Schema bundle pinning** | 客戶可 pin specific bundle id；platform 升級不強制 migrate |
| **Canonical event immutability** | §7.5 schema 不會 break；只能 additive 擴充 |
| **Audit storage immutable** | §10.2 immutable_audit_log 永不刪除（除 legal hold 解凍後 RTBF 排隊） |
| **Decoder support window** | 任一 schema version 的 decoder 至少保留至最後一筆該 schema trace retention 結束 |
| **Golden corpus 通過** | 每次 decoder release 必須通過 |
| **Dual decode period** | 版本轉換最少 12 個月雙解碼 |
| **Breaking changes require migration tool** | spendguard.migrate 必備 |
| **Alpha SLA** | 99.5% availability for canonical ingest（GA 為 99.95%） |

---

## 17. Adoption History

| Round | Codex 反饋 | 採納率 | 主要產出 |
|---|---|---|---|
| Round 1 | 8 個 partial / 反駁點（含致命 OTel ID 格式錯誤） | 100% | Foundation 倒置；W3C/UUID v7 split；6 層 required tiers；§10 升 7 項 high-irreversibility |
| Round 2 | 2 個新 high-irreversibility gap（sampling pipeline split / storage classes 三層） | 100% | Sampling 繞過 OTel；三層 storage；idempotency fallback；customer_id > 10M 規則；3 amount fields；CloudEvents envelope |
| Round 3 | Minimal verification | 100% | v2.1 patch（schema_distribution + producer_trust + lookup_edges 定義 + canonical_ingest 可用性 + ingest_position 語意）→ **LOCK** |

---

## 18. Lock 後的下一步

1. **Reference impl POC 開工**（§14）— 與本 spec 平行展開
2. **Sidecar architecture RFC**（Stage 1C）— 開始下一個 RFC
3. **Ledger storage model RFC**（Stage 1D）
4. **First customer design partner onboarding**（trace adapter + Contract DSL 整合）
5. **Schema bundle registry 啟動**（§12）

---

*Document version: trace-schema-spec-v1alpha1 (LOCKED) | Generated: 2026-05-06 | Adoption: 100% across 3 Codex rounds | POC prerequisites listed §0.2 | GA prerequisites listed §0.3 | Companion: agent-runtime-spend-guardrails-complete.md (v1.3) + contract-dsl-spec-v1alpha1.md (LOCKED)*
