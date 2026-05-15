# Cost Advisor P3.8 — Multi-Budget Index Pinning via Bundle-Registry Remap

> **Date**: 2026-05-15
> **Branch / commit**: `feat/cost-advisor-p3.8-multi-budget-index-pinning`
> **Demo**: `DEMO_MODE=cost_advisor make demo-up` PASS on fresh volume with new 2-budget contract; sidecar `event=hot_reload_swapped` still fires once per rotation (CA-P3.7 path unchanged)
> **Codex iteration**: r1 GREEN on first pass (no P1/P2; two P3 future-proofing notes folded into doc-comments)
> **Closes**: the last documented broken-UX gap in Cost Advisor v0.1 — multi-budget contracts now work end-to-end

---

## 1. The bug this slice fixes

CA-P3.1 locked the patch-emission contract: cost_advisor emits 2-op RFC-6902 patches with `test` + `replace` at the SAME budget array index, and `patch_validator` (both the Rust struct and the migration-0044 SQL CHECK constraint) enforces same-index pinning at write-time.

The rule's runtime, however, has no access to the active contract bundle (`services/cost_advisor/src/runtime.rs:496-535`). It emits paths at hardcoded `/spec/budgets/0/...` and pins identity with a `test` op carrying the offending budget's UUID. For single-budget contracts this works. For multi-budget contracts where the offending budget is at array index `j > 0`:

- The patch's `test` op fires `/spec/budgets/0/id == <offending_uuid>`.
- But `contract[0].id` is some OTHER budget's UUID.
- `json_patch::test` rejects → the whole patch is aborted by `json_patch::patch`.
- `bundle_registry::apply::process_approval` logs the apply error; the approval row stays in state=`approved` with no contract update; the operator is stuck (no introspection of which index the budget is actually at; no way to edit the patch since `approval_requests` is immutable post-resolve per migration 0029).

Documented in CA-P3.1's "UX caveat" header comment: "Multi-budget contracts where the offending budget isn't at array index 0 produce apply-failing proposals... Operator must reject + manually fix the bundle."

CA-P3.8 closes this gap **without changing the rule, the proto, the validator, or the writer**. The remap happens entirely inside `bundle_registry::apply::apply_patch_to_yaml`.

---

## 2. Design — transparent remap at apply time

### 2.1 What changes

```
Pre-CA-P3.8:
  cost_advisor → /spec/budgets/0/... patch → approval_requests
                          → operator approve
                          → bundle_registry::apply
                            → json_patch::patch  ❌ test op fails on multi-budget

Post-CA-P3.8:
  cost_advisor → /spec/budgets/0/... patch → approval_requests
                          → operator approve
                          → bundle_registry::apply
                            → remap_budget_indices  ✓ rewrites paths 0→j
                            → json_patch::patch     ✓ apply lands at real index
```

`remap_budget_indices` runs INSIDE `apply_patch_to_yaml`, between YAML→JSON parse and `json_patch::patch`. It takes the parsed contract value + the patch JSON; returns a rewritten patch.

### 2.2 Three-pass algorithm

**Pass 1 — collect pins** (`services/bundle_registry/src/apply.rs:230-265`):

Walk the patch ops, find every `test` op at the exact form `/spec/budgets/<i>/id` with a string value. Build a `BTreeMap<u32, String>` of `src_idx → pinned_uuid`. Reject (with `bail!`) conflicting pins at the same `src_idx` with different UUIDs — that's a structural bug in upstream. Same-UUID-twice at same index is idempotent (silently merged).

**Pass 2 — resolve pins against contract** (`apply.rs:268-280`):

For each pinned `(src_idx, uuid)`, scan `contract.spec.budgets[]` (an array) looking for an entry whose `.id` field equals `uuid`. If found at array position `j`, record `src_idx → j` in the remap. If `j == src_idx` no entry is recorded (no rewrite needed). If no match in any position, no entry is recorded — `json_patch::test` will fail naturally at apply time, surfacing as "apply error" to the operator (same UX as a stale-budget finding pointing at a budget that's been deleted).

**Pass 3 — rewrite paths uniformly** (`apply.rs:281-309`):

Walk the patch a second time. For each op whose `path` matches `/spec/budgets/<src>/<suffix>`, look up `src` in the remap; if found, rewrite the path's index segment to `<dst>`. Both `test` and `replace` ops at the same `src` are rewritten with the same `dst`, **so the same-index pinning invariant is preserved by construction post-remap**.

### 2.3 Preservation of CA-P3.1 invariant

This is the design's central claim: the validator's "every replace on `/spec/budgets/<i>/*` MUST have a preceding test op on `/spec/budgets/<i>/id` at the same `<i>`" rule is enforced at WRITE time against the EMITTER's index (always 0 today). The remap rewrites both ops uniformly using the SAME remap entry — they land on the SAME destination index. The post-remap patch always satisfies the same-index property even though it was structured at write-time around index 0.

This is verified by `test remap_preserves_first_budget_when_targeting_second` in the test suite: a 2-op patch at `/spec/budgets/0/*` against a 2-budget contract where the demo budget is at index 1 ends up applying both ops at index 1; the placeholder at index 0 stays bit-identical.

### 2.4 What this slice deliberately did NOT do

- **No proto changes** — `cost_advisor.proto`'s `FindingScope.budget_id` already carries the UUID; no new field needed.
- **No rule changes** — `idle_reservation_rate_v1` runtime in `runtime.rs:496-535` continues to emit naive index-0 patches.
- **No validator changes** — `patch_validator.rs` (Rust) and `cost_advisor_validate_proposed_dsl_patch` (migration-0044 SQL CHECK) are unchanged. The validator's same-index rule is satisfied by the EMITTER (which uses index 0 for both); the remap is invisible to the validator.
- **No DB migration** — the remap operates on patch JSON + contract YAML at apply time, in process memory.
- **No demo seed schema change** — `cost_advisor_demo_seed.sql` still seeds reservations against budget `44444444-...`. We achieve "exercise the remap on every cost_advisor demo run" by REORDERING the contract bundle (putting a placeholder at index 0) rather than seeding a 2nd real budget.

---

## 3. Components

| File | Lines added | Purpose |
|---|---|---|
| `services/bundle_registry/src/apply.rs` | +190 | `remap_budget_indices` 3-pass function, `parse_budget_id_path`, `parse_budget_path_prefix` helpers (mirror `patch_validator.rs::parse_budget_index` RFC-6901 rules), and **8 new unit tests** |
| `deploy/demo/init/bundles/generate.sh` | +20 / −10 | Contract bundle now ships with a placeholder budget at index 0 + the existing demo budget at index 1 |
| `deploy/demo/cost_advisor_demo.sh` | +35 / −10 | Step 5 now per-budget asserts TTL=45 at index 1 (the patched budget) AND TTL=600 at index 0 (placeholder unchanged) |

No proto, no migration, no other-service touch. The slice is contained.

---

## 4. Demo regression detection

The demo's step 5 makes TWO assertions:

1. `TTL_DEMO == 45` — budget at index 1 (the real demo budget) received the patch.
2. `TTL_PLACEHOLDER == 600` — budget at index 0 (placeholder) is bit-identical to its generate.sh-source value.

The second assertion is the actual nightmare-case regression detector. Without it, a remap bug that silently mutated budget #0 instead of budget #1 would pass assertion #1 by accident (TTL=45 on the wrong budget) and ship. The two-sided check forces "the patch went to the RIGHT budget, not the WRONG one."

The placeholder budget (`aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa`) is deliberately not referenced by any DB seed, any rule, or any other demo mode — it exists solely to force every cost_advisor demo run through the remap path. Without it the demo would never exercise the remap because the real budget would be at index 0.

---

## 5. Codex r1 review

Run on the full staged diff. Verdict: **SHIP** (first pass). No P1 / P2 findings.

### P3-1 — future-proofing if allowlist relaxes
File: `apply.rs:281` (Pass 3 loop).
Today's allowlist permits only `test` + `replace` ops, neither of which has a `from` JSON Pointer. If a future slice relaxes the allowlist to include `move` or `copy`, that slice must teach Pass 3 to also rewrite the `from` field — otherwise a relocated op would dangle post-remap.

**Folded into the slice as an inline doc-comment in Pass 3** (`apply.rs:282-293`), referencing `patch_validator.rs::rejects_add_op` / `rejects_remove_op` as the gates.

### P3-2 — demo awk lookup couples to `serde_yaml` key ordering
File: `cost_advisor_demo.sh:304-310` (`read_budget_ttl`).
The awk skips forward from `id: <uuid>` to the next `reservation_ttl_seconds:` line, which works because `serde_yaml` emits map keys alphabetically and `id` < `reservation_ttl_seconds`. A future serde_yaml version change could silently misfire the assertion. Switching to `yq` is the proper fix but adds a demo dep.

**Folded into the slice as an inline shell comment near `read_budget_ttl`** (`cost_advisor_demo.sh:298-308`), with the explicit recommendation to switch to `yq` if assertion misfires post-upgrade.

### Attack-vector verdicts the reviewer specifically validated

- **Wrongly-redirected ops at source index 0 with no test op**: cannot happen — validator's same-index pinning rule means every replace has a co-located test. ✓
- **Same-index invariant preservation**: preserved by construction — both ops rewritten using the same `src → dst` map entry. ✓
- **`add`/`remove` injection**: blocked by current allowlist; P3-1 covers future relaxation.
- **Two test ops pinning the SAME UUID at different source indices**: safe-but-weird; falls through to idempotent test ops on the same destination + replaces all landing on the same budget. The proposer's claim "UUID at both indices" is structurally false; failing into a no-op-merge is acceptable.
- **Demo awk fragility**: P3-2 covers this.
- **Pre-CA-P3.8 approval rows replayed post-upgrade**: safe — old single-budget patches pinning `$DEMO_BUDGET` at naive index 0 get remapped to index 1 in the new 2-budget contract. Apply succeeds. If a pinned UUID no longer exists in the contract at all, `json_patch::test` fails → recovery_apply_failed logged → operator sees a stuck approval (same UX as documented stale-budget finding).

---

## 6. Demo verification trace

```
$ DEMO_MODE=cost_advisor make demo-up   # fresh volume

[bundles] writing demo contract source... (2 budgets: placeholder@0, demo@1)
[bundles] contract bundle sha256: 34638ae0...

[cost-advisor-demo] step 2 OK: 1 budget-scoped finding emitted + 2-op patch
  patch: [
    {"op":"test",   "path":"/spec/budgets/0/id","value":"44444444-..."},
    {"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":45}
  ]                                  ← cost_advisor STILL emits at index 0
                                       (rule unchanged; bundle-unaware)

[cost-advisor-demo] step 4 OK: dashboard POST /resolve → approved

[cost-advisor-demo] step 5: bundle hash rotated 34638ae0 → ef946216 (1s)
                            CA-P3.8 remap routed patch to demo budget
                            at index 1 (TTL=45); placeholder at index 0
                            untouched (TTL=600) ✓

[cost-advisor-demo] step 6: sidecar /contract reports new hash <500ms
                            (CA-P3.7 hot-reload path unchanged)
```

Extracted post-patch contract.yaml:

```yaml
spec:
  budgets:
  - currency: USD
    id: aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa
    limit_amount_atomic: '100000000'
    require_hard_cap: false
    reservation_ttl_seconds: 600          ← unchanged
  - currency: USD
    id: 44444444-4444-4444-8444-444444444444
    limit_amount_atomic: '1000000000'
    require_hard_cap: true
    reservation_ttl_seconds: 45           ← patch landed here
```

**Pre-CA-P3.8 (counterfactual)**: with the same 2-budget contract, bundle_registry's `json_patch::patch` would fail at the test op: `contract[0].id = aaaaaaaa-...` but test op asserts `= 44444444-...`. Whole apply aborts. Step 5 timeout (10s) elapses with `OLD_HASH == runtime.env hash`. Demo step 5 fails loud with "bundle_registry did not rotate the bundle within 10s".

Other demo modes (decision/invoice/release/ttl_sweep/deny) reference budgets by UUID in rules + reservations, so the array reorder is invisible to them. Phase 3 deny lifecycle PASSed against the 2-budget contract during this slice's verification.

---

## 7. Deferred items

- **Future allowlist relaxation** must update Pass 3 to also rewrite `from` fields (P3-1).
- **CA-P3.9+ rules emitting non-budget patches** (e.g., rule-level patches at `/spec/rules/<i>/...`) would need an analogous remap for rule indices. Not on v0.1 roadmap.
- **`yq`-based demo lookup** would harden the assertion against serde_yaml ordering changes (P3-2). Defer until a YAML upgrade actually breaks it.
- **Rule-side bundle introspection** (alternative design) — making cost_advisor read the bundle and emit at the real index directly. Considered and rejected: cross-service file access in v0.1 is undesirable, and the apply-time remap is strictly simpler (one place, one algorithm, all the necessary inputs already available to bundle_registry).

---

## 8. References

- `services/bundle_registry/src/apply.rs:146-336` — remap implementation + 8 unit tests
- `services/bundle_registry/src/listener.rs:28-92` — recovery scan caller (idempotent under remap)
- `services/cost_advisor/src/runtime.rs:496-535` — rule's naive index-0 emission (unchanged)
- `services/cost_advisor/src/patch_validator.rs` — same-index pinning invariant (unchanged)
- `services/ledger/migrations/0044_*.sql` — SQL CHECK constraint enforcing same-index pinning at write-time (unchanged)
- `deploy/demo/init/bundles/generate.sh:64-118` — 2-budget contract source
- `deploy/demo/cost_advisor_demo.sh:286-340` — step 5 per-budget regression assertions
- `docs/specs/cost-advisor-spec.md` §6.1 — multi-budget UX limitation now CLOSED
- `docs/specs/cost-advisor-p3.7-sidecar-hot-reload.md` — preceding slice (CA-P3.7); hot-reload path stays unchanged
