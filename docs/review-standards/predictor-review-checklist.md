# Predictor Review Checklist (universal + per-slice)

> 🔍 **Use this checklist during every adversarial review round** for every slice in the predictor-upgrade spec set. Universal checks (§1) apply to all slices; per-slice extensions (§2) are documented in the slice's own `§9` section.
>
> **Round-pass rule** (per HANDOFF §9 + maintainer's 2026-05-29 directive): a round passes when the reviewer's finding list is empty after fixes. Severity labels (Blocker / Major / Minor) are for triage + changelog only; **every finding gates the round**.

---

## §0. How to use this checklist

### 0.1 Per AIT round flow

```
ait run --adapter claude-code --review adversarial --review-budget deep ...
  ↓ adversarial reviewer runs through:
     §1 (universal checks) + slice's §9 (per-slice extensions)
  ↓ reviewer emits findings
ait review finding list --attempt <id>
  ↓ implementer fixes ALL findings
  ↓ re-run review
  ↓ repeat up to 5 rounds
  ↓ round-5 fail → Staff+ panel per `staff-panel-arbitration-process.md`
```

### 0.2 Reviewer mindset

- Default to finding 3-5 issues per round (per HANDOFF Evidence Collector / Reality Checker patterns)
- Require evidence for every "looks fine" claim
- Trust no fix that wasn't tested
- Cross-spec consistency is a hard fail（spec A 與 spec B 不一致 = blocker）

---

## §1. Universal checks (applies to every slice)

### 1.1 Audit-chain coverage

- [ ] Every new field in this slice is written to `audit_outbox` AND replicated to `canonical_events`
- [ ] Every new field is included in `cloudevent_payload_signature` derivation（mirror at CloudEvent proto tag 300+ per `audit-chain-prediction-extension-v1alpha1.md` §3）
- [ ] Every new field is added to `reject_audit_outbox_immutable_columns` trigger function's OLD/NEW comparison list（per audit-chain extension §5.2）
- [ ] `verify-chain` regression test runs against both (a) pre-existing rows with NULL new fields and (b) freshly-written rows with new fields populated
- [ ] No new field bypasses `verify_cloudevent` signature check

### 1.2 Tokenizer Tier discipline

- [ ] Tier 2 hot-path latency p99 < 1ms (library form) measured + asserted in benchmark
- [ ] Tier 1 NEVER called synchronously from hot path; any caller invoking Tier 1 RPC from sidecar / egress_proxy decision flow is a blocker
- [ ] Tier 3 fallback rate < 0.1% in demo modes; emits metric per use; dispatch unknown-model `tokenizer_unknown_model{ model=... }` emitted

### 1.3 Strategy A as reservation under STRICT_CEILING

- [ ] `STRICT_CEILING` policy: `reserved_strategy = 'A'` always written; reservation amount in `BudgetClaim` = Strategy A value
- [ ] Tests verify that under `STRICT_CEILING` even when B and C return values, reservation = A
- [ ] `ADAPTIVE_CEILING` path tested: reservation switches to A when `remaining_budget < 2 × A`
- [ ] `SHADOW_ONLY` path tested: B/C written to audit but reservation = A

### 1.4 Cold-start fallback chain (L4 → L3 → L2 → L1)

- [ ] Lookup algorithm matches `cold-start-baseline-spec-v1alpha1.md` §2.5 verbatim
- [ ] Each layer's "no match" path tested
- [ ] L4 promotion threshold (30 samples) enforced; smaller buckets fall through
- [ ] L3 deferred-build path returns None until enabled flag set
- [ ] L2 TOML entry not found → fall to L1 (B null)
- [ ] `cold_start_layer_used` column populated correctly per layer (NULL for L4)

### 1.5 Per-run projection Signal 1/2/3 layered

- [ ] Signal 1 always-computed; uses stats_aggregator run-length P95 or cold-start default 10
- [ ] Signal 2 dynamic re-projection every Project call
- [ ] Signal 3 override Signal 1 only when `planned_steps_hint > 0`
- [ ] Code precedence: `RUN_BUDGET_PROJECTION_EXCEEDED > RUN_STEPS_EXCEEDED > RUN_DRIFT_DETECTED`
- [ ] `RUN_STEPS_EXCEEDED` only emittable when Signal 3 active

### 1.6 Contract DSL strictly additive

- [ ] All proto changes are additive; no field number reuse, no enum value renumbering
- [ ] v1alpha1 contracts continue to evaluate byte-identically on v1alpha2 evaluator (8+ demo modes regression)
- [ ] v1alpha1 sidecar correctly refuses v1alpha2 contract bundle (per `contract-dsl-spec-v1alpha2.md` §8.1)
- [ ] Default `prediction_policy = STRICT_CEILING` filled for v1alpha1 contracts on v1alpha2 evaluator
- [ ] `prediction_policy × run_projection_action` allowed-pairs table enforced at bundle load

### 1.7 L0–L3 capability semantics unchanged

- [ ] Trust model L0 (advisory_sdk) / L1 (semantic_adapter) / L2 (egress_proxy_hard_block) / L3 (provider_key_gateway) semantics preserved
- [ ] `enforcement_strength_required` field semantics in contract DSL unchanged
- [ ] `capability_required` mismatch refuses to load (per Sidecar §3.3) for new policies

### 1.8 Failure isolation per dependency

- [ ] tokenizer service unreachable → egress_proxy fall-closed (per `tokenizer-service-spec-v1alpha1.md` §8)
- [ ] output_predictor service unreachable → sidecar conservative fall-through (A only)
- [ ] customer plugin unreachable / error → fall to Strategy B with `customer_predictor_error` metric (per `output-predictor-plugin-contract-v1alpha1.md` §5)
- [ ] run_cost_projector unreachable → sidecar pass-through (no RUN_* emitted; reservation still correct)
- [ ] stats_aggregator cache stale > 2h → fall through to cold-start chain
- [ ] Tier 1 endpoint outage → circuit breaker open; hot path unaffected

### 1.9 Multi-tenant isolation

- [ ] Every cache table has RLS policy on `tenant_id`
- [ ] Every aggregation query includes explicit `tenant_id = $param` predicate
- [ ] Customer plugin endpoint per-tenant; no multi-tenant fan-in (per `output-predictor-plugin-contract-v1alpha1.md` §7)
- [ ] Adversarial cross-tenant injection test runs in CI

### 1.10 Observability

- [ ] Every new code path emits per-handler `ok / err` counter
- [ ] Every new code path emits structured log with tenant_id + decision_id + run_id
- [ ] Latency histograms for hot-path RPCs
- [ ] `customer_predictor_*` metrics labeled with tenant
- [ ] Drift alerts emit CloudEvents (not just log lines)

### 1.11 Backwards compatibility

- [ ] All 8+ existing demo modes (`make demo-up DEMO_MODE=...`) still pass
- [ ] Existing SDK integrations (litellm / langchain / pydantic_ai / openai_agents / agt) still work with caller-supplied `claim_estimator` parameter
- [ ] Default behavior preserved when new policy / strategy fields omitted

### 1.12 SLO budgets respected

- [ ] Contract §14 50ms p99 sidecar budget maintained
- [ ] Component-level SLOs (per each spec §10/§11) verified by benchmark
- [ ] Cold start budget separate from warm budget (per §14 v1.1)

---

## §2. Per-slice extensions

Each slice's own doc (`docs/slices/SLICE_XX_*.md`) has a `§9` section with concrete adversarial questions specific to that slice. Reviewer must run §1 + slice's §9.

Example questions slices might add:

- (SLICE 03 tokenizer skeleton) — How does the dispatch table handle `gpt-4o-2024-08-06`? `gpt-4o-mini-2024-07-18`? Show test fixtures.
- (SLICE 07 plugin contract) — Plugin returns `predicted_output_tokens = -1`. Test: fall to B? Metric emitted? Circuit breaker count?
- (SLICE 09 run_cost_projector) — Two concurrent `Project` calls for same run_id. Test: state cache atomicity? No race condition on `cumulative_cost` update?

---

## §3. Severity definitions (for triage/changelog only)

per HANDOFF §9.1:

- **Blocker** — correctness bug; security vulnerability; breaks existing invariant (audit chain immutability, signature canonicalization, fencing single-writer-per-budget); breaks public API; any test failure; any audit-chain `verify-chain` regression.
- **Major** — uncovered edge case; performance regression vs current baseline; observability gap (missing metric / log / trace); spec / implementation mismatch.
- **Minor** — naming inconsistency; comment quality; light refactor; documentation polish.

**Reminder**: severity is for triage / changelog. **All findings of all severities must be fixed** before the round counts as passing.

---

## §4. Round-pass rule (locked)

> Round pass = the round's reviewer finding list is empty after fixes.

Severity labels are NOT a pass gate. A round with 10 Minor findings does NOT pass until those 10 are fixed.

Round 5 still fail → Staff+ panel arbitration per `staff-panel-arbitration-process.md`.

---

*Checklist version: predictor-review-checklist v1alpha1 | Used by every adversarial review round across SLICE_01 to SLICE_15 | Companion: `staff-panel-arbitration-process.md` for round-5 failure escalation*
