# Sidecar Architecture Specification — v1alpha1 (LOCKED)

> 🔒 **Status: LOCKED implementation spec**  
> **Lock date**: 2026-05-07  
> **Lock judgment basis**: Codex round-3 minimal verification — 「v2 收斂到位。可以 lock 為 sidecar-architecture-spec-v1alpha1，不需要 v3，但建議在 lock 版加一個 v2.1 clarification patch。」  
> **Adoption history**: Round 1/2/3 採納率 100%/100%/100%（3 輪零實質反駁）  
> **Companions**:
> - `agent-runtime-spend-guardrails-complete.md` (v1.3 strategy)
> - `contract-dsl-spec-v1alpha1.md` (LOCKED)
> - `trace-schema-spec-v1alpha1.md` (LOCKED)
> 
> **Compatibility policy**: alpha — region affinity 不可變更 / durability mode migration 30 天雙寫 / endpoint catalog signed / fencing token monotonic epoch / Helm chart 隨 spec 同步

---

## 0. Lock Status & Prerequisites

### 0.1 範圍

完整的 Sidecar Architecture 設計，可進入 reference implementation POC + first customer design partner。

### 0.2 POC 前置條件（Codex round-3 規定）

進入 reference implementation POC 前下列必須到位：

1. **Helm chart** with sidecar injection、preStop drain、readiness gating、region affinity values
2. **Pydantic-AI 與 LangGraph adapters** 實作 UDS handshake + capability claim
3. **Remote decision journal ack mode** for K8s SaaS path
4. **Canonical ingest ack mode** for Lambda path
5. **Chaos tests**：pod eviction / rolling restart / spot interruption / journal unavailable / region endpoint failover / fencing split-brain / canonical ingest down
6. **Metrics**：decision stage p99 / drain timeout count / journal replay count / endpoint catalog staleness / fencing failures

### 0.3 GA 前置條件

POC 通過後，GA 路徑前下列必達成：

1. Multi-mode 部署驗證（K8s SaaS + Lambda + air-gapped）
2. 7 個 chaos test 全通過
3. Cold start p99 實測取代 hypothesis
4. 100-replica large-scale 驗證
5. Multi-region failover 驗證（active-active strong-consistency 與 active-passive 兩模式）

### 0.4 何時可能需要 v2 spec

只有以下情況才開啟 v2 spec 修正：
- POC 揭示架構重大缺陷
- 發現新的 §8 級 high-irreversibility gap
- Contract DSL spec / Trace schema spec 升級時 sidecar 對應 break

正常情況下 v1alpha1 → v1beta1 → v1（GA）為 additive 演進，**無 breaking changes**。

---

## 1. Context（self-contained）

### 1.1 產品

**Agent Runtime Spend Guardrails** — 在 agent step / tool call / reasoning spend 邊界做 budget decision、policy enforcement、approval、rollback、audit 的 runtime 安全層。

### 1.2 已 lock 的 specs（依賴）

- `contract-dsl-spec-v1alpha1.md`：decision transaction、sub-agent grant、latency budget
- `trace-schema-spec-v1alpha1.md`：canonical ingest、storage classes、producer trust、schema distribution

### 1.3 v1alpha1 核心哲學

> **Sidecar 是 per-workload-instance 的 control plane**，不是 per-service shared process。  
> **Enforcement strength 必須顯式宣告**；contract 不可隱含「強 enforcement」假設。  
> **Region affinity 是 hard enforcement 前提**；跨 region eventual consistency 不可用於預算守衛。  
> **Durability 不是客戶選項**；deployment mode 決定可用 durability path。  
> **Drain 是 audit 不變式延伸**；終止流程必須遵守「無 audit 則無 effect」。  
> **Endpoint discovery 與 fencing 必須 signed**；不可信賴 ambient discovery。

---

## 2. Topology: per_workload_instance

```yaml
sidecar_topology:
  default: per_workload_instance
  
  k8s:
    pattern: sidecar_container_in_pod
    deployment_replica_count: each_replica_has_own_sidecar
    
  ecs:
    pattern: sidecar_container_in_task
    
  vm_or_bare_metal:
    pattern: local_daemon_per_service_instance
    
  serverless:
    pattern: see §8.2 (Lambda Extensions 獨立模式)
  
  shared_per_service_process: forbidden_for_enforcement_phase1
  per_n_pods_shared: forbidden_for_enforcement_phase1
  
  rationale:
    - UDS / named pipe 必須在同一 workload instance
    - latency 不可加 inter-pod queue（違反 Contract §14 50ms p99）
    - failure isolation 必須 per-instance
    - mTLS/SPIFFE workload identity 是 per-workload

service_identity:
  representation: spiffe_claims_and_labels
  example_spiffe_id: "spiffe://tenant_abc.spendguard.ai/agent-service/prod/replica-3"
  same_service_implication:
    same_spiffe_trust_domain: yes
    same_workload_identity: each_replica_has_own_certificate
    shared_sidecar_process: no
```

---

## 3. Three-layer Architecture

### 3.1 Semantic Enforcement Plane（必備）

```yaml
semantic_enforcement_plane:
  required_for_l3_capability:
    - in_process_adapter
    - local_sidecar (per-workload-instance)
  
  in_process_adapter:
    purpose: runtime hook + effect actuator
    language_libraries:
      python: spendguard-py-adapter
      typescript: spendguard-ts-adapter
      go: spendguard-go-adapter
    runtime_integrations:
      pydantic_ai: L3 (UsageLimits hooks)
      langgraph: L3 (super-step / checkpoint hooks)
      openai_agents: L2 (callback wrapping)
      crewai: L1-L2
      anthropic_claude_sdk: L0-L1
    responsibilities:
      - emit canonical events to local sidecar via IPC
      - apply mutations (RFC6902 patches)
      - W3C Trace Context propagation
      - sub-agent budget grant propagation
    not_responsibilities:
      - signing
      - policy evaluation
      - audit emission
  
  local_sidecar:
    purpose: policy authority + effect signer
    topology: per_workload_instance
    responsibilities:
      - Contract DSL evaluation (CEL + lattice)
      - decision transaction state machine (Contract §6)
      - ledger client
      - canonical event signing + emission (Trace §13)
      - bundle / matrix / pricing cache
      - mTLS termination + workload identity
      - signed fail-safe manifest serving
      - endpoint catalog cache
      - fencing token negotiation
```

### 3.2 Transport Fallback（可選, Phase 2）

```yaml
transport_fallback:
  optional_http_proxy:
    capability_level: L1_L2_only
    phase: phase2
    use_case: customer cannot install in-process adapter (Java / Rust / Elixir)
    
    cannot_claim:
      - agent_step_enforcement
      - tool_boundary_enforcement
      - decision_at_step_boundary
    
    can_claim:
      - llm_call_capture (response token usage)
      - egress_blocking (per provider URL)
      - request_authorization_injection
    
    must_be_paired_with: minimum_l1_or_l2_adapter
    standalone_mode: forbidden_for_enforcement
```

### 3.3 Capability Claim 強制

```yaml
capability_claim:
  contract_must_specify_required_strength: required
  
  contract_field:
    enforcement_strength_required: enum [advisory_sdk, semantic_adapter, egress_proxy_hard_block, provider_key_gateway]
  
  if_deployment_provides_lower_strength_than_contract_requires:
    action: refuse_to_load_contract
    audit_event: capability_mismatch
```

---

## 4. Enforcement Strength 4 級

| Level | Description | Deployment | Bypass Resistance |
|---|---|---|---|
| **advisory_sdk** | App can choose to bypass; SDK only suggests | in-process adapter only | bypassable |
| **semantic_adapter** | Runtime hook enforced if integrated | adapter + local sidecar | bypassable only via raw provider SDK |
| **egress_proxy_hard_block** | Provider API blockable but no step semantics | adapter + sidecar + transport HTTP proxy | not bypassable for LLM calls; cannot enforce step decisions |
| **provider_key_gateway** | Strongest for LLM calls; provider keys controlled by sidecar | full stack with key escrow | app physically cannot call providers without sidecar |

不增加 `kernel_level_eBPF` 級別 — eBPF 是 transport/egress adjunct，不是 semantic enforcement。

---

## 5. IPC: UDS Peer Credentials + Windows Fallback

```yaml
ipc:
  in_process_adapter ↔ local_sidecar:
    primary: grpc_over_unix_domain_socket
    socket_path: /var/run/spendguard/adapter.sock
    socket_permissions: 0600
    
    auth:
      uds_peer_credentials:
        method: SO_PEERCRED (Linux) / LOCAL_PEERCRED (macOS) / equivalent
        verify:
          - peer_uid matches expected_app_uid
          - peer_pid in same_pod_or_process_group
        required: true
      
      protocol_handshake:
        version_negotiation: required
        adapter_announces:
          - sdk_version
          - runtime_kind
          - capability_level (L0/L1/L2/L3)
          - tenant_id_assertion
        sidecar_announces:
          - sidecar_version
          - bundle_signature
          - schema_bundle_id
          - capability_required_by_loaded_contracts
          - active_key_epochs:                          # v1.1 補強
              producer_signing_key_epochs: [...]        # for canonical event signing (per §15 + Trace §13)
              hmac_tenant_salt_epochs: [...]            # for prompt_hash / request_hash validation (per Trace §6)
              # 12-month dual_read window: old + new epochs both valid
              # adapter SDK 必須能驗 dual epoch；retry 不會因 rotation 失敗
        on_mismatch: refuse_session + emit_audit_event
      
      shared_token_only: forbidden
      ambient_authority_alone: forbidden
    
    windows_fallback:
      primary: named_pipe
      auth: loopback_mtls
      cert_distribution: spire_or_local_signed_token
      capability: equivalent_to_uds_peer_credentials
    
    http_loopback_fallback:
      auth: loopback_mtls or signed_local_token
      capability: degraded
      use_when: UDS / named pipe unavailable
      audit_event_on_fallback: required
```

---

## 6. Decision Durability — Selection Matrix

### 6.1 三選一 Options

```yaml
decision_durability_options:
  option_1_canonical_ingest_ack:
    durability_provider: canonical ingest endpoint
    guarantee: ack_after_durable_write_at_ingest_side
    log_system_analog: kafka_acks_all / quorum_ack
  
  option_2_remote_decision_journal_ack:
    durability_provider: dedicated decision journal service
    protocol: append-only log + fsync_before_ack
    log_system_analog: kafka_acks_1 / kinesis_put_record_ack
  
  option_3_persistent_local_wal_with_replay_guarantee:
    durability_provider: persistent local disk
    requirements:
      - persistent_volume (NOT emptyDir)
      - fsync_before_ack
      - replay_on_sidecar_restart
    log_system_analog: local_fsync
    risk: pod-disk failure = data loss
```

### 6.2 Deployment Mode Selection Matrix（強制對應）

```yaml
durability_mode_selection:
  rules:
    k8s_saas_managed:
      preferred: remote_decision_journal_ack
      allowed: [remote_decision_journal_ack, canonical_ingest_ack]
      forbidden: [persistent_local_wal_only]
    
    k8s_self_hosted:
      preferred: remote_decision_journal_ack
      allowed: [remote_decision_journal_ack, canonical_ingest_ack, persistent_local_wal_with_replay_guarantee]
      requires_if_local_wal: pvc_backed_storage_class + replication
    
    lambda:
      preferred: canonical_ingest_ack
      allowed: [canonical_ingest_ack]
      forbidden: [persistent_local_wal_with_replay_guarantee]
      rationale: execution context ephemeral
    
    cloud_run_or_container_serverless:
      preferred: canonical_ingest_ack
      allowed: [canonical_ingest_ack, remote_decision_journal_ack]
      forbidden: [persistent_local_wal_with_replay_guarantee]
    
    air_gapped:
      preferred: persistent_local_wal_with_replay_guarantee
      allowed: [persistent_local_wal_with_replay_guarantee]
      requires:
        local_canonical_ingest_or_local_decision_journal: required
    
    vm_or_bare_metal:
      preferred: remote_decision_journal_ack
      allowed: [remote_decision_journal_ack, canonical_ingest_ack, persistent_local_wal_with_replay_guarantee]
  
  hard_enforcement:
    requires_ack_before_publish: true (Contract §6 invariant)
    forbidden: in_memory_only / emptyDir_only
  
  install_time_validation:
    matrix_violation_attempts: install_time_reject
    runtime_override_attempts: emit_audit + sidecar_readiness_failure
  
  customer_visibility:
    durability_mode_displayed_in_dashboard: required
    explanation_template: |
      "您的部署模式（{deployment_mode}）使用 {durability_mode} durability。
       此模式下 audit_decision 在 publish_effect 前的 ack 來源為 {ack_source}。"

durability_mode_migration:                       # v2.1 patch
  dual_write_period:
    minimum: 30_days
    rationale: |
      Migration 是 operational confidence 累積期，不是 audit retention 對齊期。
      Audit retention 仍是 7 年，由 records 自身 retention 控制，不靠 dual-write duration。
  
  cutover:
    require_both_paths_healthy: true
    audit_event_on_cutover: required
```

---

## 7. Signed Fail-safe Manifest

```yaml
fail_safe:
  signed_manifest:
    cached_locations:
      sidecar: required
      adapter: optional (Phase 2)
    
    schema:
      manifest_version: semver
      tenant_id: string
      effective_from: rfc3339_nano
      effective_until: rfc3339_nano
      route_classification:
        - {route, kind: enforcement|observability, fail_default: fail_open|fail_closed}
      bundle_ref:
        bundle_id: uuid_v7
        bundle_hash: sha256
      signature: ed25519
    
    refresh_strategy:
      pull_at: sidecar_startup + every_5_minutes
      push: webhook_or_sse_on_change
      validity_token: included_in_payload
    
    stale_window:
      normal_max_stale: 24_hours
      critical_revocation_max_stale: 5_minutes_or_push_required
      
      normal_definition: route classification + fail defaults + bundle ref
      critical_definition: revoked tenants / revoked bundles / emergency policy switch
      
      on_normal_stale_exceeded: continue_with_warning_until_max_age
      on_critical_stale_exceeded: fail_closed_for_enforcement_routes
    
    push_channel:
      protocol: server_sent_events_or_webhook
      authenticated: required
      durability: best_effort_with_fallback_to_pull
  
  defaults_when_manifest_unavailable:
    sidecar_unreachable_from_adapter:
      enforcement_route: fail_closed
      observability_route: fail_open_buffer
      audit_event: emit_when_sidecar_recovers
```

---

## 8. Endpoint Discovery（v2.1 NEW）

```yaml
endpoint_discovery:
  source_priority:
    - signed_fail_safe_manifest
    - signed_control_plane_endpoint_catalog
    - platform_service_discovery
    - k8s_service_dns
  
  rationale: |
    K8s DNS 是本地 resolution，不是 source of truth。
    Source of truth 必須是 signed catalog from control plane.
  
  endpoint_catalog:
    signed: required
    signature_algorithm: ed25519
    
    contains:
      ledger_endpoints:
        - endpoint_url
        - region
        - consistency_capability: enum [strong_global, single_writer_per_budget, eventual]
        - health_status
      
      decision_journal_endpoints:
        - endpoint_url
        - region
        - durability_capability
        - health_status
      
      canonical_ingest_endpoints:
        - endpoint_url
        - region
        - ack_mode_capability: enum [local_fsync, remote_append_ack, quorum_ack]
        - health_status
      
      bundle_registry_endpoints:
        - endpoint_url
        - global_replicated: bool
    
    max_staleness:
      normal: 24_hours
      critical_revocation: 5_minutes
    
    refresh_strategy:
      pull_at: sidecar_startup + every_5_minutes
      push: server_sent_events_on_endpoint_change
  
  hard_enforcement_filter:
    require_endpoint_consistency_capability: strong_or_single_writer
    eventual_consistency_endpoints_excluded_from_hard_enforcement: true
  
  discovery_failure:
    no_endpoint_available_in_region: fail_closed
    cross_region_endpoint_with_strong_consistency: allowed_with_audit_event
    cross_region_endpoint_eventual: forbidden_for_enforcement
```

---

## 9. Fencing Token（v2.1 NEW）

```yaml
fencing:
  token:
    source: decision_journal_or_ledger_lease
    not_source: k8s_lease_alone                  # K8s lease 不是 source of financial truth
    
    monotonic_epoch: required
    epoch_increment_on: ownership_transfer
    
    scoped_to:
      - sidecar_instance_id
      - workload_instance_id
      - reservation_id
    
    compare_and_swap_on_recover: required
    
    retrieval:
      at_decision_time: included_in_decision_record
      from_ledger_or_journal: lease_with_ttl
      ttl: aligned_with_reservation_ttl
  
  pvc_recovery:
    previous_owner_must_be_fenced_before_replay: required
    fencing_mechanism: monotonic_epoch_compare_and_swap
    split_brain_action: fail_closed_emit_audit
    
    recovery_flow:
      step_1: new_sidecar_acquires_lease_with_higher_epoch
      step_2: previous_owner_writes_with_lower_epoch_get_rejected
      step_3: new_sidecar_replays_from_journal
      step_4: emit_recovery_audit_event
  
  k8s_lease_role:
    purpose: lifecycle_assist (pod restart detection)
    not_role: financial_ownership_authority
    rationale: |
      K8s lease handles pod lifecycle but doesn't know about reservation ownership.
      Financial fencing must use ledger/journal lease with epoch.
```

---

## 10. Region Affinity & Multi-Cluster Federation

```yaml
region_affinity:
  sidecar_region: required
  ledger_region: same_region_required_for_hard_enforcement
  decision_journal_region: same_region_required
  canonical_ingest_region: same_region_preferred
  bundle_registry_region: globally_replicated_acceptable
  
  cross_region_fallback:
    hard_enforcement: 
      action: fail_closed_unless_ledger_strong_consistency_available
      rationale: cross-region eventual consistency 不可用於 hard budget
    
    observability:
      action: buffer_then_retry
      acceptable: yes
  
  active_active_constraints:
    hard_budget_requires:
      - strongly_consistent_global_ledger
      - OR: single_writer_per_budget (per-budget leader assigned)
    eventual_consistency_allowed_for: observability_only
  
  multi_cluster_federation:
    sidecar_must_know:
      - own_cluster_id
      - own_region
      - available_endpoints (via signed catalog §8)
    
    failover_decision:
      same_region_secondary_endpoint: prefer
      cross_region_endpoint: only_if_strong_consistency
      no_endpoint_available: fail_closed
    
    cross_region_routing_audit:
      required: true
      audit_event: region_failover_invoked
  
  active_passive_failover:                       # round-3 §14 answer #2
    in_flight_decisions:
      keep_old_region_journal_if_reachable: preferred
      otherwise_recover_via_journal_fencing: required
    new_decisions:
      use_new_signed_endpoint_catalog: required
```

---

## 11. Lifecycle Drain & Autoscaler Semantics

```yaml
lifecycle_drain:
  triggers:
    - pod_eviction (k8s)
    - rolling_restart (deployment update)
    - scale_down (HPA / VPA)
    - spot_interruption (cloud spot instances)
    - lambda_shutdown (extensions API)
    - deployment_terminated (customer manual)
  
  behavior:
    on_drain_initiated:
      stop_accepting_new_decisions: required
      complete_in_flight_decisions:
        if_within_drain_window: required
        if_exceeds_window: see on_drain_timeout
      publish_after_audit_only: required           # 不變式：「無 audit 則無 effect」
      release_or_recover_reservations: required
      flush_pending_canonical_events: required
      flush_decision_journal: required
    
    drain_window:
      configurable_max: tenant_overridable
      default_k8s: 60_seconds (terminationGracePeriodSeconds)
      default_lambda: 2_seconds (extensions API limit)
      default_vm: 30_seconds
  
  on_drain_timeout:
    action: fail_closed_and_recover_from_journal
    in_flight_decisions:
      already_audited_not_published: replay_publish_on_next_owner
      not_audited: rollback_via_compensating_action
    reservations:
      ttl_release: handled_by_ledger_ttl
      explicit_release_on_drain: preferred_when_possible
  
  pvc_unmount_drain:
    trigger: pod_termination_with_pvc_unmount
    behavior:
      drain_journal_first: required
      if_drain_incomplete: 
        next_owner_recovers_from_journal: required
        fencing_token_required: required (see §9)
        rationale: 防止 split-brain 兩個 sidecar 同時 own 同 reservation
  
  autoscaler_specific:
    scale_to_zero:
      drain_before_zero: required
      cold_start_on_next_invocation: cache_miss_acceptable
      slo_implication: cold p99 path 不計入 warm Contract §14 SLO
    
    scale_down_replicas:
      pod_being_evicted_drains: required
      cluster_autoscaler_respects_drain: required
    
    scale_up_replicas:
      new_pod_warm_cache: required_before_accepting_traffic
      readiness_probe: bundle_loaded + endpoint_reachable
    
    spot_interruption:                           # round-3 §14 answer #8
      grace_period: 2_minutes (typical)
      drain_action: complete_or_fail_closed_within_grace
      if_grace_too_short: fail_closed_and_recover_from_journal
  
  audit_event_on_drain:
    required: true
    captures:
      - drain_trigger
      - in_flight_count
      - drained_within_window
      - timeout_invoked
      - replay_actions_taken
      - fencing_epochs_during_recovery
```

---

## 12. Companion Spec Integration

### 12.1 Contract §6 Stage Ownership

```yaml
decision_transaction_stage_ownership:
  snapshot:        local_sidecar
  evaluate:        local_sidecar
  prepare_effect:  local_sidecar
  reserve:         remote_ledger (via local_sidecar gRPC)
  audit_decision:  remote_durable_store
                   # ⚠ specific store determined by §6.2 durability_mode_selection:
                   #   k8s_saas       → remote_decision_journal_ack
                   #   lambda         → canonical_ingest_ack (only allowed)
                   #   air_gapped     → persistent_local_wal_with_replay_guarantee
                   #   per Contract §6.3 Stage Persistence Deployment Matrix
  publish_effect:  in_process_adapter
  commit_or_release: remote_ledger
  audit_outcome:   remote_durable_store (same target as audit_decision)
```

**Cross-reference**：詳細 deployment-specific durability path 見 §6.2 + Contract §6.3。Audit_decision 在 publish_effect 前 durable ack 是 Contract §6.1「無 audit 則無 effect」不變式的物理保證。

### 12.2 Contract §14 Latency

```yaml
latency_budget_decomposition_with_region_affinity:
  contract_§14_p99_total: 50ms
  
  achievable_under:
    - per_workload_instance sidecar
    - same-region ledger (region affinity §10)
    - same-region decision journal
    - warm cache
  
  not_achievable_under:
    - cross-region ledger
    - eventual consistency requiring multi-region quorum
    - cold start
  
  cold_start_p99_separate_slo:
    target: 200ms
    status: hypothesis_until_poc_measured
    note: 不計入 warm SLO
```

### 12.3 Trace §10.1 Canonical Ingest

Sidecar 是 canonical producer signer；canonical_required path bypass OTel sampling；ack semantics required（per §6.2 selection matrix）。

### 12.4 Trace §13 Producer Trust

Sidecar 簽 canonical events（ed25519）；schema_bundle_id 隨每事件；clock_claims_untrusted；key epoch negotiation 通過 IPC handshake。

### 12.5 Strong Consistency Ledger Capability Flags

```yaml
ledger_capability_flags:                         # round-3 §14 answer #3
  defined_in: ledger_storage_rfc (Stage 1D)
  consumed_by: this_sidecar_spec
  
  capability_flags_consumed:
    - strong_global
    - single_writer_per_budget
    - eventual
  
  hard_enforcement_filter:
    requires: strong_global OR single_writer_per_budget
```

---

## 13. Phase 2+ Topology Modes

```yaml
phase_eligibility:
  phase_1_first_customer:
    saas_managed: yes
    customer_hosted_control_plane: no
    air_gapped: no
  
  phase_2:
    customer_hosted_control_plane: yes (with signed release pipeline)
  
  phase_3+:
    air_gapped: yes (full offline distribution + signed bundle pipeline)
  
  rationale: |
    Phase 1 first customer = limit blast radius and operational complexity.
    Self-hosted/air-gapped require pipelines / processes not yet built.
```

---

## 14. Resource Sizing & Cache Topology

```yaml
resource_sizing:
  per_workload_instance_sidecar:
    minimum: {memory: 128Mi, cpu: 100m}
    recommended: {memory: 256Mi, cpu: 500m}
    high_throughput: {memory: 512Mi-1Gi, cpu: 1000m-2000m}
  
  large_scale_optimization:
    shared_daemon_cache:
      pattern: DaemonSet
      allowed: yes
      purpose:
        - bundle_prewarm
        - matrix_prewarm
        - pricing_table_prewarm
        - schema_bundle_prewarm
      forbidden_for:
        - policy_evaluation
        - audit_decision
        - ledger_reservation
        - decision_publish
      rationale: |
        Read-only cache may be DaemonSet-shared;
        decision path MUST remain per-workload-instance.
    
    100_replicas_scenario:
      acceptable: yes (100 sidecar containers)
      optimization_via:
        - shared_daemon_cache for prewarm data
        - resource limits tuned per replica
      not_acceptable: shared decision sidecar across replicas
  
  noisy_neighbor_protection:
    cpu_limits: required
    memory_limits: required
    cgroup_isolation: required
  
  slo_implications:
    p99_decision_latency: 50ms (Contract §14)
    sidecar_share_of_p99: < 50% (預留 25ms 給 ledger / canonical ingest)
    sidecar_internal_p99_target: 25ms
  
  cost_attribution:
    sidecar_cost: customer_pays (their compute)
    rationale: per_workload_instance topology
```

---

## 15. Key Rotation with Epoch Negotiation

```yaml
key_rotation:
  spiffe_svid: yearly_via_spire_auto_rotation
  ed25519_producer_signing_key: yearly with 12-month dual key period
  bundle_trust_root: yearly_or_on_compromise
  mtls_ca: yearly
  
  key_epoch_negotiation_protocol:
    purpose: |
      sidecar/adapter 須能於 rotation 期間透明過渡，
      避免 in-flight decisions 被 expired key 簽章。
    
    protocol:
      handshake_includes_key_epoch: required
      adapter_sees_active_epochs: required
      sidecar_signs_with_active_epoch_at_decision_time: required
      old_signed_events_remain_verifiable_via_key_id: required
    
    backward_verification:
      retention_aligned_with_audit_retention: 7_years
      key_id_in_event: required (per Trace §13)
  
  short_rotation_for_low_traffic_tenants:
    allowed: yes
    constraint: old_keys_remain_verifiable_for_full_retention
    traffic_volume_irrelevant: confirmed
```

---

## 16. Serverless Lifecycle (Lambda Extensions)

```yaml
serverless_mode:
  aws_lambda_extensions:
    capability_max: semantic_adapter (via runtime hook)
    durability: canonical_ingest_ack only (per §6.2)
    
    init_phase:
      duration: max 10s (Lambda limit)
      sidecar_actions:
        - load_signed_manifest
        - load_endpoint_catalog
        - warm_essential_cache (bundle, matrix)
      latency_note: |
        init p99 與 invoke p99 必須分開測量；
        warm cache p99 不應包含 cold init
    
    invoke_phase:
      latency_budget: per_invocation
      durability: must_ack_before_response
      sidecar_actions:
        - process_decision_transaction
        - emit_canonical_events
        - ack from canonical ingest before invoke completes
    
    shutdown_phase:
      duration: max 2s
      drain_protocol: see §11
  
  cloud_run_sidecar:
    similar_lifecycle: required
    durability: canonical_ingest_ack
  
  vercel_edge_function:
    capability: limited_to_advisory_sdk_or_egress_proxy
    rationale: edge runtime cannot host long-lived sidecar
  
  capability_in_serverless:
    minimum: semantic_adapter
    not_supported: provider_key_gateway (key management requires persistent state)
```

---

## 17. v2.1 Patch Detail（記錄用）

進入 spec lock 前納入的 minor clarifications：

| Patch | 位置 | 內容 |
|---|---|---|
| Endpoint discovery | §8（NEW） | signed catalog from control plane；K8s DNS as resolution not source of truth；max_staleness normal 24h / critical 5m |
| Fencing token | §9（NEW） | source: decision_journal_or_ledger_lease；monotonic_epoch；compare_and_swap_on_recover；K8s lease 不是 financial truth source |
| Durability mode migration | §6.2 | dual_write_period: 30 days operational（非 7 年 audit retention） |
| Active-passive failover details | §10 | in-flight decisions 保留舊 region 若可達；否則 fencing recovery |
| K8s lease 角色澄清 | §9 | lifecycle assist OK，不是 financial ownership authority |

---

## 18. v1alpha1 完整範例

### 18.1 Helm chart values.yaml

```yaml
spendguard:
  tenant_id: tenant_abc
  
  region:
    sidecar: us-west-2
    ledger: us-west-2          # 必須同 region for hard enforcement
    journal: us-west-2
    canonical_ingest: us-west-2
  
  durability_mode: remote_decision_journal_ack   # k8s_saas_managed matrix
  
  enforcement_strength: semantic_adapter
  
  endpoint_catalog:
    pull_url: https://catalog.us-west-2.spendguard.ai
    push_url: wss://catalog-events.us-west-2.spendguard.ai
    max_staleness:
      normal: 24h
      critical: 5m
  
  fail_safe_manifest:
    pull_url: https://manifest.us-west-2.spendguard.ai
    push_url: wss://manifest-events.us-west-2.spendguard.ai
  
  resources:
    sidecar:
      limits: {memory: 256Mi, cpu: 500m}
      requests: {memory: 128Mi, cpu: 100m}
    daemon_cache:
      enabled: true
      limits: {memory: 512Mi, cpu: 200m}
  
  drain:
    terminationGracePeriodSeconds: 60
  
  fencing:
    source: decision_journal_lease
    epoch_check: required
```

### 18.2 K8s deployment（per-pod sidecar）

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: customer-agent-service
spec:
  replicas: 5                                    # 5 pods, 5 sidecars
  template:
    metadata:
      annotations:
        spendguard.io/inject: "true"
        spendguard.io/region: us-west-2
    spec:
      terminationGracePeriodSeconds: 60          # drain window
      
      containers:
        - name: customer-app
          image: customer/agent-app:v1.2.3
          env:
            - name: SPENDGUARD_SIDECAR_SOCKET
              value: "/var/run/spendguard/adapter.sock"
        
        - name: spendguard-sidecar
          image: spendguard/sidecar:v0.1.0-alpha
          env:
            - name: REGION
              value: us-west-2
            - name: ENDPOINT_CATALOG_URL
              value: catalog.us-west-2.spendguard.ai
            - name: DURABILITY_MODE
              value: remote_decision_journal_ack
            - name: ENFORCEMENT_STRENGTH
              value: semantic_adapter
          
          lifecycle:
            preStop:
              exec:
                command: ["/bin/spendguard", "drain", "--timeout=55s"]
          
          readinessProbe:
            exec:
              command: ["/bin/spendguard", "ready"]
            initialDelaySeconds: 5
            periodSeconds: 10
          
          livenessProbe:
            exec:
              command: ["/bin/spendguard", "alive"]
```

### 18.3 DaemonSet shared cache

```yaml
apiVersion: apps/v1
kind: DaemonSet
metadata:
  name: spendguard-cache
  namespace: spendguard-system
spec:
  template:
    spec:
      containers:
        - name: cache
          image: spendguard/daemon-cache:v0.1.0-alpha
          env:
            - name: REGION
              value: us-west-2
            - name: BUNDLE_REGISTRY
              value: bundle-registry.spendguard.ai
          # Read-only cache only
          # NOT used for decision path
```

### 18.4 Lambda Extension

```yaml
runtime: aws_lambda
extension_layer:
  arn: arn:aws:lambda:us-west-2:spendguard:layer:sidecar-extension:v0.1.0-alpha
  lifecycle: extensions_api

function_environment:
  SPENDGUARD_TENANT_ID: tenant_abc
  SPENDGUARD_DURABILITY_MODE: canonical_ingest_ack   # only allowed for Lambda
  CANONICAL_INGEST_URL: https://canonical-ingest.us-west-2.spendguard.ai
  ENFORCEMENT_STRENGTH: semantic_adapter
```

---

## 19. Reference Implementation POC Plan

### 19.1 必達成的實作項

1. Helm chart with sidecar injection / preStop drain / readiness gating / region affinity values
2. Pydantic-AI 與 LangGraph adapters 實作 UDS handshake + capability claim
3. Remote decision journal ack mode for K8s SaaS path
4. Canonical ingest ack mode for Lambda path

### 19.2 Chaos Tests（7 個）

```yaml
chaos_tests:
  - pod_eviction_during_in_flight_decision
  - rolling_restart_with_drain
  - spot_interruption_within_2_minute_grace
  - decision_journal_unavailable_with_failover
  - region_endpoint_failover_active_passive
  - fencing_split_brain_with_epoch_compare
  - canonical_ingest_down_with_durability_fallback_per_route
```

### 19.3 Required Metrics

```yaml
metrics:
  - decision_stage_p99_ms (split by stage: snapshot / evaluate / reserve / audit_decision / publish_effect)
  - drain_timeout_count
  - journal_replay_count
  - endpoint_catalog_staleness_seconds
  - fencing_failures_count
  - cache_hit_rate (bundle / matrix / pricing)
  - canonical_ingest_buffer_depth
  - canonical_ingest_drop_count (observability_route only)
  - mtls_handshake_failures
  - signature_verification_failures
  - decision_transaction_retry_count
```

### 19.4 End-to-End Scenarios

1. K8s SaaS quickstart：Pydantic-AI adapter + sidecar + remote journal → 完整 audit chain
2. Lambda：semantic_adapter + canonical_ingest_ack → invoke p99 內 ack
3. Air-gapped POC（Phase 2+ 預演）
4. Multi-region active-passive failover（in-flight 恢復）
5. PVC unmount + fencing recovery split-brain test

---

## 20. Companion Compatibility Policy（alpha）

| 承諾 | 細節 |
|---|---|
| **Region affinity 不可變更** | 客戶 install 時鎖定 region；migration require dual deployment |
| **Durability mode migration** | 30 天 dual-write period；audit retention 7 年由 records 自身控制 |
| **Endpoint catalog signed** | Ed25519 signed；max staleness normal 24h / critical 5m |
| **Fencing token monotonic epoch** | 由 ledger/journal lease 提供；K8s lease 僅 lifecycle assist |
| **Helm chart 隨 spec 同步** | v1alpha1 spec 與 helm chart 同步 release |
| **Sidecar binary 簽章** | Ed25519 signed releases；Sigstore transparency log |
| **Old/new evaluator dual-run** | Migration 期間並行；客戶可 diff |
| **Alpha SLA** | Sidecar 99.5% availability；GA 為 99.95% |

---

## 21. Adoption History

| Round | Codex 反饋 | 採納率 | 主要產出 |
|---|---|---|---|
| Round 1 | 7 個 partial / 反駁點（含致命 topology vs identity 混淆） | 100% | Topology fix per_workload_instance；enforcement strength 4 級；UDS peer credentials；emptyDir 禁止；signed fail-safe manifest；§9 升 6 項 high-irreversibility |
| Round 2 | 3 個新 high-irreversibility gap（region affinity / durability matrix / lifecycle drain） | 100% | §10 region affinity；§11 durability selection matrix；§12 lifecycle drain；7 項 refinement |
| Round 3 | Minimal verification | 100% | v2.1 patch（endpoint discovery + fencing token + durability migration period 30 天） → **LOCK** |

---

## 22. Lock 後的下一步

1. **Reference impl POC 開工**（§19）— 與本 spec 平行展開
2. **Ledger storage model RFC**（Stage 1D）— 開始下一個 RFC（依賴 sidecar 對 ledger 的需求 + capability flags）
3. **First customer design partner onboarding**（K8s SaaS-managed 模式 + Helm chart）
4. **Endpoint catalog service** 實作（§8 signed catalog from control plane）
5. **Decision journal service** 實作（§6.2 option_2 for K8s SaaS path）

---

*Document version: sidecar-architecture-spec-v1alpha1 (LOCKED) | Generated: 2026-05-07 | Adoption: 100% across 3 Codex rounds | POC prerequisites listed §0.2 | GA prerequisites listed §0.3 | Companion: agent-runtime-spend-guardrails-complete.md (v1.3) + contract-dsl-spec-v1alpha1.md (LOCKED) + trace-schema-spec-v1alpha1.md (LOCKED)*
