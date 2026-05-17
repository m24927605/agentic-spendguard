# Issue #59 — Approval resume frozen-at-PRE pricing

> **Status**: spec v1 — design draft. Not yet implementation-locked. Codex r0 not yet run.
> **Source**: P2 followup from PR #58 (`fix(issue #9): close the REQUIRE_APPROVAL → resolve → resume loop end-to-end`).
> **Goal**: close the pricing-drift attack surface between REQUIRE_APPROVAL emission time and the operator's resume.
> **Audience**: implementer (or codex agent) closing #59. Reader should be able to read this spec + the referenced files and have zero open design questions.
> **Capability touched**: spec §4.1.5 PricingFreeze semantics (frozen-at-PRE) — currently honored by the CONTINUE/STOP paths but violated by the resume path.

---

## 1. Context — what gap this closes

PR #58 closed the REQUIRE_APPROVAL → control-plane resolve → ResumeAfterApproval round-trip and shipped four runtime bugs. Two P2 limitations were inlined as POC carve-outs; **this spec closes P2-2**: the resume path constructs the `PricingFreeze` proto from the sidecar's currently-installed contract bundle metadata, not from the pricing snapshot frozen at REQUIRE_APPROVAL time.

**The attack surface**:

```
T+0    contract bundle B0 installed; pricing $0.02/token
T+1    DecisionRequest: claim 500M atomic (= 25M tokens @ $0.02)
T+2    sidecar evaluator → REQUIRE_APPROVAL @ B0 pricing
T+3    post_approval_required_decision SP writes approval_requests row
       — decision_context_json captures budget_id, bundle hash, fencing,
         but NOT pricing_version/price_snapshot_hash
T+4    operator reviews "approve $0.02 × 25M tokens = $500" — approves
T+5    CA-P3.7 hot-reload: bundle B1 installed; pricing changed to $0.01/token
T+6    SDK e.resume(client) → sidecar ResumeAfterApproval
T+7    sidecar reads B1's pricing (currently-installed bundle), builds
       ReserveSetRequest with $0.01 pricing → reserves 25M tokens but
       SEMANTICALLY only $250 worth of budget (vs the $500 the operator
       approved).
```

**Existing partial mitigation**: `decision_context_json.contract_bundle_hash_hex` is captured at REQUIRE_APPROVAL time. If the resume could cross-check the bundle hash and refuse on mismatch, the attack surface collapses to "operator must reissue approval after hot-reload" (acceptable behavior). But the resume path never reads `contract_bundle_hash_hex` from decision_context.

**Why it's P2 not P1**: POC demo runs sub-second on a fresh volume; no hot-reload fires between approval and resume. Production deployments with long approval TTLs (hours) + scheduled bundle rotations make this a real risk.

---

## 2. Design

Two equally valid designs. Pick **Design A** (frozen-pricing) for v1 because it's the cleanest invariant; Design B (hash-pin only) is the smaller change set but leaves pricing-drift behavior fragile.

### Design A (chosen): freeze pricing in `decision_context_json` at REQUIRE_APPROVAL time

At sidecar's `run_record_denied_decision` REQUIRE_APPROVAL path, capture the four pricing fields into `decision_context_json` alongside the existing bundle hash. At resume time, read them back from the parsed payload and use them to construct `PricingFreeze` for `Ledger.ReserveSet`. Independently of pricing, also assert `contract_bundle_hash_hex` matches the sidecar's currently-loaded bundle; if not, return a typed `BUNDLE_HOT_RELOADED` error so the SDK can surface a clean "reissue your approval" path.

```
REQUIRE_APPROVAL emit (sidecar/decision/transaction.rs:632)
    decision_context_json:
        contract_bundle_hash_hex: <existing>
      + pricing_version:          <bundle.pricing_version>
      + price_snapshot_hash_hex:  hex(bundle.price_snapshot_hash)
      + fx_rate_version:          <bundle.fx_rate_version>
      + unit_conversion_version:  <bundle.unit_conversion_version>
        ...

resume (sidecar/server/adapter_uds.rs:approval_resume_payload::into_reserve_set_request)
    parsed = decode(approval_requests.decision_context)
    live_bundle = state.inner.contract_bundle.read()

    if parsed.contract_bundle_hash_hex != hex(live_bundle.bundle_hash):
        return Err("[BUNDLE_HOT_RELOADED] approval was issued under a
                    different bundle; reissue required")

    pricing = PricingFreeze {
        pricing_version: parsed.pricing_version,                       // frozen-at-PRE
        price_snapshot_hash: hex_decode(parsed.price_snapshot_hash_hex),
        fx_rate_version: parsed.fx_rate_version,
        unit_conversion_version: parsed.unit_conversion_version,
    }
```

### Design B (rejected): bundle-hash check only, leave pricing live

Same hash check, but read pricing from `live_bundle` if hash matches. Pricing is implicitly frozen because bundle hash being equal means the metadata.json is byte-equal too. Looks simpler but couples a derived property (pricing same iff bundle same) to a transport assertion (hash equality), and breaks the moment bundles can mutate pricing without changing contract identity. Rejected.

### Why decision_context not requested_effect

`requested_effect` is a per-claim payload (unit_id, amount_atomic, direction). Pricing is a per-snapshot property of the bundle, not a per-claim property. Putting it in `decision_context` keeps the conceptual layering clean and avoids forcing every claim to carry redundant pricing copies.

---

## 3. Components

### 3.1 Sidecar producer (write side)

**File**: `services/sidecar/src/decision/transaction.rs:632-665`

Current shape (verbatim from PR #58 commit `01aa080`):

```rust
let decision_ctx = serde_json::json!({
    "tenant_id":                       ctx.tenant_id,
    "budget_id":                       primary_claim.map(|c| c.budget_id.clone()).unwrap_or_default(),
    "window_instance_id":              primary_claim.map(|c| c.window_instance_id.clone()).unwrap_or_default(),
    "fencing_scope_id":                fencing.scope_id,
    "fencing_epoch":                   fencing.epoch,
    "decision_id":                     decision_id.to_string(),
    "matched_rule_ids":                matched_rules,
    "reason_codes":                    reason_codes,
    "contract_bundle_id":              bundle.bundle_id.to_string(),
    "contract_bundle_hash_hex":        hex::encode(&bundle.bundle_hash),
    "schema_bundle_id":                state.inner.schema_bundle.read().as_ref().map(|s| s.bundle_id.to_string()).unwrap_or_default(),
    "schema_bundle_canonical_version": state.inner.schema_bundle.read().as_ref().map(|s| s.canonical_schema_version.clone()).unwrap_or_default(),
});
```

Add four fields:

```rust
    "pricing_version":          bundle.pricing_version.clone(),
    "price_snapshot_hash_hex":  hex::encode(&bundle.price_snapshot_hash),
    "fx_rate_version":          bundle.fx_rate_version.clone(),
    "unit_conversion_version":  bundle.unit_conversion_version.clone(),
```

### 3.2 Sidecar resume (read side)

**File**: `services/sidecar/src/server/adapter_uds.rs:approval_resume_payload`

Extend the `DecisionContext` deserialize struct:

```rust
#[derive(Debug, Deserialize)]
pub struct DecisionContext {
    pub tenant_id: String,
    pub budget_id: String,
    pub window_instance_id: String,
    pub fencing_scope_id: String,
    pub fencing_epoch: u64,
    pub decision_id: String,
    #[serde(default)]
    pub matched_rule_ids: Vec<String>,
    #[serde(default)]
    pub reason_codes: Vec<String>,
    pub contract_bundle_id: String,
    pub contract_bundle_hash_hex: String,
    #[serde(default)]
    pub schema_bundle_id: String,
    #[serde(default)]
    pub schema_bundle_canonical_version: String,
    // ─── NEW (issue #59) ─────────────────────────────────────────────
    // Frozen at REQUIRE_APPROVAL time. Resume must reconstruct
    // PricingFreeze from these fields, NOT from the live bundle.
    pub pricing_version: String,
    pub price_snapshot_hash_hex: String,
    pub fx_rate_version: String,
    pub unit_conversion_version: String,
}
```

Update `into_reserve_set_request`:

```rust
// Bundle-hash hot-reload check (fail-closed).
let live_bundle = state.inner.contract_bundle.read().clone()
    .ok_or_else(|| "no contract bundle installed".to_string())?;
let live_hash_hex = hex::encode(&live_bundle.bundle_hash);
if self.decision.contract_bundle_hash_hex != live_hash_hex {
    return Err(format!(
        "[BUNDLE_HOT_RELOADED] approval was issued under bundle {} but \
         the sidecar's currently-installed bundle is {}; reissue required",
        self.decision.contract_bundle_hash_hex, live_hash_hex
    ));
}

// Reconstruct PricingFreeze from frozen-at-PRE fields.
let price_snapshot_hash = hex::decode(&self.decision.price_snapshot_hash_hex)
    .map_err(|e| format!("price_snapshot_hash_hex decode: {e}"))?;
let pricing = PricingFreeze {
    pricing_version:         self.decision.pricing_version.clone(),
    price_snapshot_hash:     price_snapshot_hash.into(),
    fx_rate_version:         self.decision.fx_rate_version.clone(),
    unit_conversion_version: self.decision.unit_conversion_version.clone(),
};
```

Delete the inline `state.inner.contract_bundle.read()` block for pricing extraction (the bundle is still read above for the hash check, but pricing now comes from decision_context).

### 3.3 SDK surface (Python)

**File**: `sdk/python/src/spendguard/errors.py` + `client.py`

Add a typed error class so the resume's `[BUNDLE_HOT_RELOADED]` surfaces as a structured exception in user code rather than a raw `SpendGuardError`:

```python
class ApprovalBundleHotReloadedError(SpendGuardError):
    """Raised when the operator's approval was issued under a different
    contract bundle than is currently installed in the sidecar. The
    approval is semantically stale; the agent must reissue the original
    DecisionRequest to get a fresh approval row tied to the new bundle.
    """
    def __init__(self, message, *, original_bundle_hash, current_bundle_hash):
        super().__init__(message)
        self.original_bundle_hash = original_bundle_hash
        self.current_bundle_hash = current_bundle_hash
```

In `client.py:resume_after_approval`, when the sidecar returns a string error containing `[BUNDLE_HOT_RELOADED]`, parse the two hashes from the message and raise `ApprovalBundleHotReloadedError`.

### 3.4 Verify SQL

**File**: `deploy/demo/verify_step_approval.sql`

§B currently checks `decision_context_bytes < 50 OR requested_effect_bytes < 50` for sanity. Add a structural check that the four new pricing fields exist:

```sql
DO $$
DECLARE
    has_pricing_fields BOOLEAN;
BEGIN
    SELECT bool_and(
        decision_context ? 'pricing_version'
        AND decision_context ? 'price_snapshot_hash_hex'
        AND decision_context ? 'fx_rate_version'
        AND decision_context ? 'unit_conversion_version'
    )
      INTO has_pricing_fields
      FROM approval_requests
     WHERE created_at > now() - interval '5 minutes'
       AND state = 'approved';
    RAISE NOTICE '[verify] §B+ pricing fields present: %', has_pricing_fields;
    IF NOT has_pricing_fields THEN
        RAISE EXCEPTION '§B+ FAIL: decision_context missing the 4 issue-59 pricing fields';
    END IF;
END$$;
```

---

## 4. Implementation slices

### Slice 1 — wire pricing through (server side)

Touch: `services/sidecar/src/decision/transaction.rs` (4 lines added to the JSON).
Tests: existing demo regression (DEMO_MODE=approval) must still PASS.
Codex: r1 review expected. Look for: hex encoding mismatch with the proto's `bytes` field for `price_snapshot_hash`; non-empty empty-string corner case.

### Slice 2 — wire pricing through (resume side) + bundle-hash check

Touch: `services/sidecar/src/server/adapter_uds.rs` `approval_resume_payload` struct + `into_reserve_set_request`. Update `verify_step_approval.sql` to assert the four pricing fields exist + run.
Tests: DEMO_MODE=approval must PASS with the new fields populating end-to-end.
Codex: r1 review. Look for: hash check using full bundle_hash (32 bytes) not truncated (16); hex decode failure path; race between `state.inner.contract_bundle.read()` calls if held twice (hold once into a local Arc clone).

### Slice 3 — SDK typed error

Touch: `sdk/python/src/spendguard/errors.py` (new class) + `client.py:resume_after_approval` (parse + raise).
Tests: new unit test asserting `ApprovalBundleHotReloadedError` is raised when the sidecar returns a `[BUNDLE_HOT_RELOADED]` string error.
Codex: r1 review. Look for: regex/string parsing brittleness; thread safety; error subclassing.

### Slice 4 — hot-reload regression demo

Touch: `deploy/demo/demo/run_demo.py` adds a `DEMO_MODE=approval_hot_reload` variant that:
1. Triggers REQUIRE_APPROVAL.
2. Rotates the contract bundle via `bundle_registry` (or directly via the bundles-init container) so the sidecar's `runtime.env` watcher picks up a new hash.
3. Calls `e.resume(client)`.
4. Asserts the SDK raises `ApprovalBundleHotReloadedError`.

`Makefile` adds the new mode + verify SQL.

Tests: full demo PASS on fresh volume.
Codex: r1 review. Look for: timing race between bundle rotation and resume (`sleep 2`-style hacks → real `kubectl wait`-style hash polling).

### Slice 5 — final sweep + memory + memory entry update

Touch: `memory/feedback_codex_review.md` (no), update memory `project_overview.md` with the issue-59 closure summary; spec status moves to LOCKED; commit-message references PR #58 + issue #59.

---

## 5. Test plan

### Unit (Rust, Python)

- `services/sidecar/src/decision/transaction.rs` — round-trip test: build `decision_ctx` from a mock `bundle`, serialize to JSON, deserialize via `approval_resume_payload::DecisionContext`, assert all four pricing fields match byte-for-byte.
- `services/sidecar/src/server/adapter_uds.rs` — table test: given parsed payload + a mock `state.inner.contract_bundle`, assert (a) matching hash returns Ok; (b) mismatching hash returns `[BUNDLE_HOT_RELOADED]`; (c) None bundle returns "no contract bundle installed".
- `sdk/python/tests/test_errors.py` — assert `ApprovalBundleHotReloadedError` parses both hashes from a sidecar-shaped error string + inherits from `SpendGuardError`.

### Integration (kind/docker via demo)

- `DEMO_MODE=approval make demo-up` — existing demo must PASS (no regression).
- `DEMO_MODE=approval_hot_reload make demo-up` — new mode must PASS:
  - REQUIRE_APPROVAL emitted under bundle B0
  - Bundle rotated to B1 (different hash)
  - resume() raises `ApprovalBundleHotReloadedError` with both hashes populated
- verify_step_approval.sql §B+ pricing-fields-present check must PASS in the existing demo (not just the hot_reload variant).

### Adversarial (codex)

- Empty / malformed `price_snapshot_hash_hex` in decision_context — must fail-closed.
- Mismatched bundle hash between approval and resume time — must surface `[BUNDLE_HOT_RELOADED]`, not silently use live pricing.
- Replay path on retry (idempotency_key collision) — verify the `approval_request_id: String::new()` POC gap from PR #58 is NOT regressed; this issue is orthogonal to that one.
- Bundle hot-reloads BACK to the original between approval and resume — hash matches again → resume should succeed (idempotent semantics).

---

## 6. Acceptance criteria

- [ ] All four pricing fields land in `decision_context_json` at REQUIRE_APPROVAL time (verified by §B+ verify SQL).
- [ ] Resume reads pricing from `decision_context_json`, not from `state.inner.contract_bundle.read()`.
- [ ] Resume returns `[BUNDLE_HOT_RELOADED]` typed error when bundle hash differs between approval and resume.
- [ ] Python SDK surfaces the typed error as `ApprovalBundleHotReloadedError(original_bundle_hash, current_bundle_hash)`.
- [ ] `DEMO_MODE=approval make demo-up` regression PASSes.
- [ ] `DEMO_MODE=approval_hot_reload make demo-up` new mode PASSes.
- [ ] No new `expose_secret()` call sites in sidecar (pricing fields aren't secrets but the audit invariant holds).
- [ ] Codex review reaches GREEN within 5 rounds; Staff escalation playbook from `auto-instrument-egress-proxy-spec.md` §14.1 triggers on r5 RED.

---

## 7. Code review standards (codex prompts)

**r1 adversarial focus**:
- Is `price_snapshot_hash_hex` encoded as hex on write + decoded as hex on read? Both sides must use the same `hex` crate API.
- Is the bundle-hash equality check using the full 32-byte hash (`hex::encode` of full slice), not a truncated form?
- What happens on `bundle_registry` rotating between read of `decision_context` and read of `state.inner.contract_bundle`? Is the check TOCTOU-safe (hold one Arc clone for the whole operation)?
- Does the new `ApprovalBundleHotReloadedError` subclass properly preserve the `original_bundle_hash` + `current_bundle_hash` across pickle / asyncio crossings?
- Demo verify SQL §B+: does it use `?` operator on JSONB correctly (postgres ≥ 9.4 only)?
- Is the new `DEMO_MODE=approval_hot_reload` actually exercising a real bundle rotation, or stubbing it via env var? Real rotation only.

**r2-r5 expected patterns**:
- TL;DR vs body inconsistencies (recurring per `auto-instrument-egress-proxy-spec.md` r1/r3/r4/r6).
- Off-by-one in idempotency_key derivation if any.
- Operator UX: error message must include both hashes so the operator can confirm "yes I rotated the bundle on purpose".

**Staff escalation triggers** (per `auto-instrument-egress-proxy-spec.md` §14.1):
- 4-of-4 codex rounds RED with no productive convergence → Staff (distributed-systems / security / ledger-audit).
- Discovery of unstated invariant (e.g., approval TTL semantics interact with bundle rotation cadence) → Staff before committing.

---

## 8. Demo verification

Per memory `feedback_demo_quality_gate.md`: every service must really run. The closure gate for this issue is:

```bash
$ make demo-down -v
$ DEMO_MODE=approval make demo-up
... (existing PASS preserved) ...
[verify] §B+ pricing fields present: true

$ make demo-down -v
$ DEMO_MODE=approval_hot_reload make demo-up
[demo] REQUIRE_APPROVAL raised approval_id=<U1> decision_id=<D1> under bundle hash B0=...
[demo] rotating bundle to B1...
[demo] sidecar /contract reports hash=B1
[demo] control-plane resolved approval_id=<U1> -> approved
[demo] resume() raised ApprovalBundleHotReloadedError(original=B0, current=B1)
[demo] PASS — frozen-at-PRE pricing invariant verified end-to-end
```

---

## 9. Deferred items (NOT shipped in #59)

- KMS-backed signing of `decision_context_json` so the pricing fields can't be tampered with at rest. The audit chain signature on the carrier `ledger_transaction` row already covers the decision-context payload transitively (via the audit row's CloudEvent), so this is belt-and-suspenders.
- Approval-time pricing audit endpoint that surfaces "the pricing the operator approved was X" to compliance reviewers. Useful for SOC2 but not load-bearing for the invariant.
- Multi-bundle approvals (an approval that's valid across multiple bundle hashes by policy). v0.1 is strict single-hash; multi-hash is a contract-DSL extension.

---

## 10. References

- PR #58 — original closed-loop implementation
- Memory `feedback_demo_quality_gate.md` — demo as quality gate
- Memory `feedback_codex_review.md` — adversarial review standard
- Memory `project_overview.md` "CA-P3.7 — sidecar contract bundle hot-reload" — the hot-reload code path that creates the risk surface
- `services/sidecar/src/decision/transaction.rs:632-665` — REQUIRE_APPROVAL emit site
- `services/sidecar/src/server/adapter_uds.rs:approval_resume_payload` — resume read site
- `services/ledger/migrations/0037_post_approval_required_decision_sp.sql` — SP that captures decision_context_json (no schema change needed; JSONB already accepts the new fields)
