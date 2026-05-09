//! Parse contract YAML out of the bundle tarball.
//!
//! Bundle layout (per `deploy/demo/init/bundles/generate.sh`):
//!   <bundle_id>.tgz contains:
//!     - manifest.json
//!     - contract.yaml   (NEW in Phase 3)
//!
//! Phase 2B shipped a `contract.cel` placeholder; Phase 3 generate.sh
//! replaces it with `contract.yaml`. The loader extracts contract.yaml
//! from the tarball bytes and serde-deserializes into `Contract`.
//!
//! Fail-closed: parse errors at startup → sidecar refuses to come up.
//! Silent fallback to "no rules → CONTINUE everything" would be a
//! compliance gap (no audit trail of what was supposed to gate the call).

use std::io::Read;

use anyhow::{anyhow, Context, Result};
use flate2::read::GzDecoder;
use serde::Deserialize;
use tar::Archive;
use uuid::Uuid;

use crate::contract::types::{Action, Budget, Condition, Contract, Rule};
use crate::proto::sidecar_adapter::v1::decision_response::Decision;

#[derive(Debug, Deserialize)]
struct YamlContract {
    #[serde(rename = "apiVersion")]
    api_version: String,
    kind: String,
    metadata: YamlMetadata,
    spec: YamlSpec,
}

#[derive(Debug, Deserialize)]
struct YamlMetadata {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct YamlSpec {
    #[serde(default)]
    budgets: Vec<YamlBudget>,
    #[serde(default)]
    rules: Vec<YamlRule>,
}

#[derive(Debug, Deserialize)]
struct YamlBudget {
    id: String,
    limit_amount_atomic: String,
    currency: String,
    #[serde(default = "default_ttl")]
    reservation_ttl_seconds: i64,
    #[serde(default)]
    require_hard_cap: bool,
}

fn default_ttl() -> i64 {
    600
}

#[derive(Debug, Deserialize)]
struct YamlRule {
    id: String,
    when: YamlCondition,
    then: YamlAction,
}

#[derive(Debug, Deserialize)]
struct YamlCondition {
    budget_id: String,
    #[serde(default)]
    claim_amount_atomic_gt: Option<String>,
    #[serde(default)]
    claim_amount_atomic_gte: Option<String>,
}

#[derive(Debug, Deserialize)]
struct YamlAction {
    decision: String,
    reason_code: String,
    #[serde(default)]
    approver_role: Option<String>,
}

const SUPPORTED_API_VERSION: &str = "contract.spendguard.io/v1alpha1";
const SUPPORTED_KIND: &str = "Contract";

/// Extract contract.yaml from a gzipped tarball and parse to `Contract`.
pub fn parse_from_tgz(bundle_bytes: &[u8]) -> Result<Contract> {
    let yaml_bytes = extract_contract_yaml(bundle_bytes)
        .context("extract contract.yaml from bundle tarball")?;
    parse_yaml(&yaml_bytes).context("parse contract.yaml")
}

fn extract_contract_yaml(tgz: &[u8]) -> Result<Vec<u8>> {
    let gz = GzDecoder::new(tgz);
    let mut ar = Archive::new(gz);
    for entry in ar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name == "contract.yaml" {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            return Ok(buf);
        }
    }
    Err(anyhow!(
        "contract.yaml not found in bundle tarball (Phase 3 wedge requires real contract YAML, not placeholder .cel)"
    ))
}

fn parse_yaml(bytes: &[u8]) -> Result<Contract> {
    let parsed: YamlContract = serde_yaml::from_slice(bytes)?;
    if parsed.api_version != SUPPORTED_API_VERSION {
        return Err(anyhow!(
            "unsupported apiVersion {}; expected {}",
            parsed.api_version,
            SUPPORTED_API_VERSION
        ));
    }
    if parsed.kind != SUPPORTED_KIND {
        return Err(anyhow!(
            "unsupported kind {}; expected {}",
            parsed.kind,
            SUPPORTED_KIND
        ));
    }

    let id = Uuid::parse_str(&parsed.metadata.id)
        .with_context(|| format!("metadata.id '{}' is not a UUID", parsed.metadata.id))?;

    let budgets = parsed
        .spec
        .budgets
        .into_iter()
        .map(|b| {
            Ok(Budget {
                id: Uuid::parse_str(&b.id)
                    .with_context(|| format!("budget.id '{}' is not a UUID", b.id))?,
                limit_amount_atomic: b.limit_amount_atomic,
                currency: b.currency,
                reservation_ttl_seconds: b.reservation_ttl_seconds,
                require_hard_cap: b.require_hard_cap,
            })
        })
        .collect::<Result<Vec<Budget>>>()?;

    let rules = parsed
        .spec
        .rules
        .into_iter()
        .map(|r| {
            let budget_id = Uuid::parse_str(&r.when.budget_id).with_context(|| {
                format!("rule '{}' when.budget_id '{}' is not a UUID", r.id, r.when.budget_id)
            })?;
            let decision = match r.then.decision.as_str() {
                "CONTINUE" => Decision::Continue,
                "DEGRADE" => Decision::Degrade,
                "SKIP" => Decision::Skip,
                "STOP" => Decision::Stop,
                "REQUIRE_APPROVAL" => Decision::RequireApproval,
                other => {
                    return Err(anyhow!(
                        "rule '{}' has unknown decision '{}'",
                        r.id,
                        other
                    ))
                }
            };
            Ok(Rule {
                id: r.id,
                when: Condition {
                    budget_id,
                    claim_amount_atomic_gt: r.when.claim_amount_atomic_gt,
                    claim_amount_atomic_gte: r.when.claim_amount_atomic_gte,
                },
                then: Action {
                    decision,
                    reason_code: r.then.reason_code,
                    approver_role: r.then.approver_role,
                },
            })
        })
        .collect::<Result<Vec<Rule>>>()?;

    Ok(Contract {
        id,
        name: parsed.metadata.name,
        budgets,
        rules,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_YAML: &str = r#"
apiVersion: contract.spendguard.io/v1alpha1
kind: Contract
metadata:
  id: 22222222-2222-4222-8222-222222222222
  name: demo-contract
spec:
  budgets:
    - id: 11111111-1111-4111-8111-111111111111
      limit_amount_atomic: "1000000000"
      currency: USD
      reservation_ttl_seconds: 600
      require_hard_cap: true
  rules:
    - id: hard-cap-deny
      when:
        budget_id: 11111111-1111-4111-8111-111111111111
        claim_amount_atomic_gt: "1000000000"
      then:
        decision: STOP
        reason_code: BUDGET_EXHAUSTED
"#;

    #[test]
    fn parses_sample_yaml() {
        let c = parse_yaml(SAMPLE_YAML.as_bytes()).expect("parse");
        assert_eq!(c.budgets.len(), 1);
        assert_eq!(c.rules.len(), 1);
        assert_eq!(c.rules[0].id, "hard-cap-deny");
        assert!(matches!(c.rules[0].then.decision, Decision::Stop));
    }

    #[test]
    fn rejects_unknown_api_version() {
        let yaml = SAMPLE_YAML.replace("v1alpha1", "v9");
        assert!(parse_yaml(yaml.as_bytes()).is_err());
    }
}
