//! Apply path: fetch approval row, load active bundle, apply patch,
//! write new bundle + runtime.env.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use tracing::{debug, info};
use uuid::Uuid;

use crate::bundle;
use crate::Config;

pub struct ApplyResult {
    pub new_bundle_hash: String,
}

pub async fn process_approval(
    pool: &PgPool,
    approval_id: Uuid,
    tenant_id: Uuid,
    config: &Config,
) -> Result<ApplyResult> {
    // 1. Fetch the approval row's patch + state.
    let row: (String, Option<Value>, Option<Uuid>) = sqlx::query_as(
        r#"
        SELECT state, proposed_dsl_patch, proposing_finding_id
          FROM approval_requests
         WHERE approval_id = $1 AND tenant_id = $2
        "#,
    )
    .bind(approval_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .context("SELECT approval_requests row")?;
    let (state, patch, finding_id) = row;

    if state != "approved" {
        // The NOTIFY can race with a subsequent state change; re-check.
        bail!("approval {} is not approved (state={})", approval_id, state);
    }
    let patch =
        patch.ok_or_else(|| anyhow!("approval {} has NULL proposed_dsl_patch", approval_id))?;
    debug!(
        approval_id = %approval_id,
        finding_id = ?finding_id,
        patch_op_count = patch.as_array().map(|a| a.len()).unwrap_or(0),
        "approval row fetched"
    );

    // 2. Load the current contract bundle from disk.
    let bundle_path = config
        .contract_bundle_dir
        .join(format!("{}.tgz", config.contract_bundle_id));
    let current_bundle = bundle::read_bundle(&bundle_path)
        .with_context(|| format!("read contract bundle {}", bundle_path.display()))?;

    debug!(
        bundle_path = %bundle_path.display(),
        contract_yaml_bytes = current_bundle.contract_yaml.len(),
        manifest_json_bytes = current_bundle.manifest_json.len(),
        "bundle loaded"
    );

    // 3. Apply the RFC-6902 patch to the contract YAML.
    //    YAML → JSON → patch → JSON → YAML round-trip. The patch
    //    `test` op in the first slot pins identity (CA-P3.1); the
    //    `replace` op then mutates the leaf.
    let new_contract_yaml = apply_patch_to_yaml(&current_bundle.contract_yaml, &patch)
        .context("apply RFC-6902 patch to contract YAML")?;

    // 4. Re-build the .tgz deterministically (same flags as
    //    deploy/demo/init/bundles/generate.sh: --sort=name --owner=0
    //    --group=0 --mtime='UTC 1970-01-01' so re-runs produce the
    //    same sha256 IFF the inputs are bit-identical).
    let new_bundle = bundle::Bundle {
        contract_yaml: new_contract_yaml,
        manifest_json: current_bundle.manifest_json.clone(),
    };
    let (new_tgz_bytes, new_hash) = bundle::pack_bundle(&new_bundle).context("re-pack bundle")?;
    info!(
        approval_id = %approval_id,
        old_hash = %current_bundle.sha256_hex,
        new_hash = %new_hash,
        "patched bundle ready"
    );

    if new_hash == current_bundle.sha256_hex {
        // Idempotent re-run: the patch produced bit-identical bytes.
        // Skip the disk write to avoid spurious manifest churn.
        info!(approval_id = %approval_id, "patch produced no-op (bundle bytes unchanged); skipping write");
        return Ok(ApplyResult {
            new_bundle_hash: new_hash,
        });
    }

    // 5. Atomic write order matters (codex CA-P3.5 r1 P2):
    //    sidecar startup verifies runtime.env hash == sha256(tgz on disk).
    //    Write order: (a) new .tgz, (b) new .sig, (c) runtime.env LAST.
    //    A crash before (c) leaves runtime.env still pointing at the OLD
    //    hash, while .tgz on disk is new — sidecar startup hash check
    //    would FAIL (fail-closed, refuses to come up). The operator
    //    sees the failure + can re-run bundle_registry recovery to
    //    finish the publish (idempotent — patch is replay-safe).
    //    Crash window: between (a) and (c) write/fsync calls.
    //    Real fix would be a versioned manifest pointer; deferred to
    //    P3.6+ scope.
    bundle::write_bundle_atomic(
        &bundle_path,
        &new_tgz_bytes,
        &config
            .contract_bundle_dir
            .join(format!("{}.tgz.sig", config.contract_bundle_id)),
    )
    .context("atomic write of bundle + signature")?;

    bundle::update_runtime_env(&config.runtime_env_path, &new_hash)
        .context("update runtime.env hash")?;

    Ok(ApplyResult {
        new_bundle_hash: new_hash,
    })
}

fn apply_patch_to_yaml(
    contract_yaml: &str,
    patch_json: &serde_json::Value,
) -> Result<String> {
    // Parse the YAML into a generic JSON value via serde_yaml so
    // RFC-6902 patches can walk the same tree (mapping ↔ object,
    // sequence ↔ array, scalar ↔ scalar).
    //
    // Codex CA-P3.5 r1 P2: this round-trip is NOT format-preserving.
    // Comments are stripped; map keys may reorder; quote styles may
    // change (double → single); sequence indentation may shift. The
    // sidecar parses the same logical contract because parse.rs
    // reads structural fields, but the bundle bytes will NOT be
    // bit-equivalent to a `generate.sh` re-derivation from source.
    // For v0.1 this is acceptable: the bundle hash is treated as an
    // opaque content-addressed ID by downstream consumers, not as a
    // reproducible identity. Operators rebuilding from source must
    // use bundle_registry's actual output bytes, not regenerate.
    let mut value: serde_json::Value =
        serde_yaml::from_str(contract_yaml).context("parse contract YAML")?;

    // CA-P3.8: transparently remap budget-array indices in the patch
    // before apply. cost_advisor emits patches naively at
    // /spec/budgets/0/* because it doesn't load contract bundles
    // (it operates on ledger reservations + audit events only). The
    // RFC-6902 test op pins the offending budget's UUID; we look up
    // which array index that UUID is actually at in the current
    // contract and rewrite all matching patch paths.
    //
    // This preserves the patch_validator's same-index test+replace
    // invariant POST-remap (both ops are rewritten consistently from
    // index `i` to index `j`), and falls through unchanged for the
    // single-budget v0.1 demo case (i == j == 0).
    //
    // Failure modes:
    //   * Test op pins a UUID not in the current contract → no remap
    //     entry produced for that index; json_patch::test will then
    //     fail naturally on apply (test value != contract[i].id),
    //     surfacing as "apply error" to the operator. Same UX as a
    //     genuinely stale finding against a deleted budget.
    //   * Multiple test ops at the same source index pinning
    //     different UUIDs → rejected before remap. Patch is structurally
    //     invalid and would fail the validator anyway.
    let remapped_patch = remap_budget_indices(patch_json, &value)
        .context("CA-P3.8: remap budget indices")?;

    let patch: json_patch::Patch =
        serde_json::from_value(remapped_patch).context("parse RFC-6902 patch")?;

    json_patch::patch(&mut value, &patch)
        .context("apply patch to contract YAML (likely test op identity mismatch)")?;

    let yaml = serde_yaml::to_string(&value).context("re-serialize contract YAML")?;
    Ok(yaml)
}

/// CA-P3.8: rewrite `/spec/budgets/<src>/...` paths to
/// `/spec/budgets/<dst>/...` where `dst` is the actual array index
/// of the budget whose `id` matches the patch's `test` op value at
/// `<src>`. Returns the rewritten patch (cloned, original
/// untouched). Falls through with no rewrite when `src == dst` (the
/// common v0.1 single-budget case).
///
/// Algorithm:
///   1. Walk the patch ops; collect `test` ops at
///      `/spec/budgets/<i>/id` into a `<i> → pinned_uuid` map.
///      Reject conflicting pins at the same `<i>` (structural bug
///      in upstream emitter).
///   2. For each pin, scan `contract.spec.budgets[]` looking for an
///      entry whose `id` equals the pinned UUID. Record `<i> → <j>`.
///      Entries with no match are left out — the apply will fail
///      naturally on the test op.
///   3. Walk the patch a second time; for any op whose path matches
///      `/spec/budgets/<i>/<rest>`, rewrite to
///      `/spec/budgets/<j>/<rest>` if `i` has a remap; otherwise
///      leave unchanged.
fn remap_budget_indices(
    patch_json: &serde_json::Value,
    contract: &serde_json::Value,
) -> Result<serde_json::Value> {
    use std::collections::BTreeMap;

    let ops = patch_json
        .as_array()
        .ok_or_else(|| anyhow!("patch is not a JSON array"))?;

    // Pass 1: collect test ops at /spec/budgets/<i>/id.
    let mut pinned: BTreeMap<u32, String> = BTreeMap::new();
    for op in ops {
        let op_kind = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
        if op_kind != "test" {
            continue;
        }
        let path = op.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let Some(src_idx) = parse_budget_id_path(path) else {
            continue;
        };
        let Some(uuid_value) = op.get("value").and_then(|v| v.as_str()) else {
            // Test op at /spec/budgets/<i>/id with non-string value
            // is malformed — let json_patch::test fail on apply.
            continue;
        };
        if let Some(existing) = pinned.get(&src_idx) {
            if existing != uuid_value {
                bail!(
                    "conflicting test pins at /spec/budgets/{}/id: {:?} vs {:?}",
                    src_idx,
                    existing,
                    uuid_value
                );
            }
            // Same uuid asserted twice — harmless, idempotent.
            continue;
        }
        pinned.insert(src_idx, uuid_value.to_string());
    }

    if pinned.is_empty() {
        return Ok(patch_json.clone());
    }

    // Pass 2: build src_idx → dst_idx remap by scanning contract.spec.budgets[].
    let budgets = contract
        .get("spec")
        .and_then(|s| s.get("budgets"))
        .and_then(|b| b.as_array());
    let mut remap: BTreeMap<u32, u32> = BTreeMap::new();
    if let Some(budgets) = budgets {
        for (src_idx, pinned_uuid) in &pinned {
            for (j, budget) in budgets.iter().enumerate() {
                let budget_id = budget.get("id").and_then(|v| v.as_str()).unwrap_or("");
                if budget_id == pinned_uuid {
                    let dst_idx = j as u32;
                    if dst_idx != *src_idx {
                        remap.insert(*src_idx, dst_idx);
                    }
                    break;
                }
            }
            // Pin with no match → no remap entry. json_patch::test
            // will then fail at apply (apply-time error), which is
            // the desired UX for "finding references a budget that
            // no longer exists in the contract".
        }
    }

    if remap.is_empty() {
        // Every pin already at the correct index, OR every pin
        // missing from contract (apply will fail naturally). No
        // rewrite needed.
        return Ok(patch_json.clone());
    }

    // Pass 3: rewrite all matching paths in subsequent ops.
    //
    // Codex CA-P3.8 r1 P3-1 future-proofing note: this loop rewrites
    // ONLY the `path` field. Today's allowlist (patch_validator.rs +
    // migration 0044) permits only `test` + `replace` ops, neither of
    // which has a `from` JSON Pointer. If a future slice relaxes the
    // allowlist to include `move` or `copy`, that slice MUST also
    // teach this loop to rewrite the `from` field — otherwise a
    // move/copy referencing `/spec/budgets/<src>/...` would dangle
    // post-remap. Tests `rejects_add_op` + `rejects_remove_op` in
    // patch_validator gate the current shape.
    let mut new_ops = Vec::with_capacity(ops.len());
    for op in ops {
        let mut new_op = op.clone();
        if let Some(path) = op.get("path").and_then(|v| v.as_str()) {
            if let Some((src_idx, rest)) = parse_budget_path_prefix(path) {
                if let Some(&dst_idx) = remap.get(&src_idx) {
                    let new_path = format!("/spec/budgets/{}{}", dst_idx, rest);
                    if let Some(map) = new_op.as_object_mut() {
                        map.insert(
                            "path".to_string(),
                            serde_json::Value::String(new_path),
                        );
                    }
                }
            }
        }
        new_ops.push(new_op);
    }

    Ok(serde_json::Value::Array(new_ops))
}

/// Parses paths of the exact form `/spec/budgets/<u32>/id` and
/// returns the index. Used by pass 1 to find `test` ops anchoring
/// budget identity.
fn parse_budget_id_path(path: &str) -> Option<u32> {
    let rest = path.strip_prefix("/spec/budgets/")?;
    let (idx_str, tail) = rest.split_once('/')?;
    if tail != "id" {
        return None;
    }
    // RFC 6901 array indices: 0 or [1-9][0-9]* (no leading zeros).
    // Mirrors patch_validator.rs::parse_budget_index — keep these in
    // sync if either is relaxed.
    if idx_str != "0" && (idx_str.starts_with('0') || !idx_str.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    idx_str.parse::<u32>().ok()
}

/// Parses paths starting with `/spec/budgets/<u32>/...` and returns
/// (index, suffix_starting_with_slash) so the rewriter can splice in
/// a new index. Used by pass 3 to rewrite both `test` and `replace`
/// ops consistently.
fn parse_budget_path_prefix(path: &str) -> Option<(u32, &str)> {
    let rest = path.strip_prefix("/spec/budgets/")?;
    // Find the next '/' after the index segment. If absent, no suffix
    // to rewrite (shouldn't happen for our allowlist, but be safe).
    let slash_off = rest.find('/')?;
    let idx_str = &rest[..slash_off];
    let suffix = &rest[slash_off..];
    if idx_str != "0" && (idx_str.starts_with('0') || !idx_str.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let idx = idx_str.parse::<u32>().ok()?;
    Some((idx, suffix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn applies_2op_test_replace_happy_path() {
        let contract_yaml = r#"apiVersion: contract.spendguard.io/v1alpha1
kind: Contract
metadata:
  id: 33333333-3333-4333-8333-333333333333
  name: demo
spec:
  budgets:
    - id: 44444444-4444-4444-8444-444444444444
      limit_amount_atomic: "1000000000"
      currency: USD
      reservation_ttl_seconds: 600
      require_hard_cap: true
  rules: []
"#;
        let patch = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":"44444444-4444-4444-8444-444444444444"},
            {"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":45},
        ]);
        let out = apply_patch_to_yaml(contract_yaml, &patch).expect("apply");
        // Re-parse to assert the value changed.
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        let ttl = v["spec"]["budgets"][0]["reservation_ttl_seconds"]
            .as_u64()
            .unwrap();
        assert_eq!(ttl, 45);
    }

    #[test]
    fn test_op_mismatch_aborts_patch() {
        let contract_yaml = r#"apiVersion: contract.spendguard.io/v1alpha1
kind: Contract
metadata: {id: x, name: y}
spec:
  budgets:
    - id: 44444444-4444-4444-8444-444444444444
      limit_amount_atomic: "1000000000"
      currency: USD
      reservation_ttl_seconds: 600
      require_hard_cap: true
"#;
        let patch = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":"99999999-9999-9999-9999-999999999999"},
            {"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":45},
        ]);
        let r = apply_patch_to_yaml(contract_yaml, &patch);
        assert!(r.is_err(), "test-op identity mismatch must fail the whole patch");
    }

    // -----------------------------------------------------------------
    // CA-P3.8 multi-budget remap tests
    // -----------------------------------------------------------------

    const TWO_BUDGET_CONTRACT: &str = r#"apiVersion: contract.spendguard.io/v1alpha1
kind: Contract
metadata:
  id: 33333333-3333-4333-8333-333333333333
  name: demo
spec:
  budgets:
    - id: 44444444-4444-4444-8444-444444444444
      limit_amount_atomic: "1000000000"
      currency: USD
      reservation_ttl_seconds: 600
      require_hard_cap: true
    - id: 55555555-5555-4555-8555-555555555555
      limit_amount_atomic: "2000000000"
      currency: USD
      reservation_ttl_seconds: 300
      require_hard_cap: false
  rules: []
"#;

    #[test]
    fn remaps_patch_targeting_second_budget() {
        // cost_advisor emits naive index 0 with test op pinning the
        // SECOND budget's UUID. bundle_registry must remap to index 1
        // so the patch lands on budget 55555555 (not budget 44444444).
        let patch = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":"55555555-5555-4555-8555-555555555555"},
            {"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":45},
        ]);
        let out = apply_patch_to_yaml(TWO_BUDGET_CONTRACT, &patch).expect("apply");
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        // budget #1 (index 1) gets the new TTL.
        assert_eq!(v["spec"]["budgets"][1]["reservation_ttl_seconds"].as_u64().unwrap(), 45);
        // budget #0 (index 0) is UNCHANGED — regression detector for
        // the "patch silently mutates wrong budget" bug class.
        assert_eq!(v["spec"]["budgets"][0]["reservation_ttl_seconds"].as_u64().unwrap(), 600);
    }

    #[test]
    fn single_budget_index_0_unchanged_no_remap() {
        // Existing v0.1 demo case: budget at index 0 pinned. No remap
        // entry produced (src == dst == 0). Backward compatibility.
        let single_yaml = r#"apiVersion: contract.spendguard.io/v1alpha1
kind: Contract
metadata: {id: x, name: y}
spec:
  budgets:
    - id: 44444444-4444-4444-8444-444444444444
      limit_amount_atomic: "1000000000"
      currency: USD
      reservation_ttl_seconds: 600
      require_hard_cap: true
  rules: []
"#;
        let patch = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":"44444444-4444-4444-8444-444444444444"},
            {"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":45},
        ]);
        let out = apply_patch_to_yaml(single_yaml, &patch).expect("apply");
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["spec"]["budgets"][0]["reservation_ttl_seconds"].as_u64().unwrap(), 45);
    }

    #[test]
    fn remap_preserves_first_budget_when_targeting_second() {
        // Stronger version of the regression detector: also touch the
        // limit_amount_atomic field, ensure budget #0 (44444444) is
        // bit-identical post-apply.
        let patch = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":"55555555-5555-4555-8555-555555555555"},
            {"op":"replace","path":"/spec/budgets/0/limit_amount_atomic","value":"3000000000"},
        ]);
        let out = apply_patch_to_yaml(TWO_BUDGET_CONTRACT, &patch).expect("apply");
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["spec"]["budgets"][0]["id"].as_str().unwrap(), "44444444-4444-4444-8444-444444444444");
        assert_eq!(v["spec"]["budgets"][0]["limit_amount_atomic"].as_str().unwrap(), "1000000000");
        assert_eq!(v["spec"]["budgets"][1]["id"].as_str().unwrap(), "55555555-5555-4555-8555-555555555555");
        assert_eq!(v["spec"]["budgets"][1]["limit_amount_atomic"].as_str().unwrap(), "3000000000");
    }

    #[test]
    fn test_op_with_uuid_absent_from_contract_falls_through_to_apply_failure() {
        // Pin a UUID that's not in the contract at all. No remap
        // entry produced; json_patch::test then fires its own check
        // against `contract.spec.budgets[0].id` (which is 44444444),
        // and that comparison fails. Operator sees the apply error
        // with the same UX as a stale-budget finding.
        let patch = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":"99999999-9999-4999-8999-999999999999"},
            {"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":45},
        ]);
        let r = apply_patch_to_yaml(TWO_BUDGET_CONTRACT, &patch);
        assert!(r.is_err(), "missing-budget pin must fail apply");
    }

    #[test]
    fn conflicting_pins_at_same_index_rejected_before_remap() {
        // Two test ops at the same /spec/budgets/0/id with different
        // UUIDs is structurally invalid (would also fail the patch
        // validator). Reject before json_patch::patch sees it for a
        // clearer error message.
        let patch = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":"44444444-4444-4444-8444-444444444444"},
            {"op":"test","path":"/spec/budgets/0/id","value":"55555555-5555-4555-8555-555555555555"},
            {"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":45},
        ]);
        let r = apply_patch_to_yaml(TWO_BUDGET_CONTRACT, &patch);
        assert!(r.is_err(), "conflicting same-index pins must be rejected");
        // Surface a clear error mentioning the conflict.
        let msg = format!("{:#}", r.unwrap_err());
        assert!(msg.contains("conflicting"), "error should mention 'conflicting'; got: {msg}");
    }

    #[test]
    fn idempotent_same_pin_at_same_index_is_no_op() {
        // Same UUID asserted twice at the same source index — common
        // if a future emitter is paranoid. Should not be flagged as
        // a conflict.
        let patch = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":"55555555-5555-4555-8555-555555555555"},
            {"op":"test","path":"/spec/budgets/0/id","value":"55555555-5555-4555-8555-555555555555"},
            {"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":45},
        ]);
        let out = apply_patch_to_yaml(TWO_BUDGET_CONTRACT, &patch).expect("apply");
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["spec"]["budgets"][1]["reservation_ttl_seconds"].as_u64().unwrap(), 45);
    }

    #[test]
    fn empty_patch_passes_through() {
        let patch = json!([]);
        // An empty patch on TWO_BUDGET_CONTRACT just re-serializes
        // the YAML through the JSON round-trip — successful no-op.
        let out = apply_patch_to_yaml(TWO_BUDGET_CONTRACT, &patch).expect("apply");
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["spec"]["budgets"].as_sequence().unwrap().len(), 2);
    }

    #[test]
    fn contract_without_budgets_array_does_not_panic_on_remap() {
        // Malformed contract (missing spec.budgets) — the remapper
        // should not panic; the apply itself will fail in json_patch.
        let yaml = "apiVersion: x\nkind: Contract\nspec:\n  rules: []\n";
        let patch = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":"55555555-5555-4555-8555-555555555555"},
            {"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":45},
        ]);
        let r = apply_patch_to_yaml(yaml, &patch);
        // Without budgets array, no remap; json_patch::test then
        // fails because /spec/budgets/0/id doesn't exist.
        assert!(r.is_err(), "test op against missing budgets array must fail apply");
    }

    #[test]
    fn parse_budget_id_path_accepts_valid_indices_only() {
        assert_eq!(parse_budget_id_path("/spec/budgets/0/id"), Some(0));
        assert_eq!(parse_budget_id_path("/spec/budgets/1/id"), Some(1));
        assert_eq!(parse_budget_id_path("/spec/budgets/42/id"), Some(42));
        // Leading zeros: RFC 6901 forbids.
        assert_eq!(parse_budget_id_path("/spec/budgets/01/id"), None);
        // Wrong leaf.
        assert_eq!(parse_budget_id_path("/spec/budgets/0/ttl"), None);
        // Wrong prefix.
        assert_eq!(parse_budget_id_path("/spec/rules/0/id"), None);
        // Non-numeric.
        assert_eq!(parse_budget_id_path("/spec/budgets/x/id"), None);
    }
}
