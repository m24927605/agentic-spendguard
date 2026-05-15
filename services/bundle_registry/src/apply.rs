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

    let patch: json_patch::Patch =
        serde_json::from_value(patch_json.clone()).context("parse RFC-6902 patch")?;

    json_patch::patch(&mut value, &patch)
        .context("apply patch to contract YAML (likely test op identity mismatch)")?;

    let yaml = serde_yaml::to_string(&value).context("re-serialize contract YAML")?;
    Ok(yaml)
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
}
