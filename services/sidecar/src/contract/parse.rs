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
//!
//! ## SLICE_02 — v1alpha2 additive bump
//!
//! Per `docs/contract-dsl-spec-v1alpha2.md`:
//!   * accept `contract.spendguard.io/v1alpha1` AND `spendguard.ai/v1alpha2`
//!     (legacy `contract.spendguard.io/v1alpha1` is the demo bundle wire
//!     and is kept as a v1alpha1 alias for backward compat)
//!   * recognize top-level `prediction_policy` (default STRICT_CEILING)
//!   * recognize per-rule `run_projection_action` (default BLOCK_NEXT_CALL)
//!   * validate `(prediction_policy × run_projection_action)` allowed-pairs
//!     per §5.3 at load time; reject otherwise (refuse_to_load — sidecar
//!     emits `bundle_validation_failed` event upstream)
//!   * for v1alpha1 contracts, default-fill the new fields so the
//!     evaluator sees a fully-populated Contract regardless of source
//!     apiVersion (byte-identical audit per §6.4)

use std::io::Read;

use anyhow::{anyhow, Context, Result};
use flate2::read::GzDecoder;
use serde::Deserialize;
use tar::Archive;
use uuid::Uuid;

use crate::contract::types::{
    is_allowed_pair, Action, Budget, Condition, Contract, PredictionPolicy, Rule,
    RunProjectionAction,
};
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
    /// SLICE_02 v1alpha2 additive. Optional on the wire; default fill
    /// applies post-parse (per spec §6.4). When set on a v1alpha1
    /// contract this is treated as a forward-compat hint (still
    /// validated against allowed-pairs).
    #[serde(default)]
    prediction_policy: Option<String>,
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
    /// SLICE_02 v1alpha2 additive. Optional on the wire; default
    /// `BLOCK_NEXT_CALL` (per spec §5) applies if omitted.
    #[serde(default)]
    run_projection_action: Option<String>,
    /// SLICE_02 round-1 M3 (round-2 revised): capture the spec §6.3
    /// CEL `condition:` field so we can hard-error on v1alpha2
    /// contracts that use it BEFORE SLICE_09 wires the CEL evaluator.
    /// Silent-ignore on v1alpha2 would be a worse-than-default foot-gun
    /// (operators copying §6.3 examples would see no enforcement and
    /// assume the rule is active).
    ///
    /// Round-2 asymmetric handling: on **v1alpha1** contracts the
    /// `condition:` field is LEGACY (per v1alpha1 spec §18) and the
    /// wedge evaluator falls back to the declarative `when:` form —
    /// a `tracing::warn!` is emitted on parse but the contract loads
    /// successfully (consistent with M1's forward-compat-hint pattern).
    /// On **v1alpha2** contracts `condition:` is REJECTED — v1alpha2
    /// authors explicitly opt into the predictor-aware surface.
    #[serde(default)]
    condition: Option<String>,
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

/// Legacy demo bundle apiVersion (pre-spec wire). Kept as a v1alpha1
/// alias for backward compat — the locked `contract-dsl-spec-v1alpha1.md`
/// §3 names the apiVersion `spendguard.ai/v1alpha1`, but the existing
/// demo bundles ship with `contract.spendguard.io/v1alpha1`. SLICE_02
/// accepts BOTH so the 8+ demo-mode regression test stays green without
/// rewriting every bundle.
const LEGACY_API_VERSION_V1ALPHA1: &str = "contract.spendguard.io/v1alpha1";
/// Canonical v1alpha1 apiVersion (per `contract-dsl-spec-v1alpha1.md`).
const CANONICAL_API_VERSION_V1ALPHA1: &str = "spendguard.ai/v1alpha1";
/// v1alpha2 additive apiVersion (per `contract-dsl-spec-v1alpha2.md` §6.3).
const CANONICAL_API_VERSION_V1ALPHA2: &str = "spendguard.ai/v1alpha2";

const SUPPORTED_KIND: &str = "Contract";

/// apiVersion classification for default-fill / forward-compat behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApiVersion {
    V1alpha1,
    V1alpha2,
}

fn classify_api_version(s: &str) -> Option<ApiVersion> {
    match s {
        LEGACY_API_VERSION_V1ALPHA1 | CANONICAL_API_VERSION_V1ALPHA1 => Some(ApiVersion::V1alpha1),
        CANONICAL_API_VERSION_V1ALPHA2 => Some(ApiVersion::V1alpha2),
        _ => None,
    }
}

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

    // Classify the apiVersion. SLICE_02: accept v1alpha1 (legacy + canonical)
    // and v1alpha2. Unknown apiVersions still fail-closed.
    let api_version_kind = classify_api_version(&parsed.api_version).ok_or_else(|| {
        anyhow!(
            "unsupported apiVersion {}; expected one of [{}, {}, {}]",
            parsed.api_version,
            LEGACY_API_VERSION_V1ALPHA1,
            CANONICAL_API_VERSION_V1ALPHA1,
            CANONICAL_API_VERSION_V1ALPHA2
        )
    })?;

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

    // SLICE_02 §6.4: contract-level prediction_policy default-fill.
    //   * v1alpha1 contract → ALWAYS STRICT_CEILING regardless of any
    //     forward-compat hint on the wire (a v1alpha1 contract author
    //     does not know about prediction_policy semantics, so the
    //     conservative read is "default applies").
    //   * v1alpha2 contract → use the YAML value if present, otherwise
    //     default STRICT_CEILING (per spec §4 default).
    //
    // Implementation: for v1alpha1, ignore the YAML field. This makes
    // the v1alpha1 → v1alpha2 byte-identical regression test stable
    // even if a v1alpha1 author drops a stray `prediction_policy:
    // EMPIRICAL_RUN_CEILING` line in their YAML (the parse silently
    // discards it; calibration-report sees STRICT_CEILING).
    //
    // SLICE_02 round-1 M1: emit a tracing::warn! when a v1alpha1
    // contract specifies prediction_policy on the wire. Pre-round-1
    // the value was silently discarded; operators editing the YAML
    // to opt into EMPIRICAL_RUN_CEILING would see no behavior change
    // and could not tell whether the hint was honored. The warning
    // closes the observability gap without changing semantics (still
    // defaults to STRICT_CEILING under v1alpha1).
    if api_version_kind == ApiVersion::V1alpha1 {
        if let Some(hint) = parsed.spec.prediction_policy.as_deref() {
            tracing::warn!(
                api_version = %parsed.api_version,
                ignored_policy = %hint,
                contract_id = %parsed.metadata.id,
                "v1alpha1 contract specified prediction_policy hint but \
                 apiVersion is v1alpha1 — hint discarded; default \
                 STRICT_CEILING applies. Bump apiVersion to \
                 spendguard.ai/v1alpha2 to activate the declared policy."
            );
        }
    }
    let prediction_policy = match api_version_kind {
        ApiVersion::V1alpha1 => PredictionPolicy::default(),
        ApiVersion::V1alpha2 => match parsed.spec.prediction_policy.as_deref() {
            Some(s) => PredictionPolicy::from_str(s).ok_or_else(|| {
                anyhow!(
                    "spec.prediction_policy '{}' is not a known enum value (allowed: \
                     STRICT_CEILING, EMPIRICAL_RUN_CEILING, ADAPTIVE_CEILING, SHADOW_ONLY)",
                    s
                )
            })?,
            None => PredictionPolicy::default(),
        },
    };

    let rules = parsed
        .spec
        .rules
        .into_iter()
        .map(|r| {
            let budget_id = Uuid::parse_str(&r.when.budget_id).with_context(|| {
                format!(
                    "rule '{}' when.budget_id '{}' is not a UUID",
                    r.id, r.when.budget_id
                )
            })?;

            // SLICE_02 round-2 M (revises round-1 M3): asymmetric handling
            // of the CEL `condition:` field per spec §6.3 wiring boundary.
            //
            // v1alpha2 contracts → HARD-REJECT. Per spec §6.3 the v1alpha2
            // CEL evaluator is wired in SLICE_09, not SLICE_02 — until
            // then the only honored condition surface is the declarative
            // when.claim_amount_atomic_gt / when.claim_amount_atomic_gte
            // form. Silent-ignore would be the worst possible foot-gun
            // (operators copying §6.3 examples would see no enforcement
            // and assume the rule was active). v1alpha2 contract authors
            // explicitly opt into the predictor-aware surface so the
            // strict signal is appropriate.
            //
            // v1alpha1 contracts → WARN, do not reject. v1alpha1 spec §18
            // documents `condition: |` CEL form in rule bodies as part
            // of the quickstart contract. The §2 row-18 invariant
            // ("v1alpha1 quickstart 100% 正確") forbids breaking that
            // surface. The wedge evaluator falls back to the declarative
            // `when:` form, and a tracing::warn! emits a breadcrumb
            // consistent with M1's forward-compat-hint pattern at
            // parse.rs:247-258 above.
            if r.condition.is_some() {
                match api_version_kind {
                    ApiVersion::V1alpha2 => {
                        return Err(anyhow!(
                            "bundle_validation_failed: rule '{}' uses CEL `condition:` \
                             field; CEL conditions wired in SLICE_09 — use \
                             claim_amount_atomic_gt / claim_amount_atomic_gte under \
                             `when:` in SLICE_02. See contract-dsl-spec-v1alpha2.md \
                             §6.3 SLICE_02-vs-SLICE_09 wiring boundary.",
                            r.id
                        ));
                    }
                    ApiVersion::V1alpha1 => {
                        tracing::warn!(
                            api_version = %parsed.api_version,
                            rule_id = %r.id,
                            "v1alpha1 contract rule carries `condition:` field — CEL \
                             conditions are not honored by the wedge evaluator; rule \
                             will use declarative when:/then: form only. See \
                             docs/contract-dsl-spec-v1alpha2.md §6.3 SLICE_02-vs-SLICE_09 \
                             wiring boundary."
                        );
                    }
                }
            }

            // SLICE_02 round-1 M1: emit a tracing::warn! when a v1alpha1
            // contract rule specifies run_projection_action on the wire.
            // Same observability rationale as the contract-level hint
            // above — silent-discard hides the operator's mistake.
            if api_version_kind == ApiVersion::V1alpha1 {
                if let Some(hint) = r.run_projection_action.as_deref() {
                    tracing::warn!(
                        api_version = %parsed.api_version,
                        rule_id = %r.id,
                        ignored_action = %hint,
                        "v1alpha1 contract rule specified \
                         run_projection_action hint but apiVersion is \
                         v1alpha1 — hint discarded; default BLOCK_NEXT_CALL \
                         applies. Bump apiVersion to spendguard.ai/v1alpha2 \
                         to activate the declared per-rule action."
                    );
                }
            }

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

            // SLICE_02 §6.4: rule-level run_projection_action default-fill.
            //   * v1alpha1 rule → ALWAYS BLOCK_NEXT_CALL regardless of
            //     any forward-compat hint on the wire (same conservative
            //     read as prediction_policy above).
            //   * v1alpha2 rule → use the YAML value if present,
            //     otherwise default BLOCK_NEXT_CALL.
            let run_projection_action = match api_version_kind {
                ApiVersion::V1alpha1 => RunProjectionAction::default(),
                ApiVersion::V1alpha2 => match r.run_projection_action.as_deref() {
                    Some(s) => RunProjectionAction::from_str(s).ok_or_else(|| {
                        anyhow!(
                            "rule '{}' run_projection_action '{}' is not a known enum value \
                             (allowed: BLOCK_NEXT_CALL, REQUIRE_APPROVAL, ALERT_ONLY)",
                            r.id,
                            s
                        )
                    })?,
                    None => RunProjectionAction::default(),
                },
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
                run_projection_action,
            })
        })
        .collect::<Result<Vec<Rule>>>()?;

    // SLICE_02 §5.3 allowed-pairs validation at bundle load time.
    //
    // This runs AFTER default-fill so v1alpha1 contracts (which always
    // resolve to STRICT_CEILING + BLOCK_NEXT_CALL — the single STRICT
    // pair) always pass without operator action.
    //
    // For v1alpha2 contracts, every rule's (policy, action) pair must
    // be in the allowed set; the FIRST violation refuses_to_load. This
    // is the bundle_validation_failed path per spec §5.3 — the
    // caller (bootstrap::bundles) translates the anyhow error into
    // the audit event upstream.
    for rule in &rules {
        if !is_allowed_pair(prediction_policy, rule.run_projection_action) {
            return Err(anyhow!(
                "rule '{}' violates v1alpha2 §5.3 allowed-pairs table: \
                 prediction_policy={} disallows run_projection_action={} \
                 (allowed pairs: STRICT_CEILING+BLOCK_NEXT_CALL only; \
                 EMPIRICAL_RUN_CEILING/ADAPTIVE_CEILING accept all 3; \
                 SHADOW_ONLY+ALERT_ONLY only)",
                rule.id,
                prediction_policy.as_str(),
                rule.run_projection_action.as_str(),
            ));
        }
    }

    Ok(Contract {
        id,
        name: parsed.metadata.name,
        budgets,
        rules,
        prediction_policy,
        api_version: parsed.api_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_YAML_V1ALPHA1_LEGACY: &str = r#"
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

    const SAMPLE_YAML_V1ALPHA2: &str = r#"
apiVersion: spendguard.ai/v1alpha2
kind: Contract
metadata:
  id: 22222222-2222-4222-8222-222222222222
  name: demo-contract
spec:
  prediction_policy: EMPIRICAL_RUN_CEILING
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
      run_projection_action: REQUIRE_APPROVAL
"#;

    #[test]
    fn parses_legacy_v1alpha1_yaml() {
        let c = parse_yaml(SAMPLE_YAML_V1ALPHA1_LEGACY.as_bytes()).expect("parse");
        assert_eq!(c.budgets.len(), 1);
        assert_eq!(c.rules.len(), 1);
        assert_eq!(c.rules[0].id, "hard-cap-deny");
        assert!(matches!(c.rules[0].then.decision, Decision::Stop));
        // SLICE_02 §6.4 — v1alpha1 contract gets default fill.
        assert_eq!(c.prediction_policy, PredictionPolicy::StrictCeiling);
        assert_eq!(
            c.rules[0].run_projection_action,
            RunProjectionAction::BlockNextCall
        );
        assert_eq!(c.api_version, "contract.spendguard.io/v1alpha1");
    }

    #[test]
    fn parses_canonical_v1alpha1_yaml() {
        // Spec-canonical apiVersion (spendguard.ai/v1alpha1) classifies
        // as v1alpha1 + receives default fill.
        let yaml = SAMPLE_YAML_V1ALPHA1_LEGACY
            .replace("contract.spendguard.io/v1alpha1", "spendguard.ai/v1alpha1");
        let c = parse_yaml(yaml.as_bytes()).expect("parse");
        assert_eq!(c.prediction_policy, PredictionPolicy::StrictCeiling);
        assert_eq!(
            c.rules[0].run_projection_action,
            RunProjectionAction::BlockNextCall
        );
    }

    #[test]
    fn parses_v1alpha2_yaml() {
        let c = parse_yaml(SAMPLE_YAML_V1ALPHA2.as_bytes()).expect("parse");
        // v1alpha2 reads the YAML value.
        assert_eq!(c.prediction_policy, PredictionPolicy::EmpiricalRunCeiling);
        assert_eq!(
            c.rules[0].run_projection_action,
            RunProjectionAction::RequireApproval
        );
    }

    #[test]
    fn v1alpha1_ignores_forward_compat_hint() {
        // A v1alpha1 contract that drops a stray prediction_policy
        // line gets the field IGNORED (we default-fill to
        // STRICT_CEILING). This stabilises the byte-identical
        // regression for SLICE_02 acceptance §8.2.
        let yaml = r#"
apiVersion: contract.spendguard.io/v1alpha1
kind: Contract
metadata:
  id: 22222222-2222-4222-8222-222222222222
  name: demo-contract
spec:
  prediction_policy: EMPIRICAL_RUN_CEILING
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
      run_projection_action: ALERT_ONLY
"#;
        let c = parse_yaml(yaml.as_bytes()).expect("parse");
        // v1alpha1 ⇒ default-fill even when hints present.
        assert_eq!(c.prediction_policy, PredictionPolicy::StrictCeiling);
        assert_eq!(
            c.rules[0].run_projection_action,
            RunProjectionAction::BlockNextCall
        );
    }

    #[test]
    fn rejects_unknown_api_version() {
        let yaml = SAMPLE_YAML_V1ALPHA1_LEGACY.replace("v1alpha1", "v9");
        let err = parse_yaml(yaml.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("unsupported apiVersion"));
    }

    #[test]
    fn rejects_unknown_prediction_policy_on_v1alpha2() {
        let yaml = SAMPLE_YAML_V1ALPHA2
            .replace("prediction_policy: EMPIRICAL_RUN_CEILING", "prediction_policy: BANANA");
        let err = parse_yaml(yaml.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("prediction_policy"));
        assert!(err.to_string().contains("BANANA"));
    }

    #[test]
    fn rejects_unknown_run_projection_action_on_v1alpha2() {
        let yaml = SAMPLE_YAML_V1ALPHA2
            .replace("run_projection_action: REQUIRE_APPROVAL", "run_projection_action: BANANA");
        let err = parse_yaml(yaml.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("run_projection_action"));
        assert!(err.to_string().contains("BANANA"));
    }

    #[test]
    fn rejects_strict_ceiling_plus_alert_only() {
        // Spec §5.3 — STRICT_CEILING disallows ALERT_ONLY.
        // Test the precise allowed-pairs combination that drives
        // operator confusion most often.
        let yaml = SAMPLE_YAML_V1ALPHA2
            .replace("prediction_policy: EMPIRICAL_RUN_CEILING", "prediction_policy: STRICT_CEILING")
            .replace("run_projection_action: REQUIRE_APPROVAL", "run_projection_action: ALERT_ONLY");
        let err = parse_yaml(yaml.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("§5.3 allowed-pairs"));
        assert!(err.to_string().contains("STRICT_CEILING"));
        assert!(err.to_string().contains("ALERT_ONLY"));
    }

    #[test]
    fn rejects_strict_ceiling_plus_require_approval() {
        let yaml = SAMPLE_YAML_V1ALPHA2
            .replace("prediction_policy: EMPIRICAL_RUN_CEILING", "prediction_policy: STRICT_CEILING")
            .replace("run_projection_action: REQUIRE_APPROVAL", "run_projection_action: REQUIRE_APPROVAL");
        // STRICT_CEILING + REQUIRE_APPROVAL also invalid per §5.3.
        let err = parse_yaml(yaml.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("§5.3 allowed-pairs"));
    }

    #[test]
    fn rejects_shadow_only_plus_block_next_call() {
        // Spec §5.3 — SHADOW_ONLY disallows BLOCK_NEXT_CALL.
        let yaml = SAMPLE_YAML_V1ALPHA2
            .replace("prediction_policy: EMPIRICAL_RUN_CEILING", "prediction_policy: SHADOW_ONLY")
            .replace("run_projection_action: REQUIRE_APPROVAL", "run_projection_action: BLOCK_NEXT_CALL");
        let err = parse_yaml(yaml.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("§5.3 allowed-pairs"));
        assert!(err.to_string().contains("SHADOW_ONLY"));
    }

    #[test]
    fn accepts_strict_ceiling_plus_block_next_call() {
        // The ONLY allowed STRICT_CEILING pair.
        let yaml = SAMPLE_YAML_V1ALPHA2
            .replace("prediction_policy: EMPIRICAL_RUN_CEILING", "prediction_policy: STRICT_CEILING")
            .replace("run_projection_action: REQUIRE_APPROVAL", "run_projection_action: BLOCK_NEXT_CALL");
        let c = parse_yaml(yaml.as_bytes()).expect("parse");
        assert_eq!(c.prediction_policy, PredictionPolicy::StrictCeiling);
        assert_eq!(
            c.rules[0].run_projection_action,
            RunProjectionAction::BlockNextCall
        );
    }

    #[test]
    fn accepts_shadow_only_plus_alert_only() {
        // The ONLY allowed SHADOW_ONLY pair.
        let yaml = SAMPLE_YAML_V1ALPHA2
            .replace("prediction_policy: EMPIRICAL_RUN_CEILING", "prediction_policy: SHADOW_ONLY")
            .replace("run_projection_action: REQUIRE_APPROVAL", "run_projection_action: ALERT_ONLY");
        let c = parse_yaml(yaml.as_bytes()).expect("parse");
        assert_eq!(c.prediction_policy, PredictionPolicy::ShadowOnly);
        assert_eq!(
            c.rules[0].run_projection_action,
            RunProjectionAction::AlertOnly
        );
    }

    #[test]
    fn rejects_v1alpha2_contract_with_cel_condition_field() {
        // SLICE_02 round-1 M3: v1alpha2 contract authors writing
        // `condition: <cel-expr>` per spec §6.3 BEFORE SLICE_09 lands
        // get a hard error; the silent-ignore foot-gun is closed.
        let yaml = r#"
apiVersion: spendguard.ai/v1alpha2
kind: Contract
metadata:
  id: 22222222-2222-4222-8222-222222222222
  name: demo-contract
spec:
  prediction_policy: EMPIRICAL_RUN_CEILING
  budgets:
    - id: 11111111-1111-4111-8111-111111111111
      limit_amount_atomic: "1000000000"
      currency: USD
      reservation_ttl_seconds: 600
      require_hard_cap: true
  rules:
    - id: stop_when_projection_exceeded
      when:
        budget_id: 11111111-1111-4111-8111-111111111111
      condition: |
        run_projection.at_decision_micros > budget("daily_usd").remaining.amountMicros
      then:
        decision: STOP
        reason_code: RUN_BUDGET_PROJECTION_EXCEEDED
      run_projection_action: BLOCK_NEXT_CALL
"#;
        let err = parse_yaml(yaml.as_bytes()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bundle_validation_failed"),
            "expected bundle_validation_failed marker, got: {msg}");
        assert!(msg.contains("CEL"),
            "expected CEL mention, got: {msg}");
        assert!(msg.contains("SLICE_09"),
            "expected SLICE_09 mention, got: {msg}");
        assert!(msg.contains("claim_amount_atomic_gt"),
            "expected pointer to SLICE_02 condition surface, got: {msg}");
    }

    #[test]
    fn v1alpha1_contract_with_cel_condition_field_parses_with_warn() {
        // SLICE_02 round-2 M (revises round-1 M3): v1alpha1 contracts
        // carrying a `condition:` field MUST parse successfully — the
        // v1alpha1 spec §18 quickstart documents the CEL `condition: |`
        // form in rule bodies and the §2 row-18 invariant requires that
        // any v1alpha1 quickstart remains 100% correct under v1alpha2
        // evaluator. The wedge evaluator falls back to the declarative
        // `when:` form; a tracing::warn! emits the breadcrumb but does
        // not abort parse (consistent with M1's forward-compat pattern).
        //
        // Note: this test asserts successful parse only. The tracing
        // crate's default subscriber is no-op in tests, so we don't
        // attempt to capture the warn line here — the parse-success
        // assertion is the load-bearing guarantee for §2 row-18.
        let yaml = r#"
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
    - id: stray-cel
      when:
        budget_id: 11111111-1111-4111-8111-111111111111
      condition: "1 == 1"
      then:
        decision: STOP
        reason_code: BUDGET_EXHAUSTED
"#;
        let c = parse_yaml(yaml.as_bytes())
            .expect("v1alpha1 contract with `condition:` MUST parse (§2 row 18 invariant)");
        assert_eq!(c.api_version, "contract.spendguard.io/v1alpha1");
        assert_eq!(c.rules.len(), 1);
        assert_eq!(c.rules[0].id, "stray-cel");
    }

    #[test]
    fn quickstart_v1alpha2_sample_still_parses_after_round1_m2_disable() {
        // SLICE_02 round-1 m2: the `require_approval_on_projected_overrun`
        // rule in examples/contracts/quickstart-v1alpha2.yaml is now
        // commented out (would otherwise flag every non-zero call for
        // approval before SLICE_09 wires the projector). This test
        // pins the post-m2 quickstart to ensure:
        //   1. The remaining structure still parses cleanly.
        //   2. Only the `stop_when_exhausted` rule is active.
        //   3. The contract-level prediction_policy is preserved.
        let yaml = include_bytes!(
            "../../../../examples/contracts/quickstart-v1alpha2.yaml"
        );
        let c = parse_yaml(yaml).expect("quickstart-v1alpha2.yaml parse");
        assert_eq!(c.api_version, "spendguard.ai/v1alpha2");
        assert_eq!(c.prediction_policy, PredictionPolicy::EmpiricalRunCeiling);
        assert_eq!(c.rules.len(), 1, "only stop_when_exhausted should be active");
        assert_eq!(c.rules[0].id, "stop_when_exhausted");
        // The active rule has no RUN_* code, so the default
        // BLOCK_NEXT_CALL applies and is allowed under
        // EMPIRICAL_RUN_CEILING.
        assert_eq!(
            c.rules[0].run_projection_action,
            RunProjectionAction::BlockNextCall
        );
    }

    #[test]
    fn v1alpha2_without_condition_field_still_parses() {
        // Regression: the M3 reject path must NOT fire when `condition:`
        // is absent (which is the only legal SLICE_02 form). Re-uses
        // the existing SAMPLE_YAML_V1alpha2 fixture.
        let c = parse_yaml(SAMPLE_YAML_V1ALPHA2.as_bytes()).expect("parse");
        assert_eq!(c.rules.len(), 1);
    }

    #[test]
    fn property_accepts_all_8_allowed_pairs() {
        // Cross-check: every (policy, action) pair that
        // `is_allowed_pair` says should pass MUST be load-accepted.
        let policies = [
            ("STRICT_CEILING", PredictionPolicy::StrictCeiling),
            (
                "EMPIRICAL_RUN_CEILING",
                PredictionPolicy::EmpiricalRunCeiling,
            ),
            ("ADAPTIVE_CEILING", PredictionPolicy::AdaptiveCeiling),
            ("SHADOW_ONLY", PredictionPolicy::ShadowOnly),
        ];
        let actions = [
            ("BLOCK_NEXT_CALL", RunProjectionAction::BlockNextCall),
            ("REQUIRE_APPROVAL", RunProjectionAction::RequireApproval),
            ("ALERT_ONLY", RunProjectionAction::AlertOnly),
        ];
        let mut accepted = 0;
        let mut rejected = 0;
        for (p_str, p_enum) in policies {
            for (a_str, a_enum) in actions {
                let yaml = SAMPLE_YAML_V1ALPHA2
                    .replace("prediction_policy: EMPIRICAL_RUN_CEILING", &format!("prediction_policy: {}", p_str))
                    .replace("run_projection_action: REQUIRE_APPROVAL", &format!("run_projection_action: {}", a_str));
                let res = parse_yaml(yaml.as_bytes());
                if is_allowed_pair(p_enum, a_enum) {
                    let c = res.unwrap_or_else(|e| panic!("allowed pair {p_str}+{a_str} rejected: {e}"));
                    assert_eq!(c.prediction_policy, p_enum);
                    assert_eq!(c.rules[0].run_projection_action, a_enum);
                    accepted += 1;
                } else {
                    res.unwrap_err();
                    rejected += 1;
                }
            }
        }
        assert_eq!(accepted, 8);
        assert_eq!(rejected, 4);
    }
}
