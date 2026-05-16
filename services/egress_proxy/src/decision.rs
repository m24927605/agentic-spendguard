//! DecisionRequest construction + ReservationContext + pricing cache.
//!
//! Slice 4b deliverable per spec §15 row 4b + §4.1 step 5 + §4.1.5.
//!
//! Key invariants:
//! - step_id derivation uses spendguard-ids (deterministic across proxy/SDK)
//! - PricingFreeze frozen at PRE; never re-read between PRE and POST
//! - arc_swap cache invalidates on runtime.env mtime advance
//! - DecisionRequest carries route="llm.call" + trigger=LLM_CALL_PRE
//! - All three SpendGuardIds present (run_id / step_id / llm_call_id /
//!   decision_id) per spec P2-r3.D fix

use anyhow::Context;
use arc_swap::ArcSwap;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::proto::common::v1::{
    self as common_pb, budget_claim::Direction, unit_ref::Kind as UnitKind,
};
use crate::proto::sidecar_adapter::v1::{
    decision_request::{Inputs, Trigger},
    DecisionRequest,
};

/// Reservation context retained per-request between PRE (decision)
/// and POST (commit). Spec §4.1.5 — FROZEN at construction; never
/// re-read until commit.
#[derive(Clone)]
pub struct ReservationContext {
    pub reservation_id: Uuid,
    pub decision_id: Uuid,
    pub effect_hash: Vec<u8>,
    pub unit: common_pb::UnitRef,
    pub pricing: common_pb::PricingFreeze,
    pub run_id: String,
    pub step_id: String,
    pub llm_call_id: String,
    pub audit_decision_event_id: Uuid,
}

/// Inputs derived from the HTTP request body + headers.
pub struct DecisionInputs<'a> {
    pub tenant_id: &'a str,
    pub budget_id: &'a str,
    pub window_instance_id: &'a str,
    pub run_id: String,           // from X-SpendGuard-Run-Id or fresh UUIDv7
    pub body_bytes: &'a [u8],
    pub model_family: String,     // parsed from request.model
    pub estimated_tokens: i64,    // heuristic or X-SpendGuard-Estimated-Tokens
    pub unit_id: &'a str,
}

/// Build a `DecisionRequest` for the sidecar.
///
/// Step IDs are derived via `spendguard-ids` so retries of the same
/// body collapse to the same step_id → same cost_advisor fingerprint
/// → finding accumulates. The unified `:call:` discriminator (NOT
/// `:proxy-call:`) per Staff escalation r5 verdict.
///
/// Note: DecisionRequest does NOT carry pricing or fencing — sidecar
/// loads pricing from its bundle. PricingFreeze becomes relevant at
/// the POST step (LlmCallPostPayload, slice 5).
pub fn build_decision_request(
    inputs: &DecisionInputs<'_>,
) -> anyhow::Result<DecisionRequest> {
    let body_json: Value = serde_json::from_slice(inputs.body_bytes)
        .context("body not valid JSON for signature derivation")?;
    let signature = spendguard_ids::default_call_signature_jcs(&body_json)
        .context("compute body signature")?;

    let step_id = format!("{}:call:{}", inputs.run_id, &signature[..16.min(signature.len())]);
    let llm_call_id = spendguard_ids::derive_uuid_from_signature(&signature, "llm_call_id");
    let decision_id = Uuid::now_v7();

    // Per-attempt idempotency key (default; spec §3.2 + §7).
    // Includes nanos so SDK auto-retry doesn't double-bill on OpenAI.
    let nanos = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_default();
    let idempotency_key = {
        let mut combined = signature.clone();
        combined.push('|');
        combined.push_str(&nanos);
        // Use blake2b for consistency with other signatures
        let bytes = blake2_helper(combined.as_bytes());
        hex::encode(&bytes[..8])
    };

    let unit = common_pb::UnitRef {
        unit_id: inputs.unit_id.to_string(),
        kind: UnitKind::Token as i32,
        currency: String::new(),
        token_kind: "output_token".to_string(),
        model_family: inputs.model_family.clone(),
        ..Default::default()
    };

    let claim = common_pb::BudgetClaim {
        budget_id: inputs.budget_id.to_string(),
        unit: Some(unit.clone()),
        amount_atomic: inputs.estimated_tokens.to_string(),
        direction: Direction::Debit as i32,
        window_instance_id: inputs.window_instance_id.to_string(),
        ..Default::default()
    };

    let inputs_msg = Inputs {
        projected_claims: vec![claim],
        projected_unit: Some(unit),
        ..Default::default()
    };

    let req = DecisionRequest {
        session_id: format!("egress-proxy:{}", inputs.run_id),
        trigger: Trigger::LlmCallPre as i32,
        route: "llm.call".to_string(),
        ids: Some(common_pb::SpendGuardIds {
            run_id: inputs.run_id.clone(),
            step_id,
            llm_call_id: llm_call_id.to_string(),
            decision_id: decision_id.to_string(),
            ..Default::default()
        }),
        idempotency: Some(common_pb::Idempotency {
            key: idempotency_key,
            request_hash: Vec::new().into(),
        }),
        inputs: Some(inputs_msg),
        ..Default::default()
    };
    Ok(req)
}

fn blake2_helper(data: &[u8]) -> Vec<u8> {
    use blake2::{digest::consts::U16, Blake2b, Digest};
    let mut h = Blake2b::<U16>::new();
    h.update(data);
    h.finalize().to_vec()
}

/// Pricing tuple snapshot, cached at process scope and refreshed on
/// `runtime.env` mtime advance. Spec §4.1.5.
#[derive(Clone, Debug)]
pub struct PricingSnapshot {
    pub pricing: common_pb::PricingFreeze,
    pub mtime: Option<SystemTime>,
}

#[derive(Clone)]
pub struct PricingCache {
    pub runtime_env_path: PathBuf,
    pub current: Arc<ArcSwap<PricingSnapshot>>,
}

impl PricingCache {
    /// Load initial snapshot from runtime.env + metadata.json.
    pub fn load(runtime_env_path: PathBuf) -> anyhow::Result<Self> {
        let snapshot = read_pricing_snapshot(&runtime_env_path)
            .with_context(|| {
                format!("initial pricing read from {}", runtime_env_path.display())
            })?;
        Ok(Self {
            runtime_env_path,
            current: Arc::new(ArcSwap::from_pointee(snapshot)),
        })
    }

    /// Get a fresh pricing tuple, refreshing the cache if the file
    /// mtime has advanced.
    pub fn get_fresh(&self) -> common_pb::PricingFreeze {
        let cached = self.current.load_full();
        if let Ok(meta) = std::fs::metadata(&self.runtime_env_path) {
            let on_disk_mtime = meta.modified().ok();
            if on_disk_mtime != cached.mtime {
                match read_pricing_snapshot(&self.runtime_env_path) {
                    Ok(fresh) => {
                        debug!(
                            old_mtime = ?cached.mtime,
                            new_mtime = ?fresh.mtime,
                            "pricing cache refreshed on mtime advance"
                        );
                        self.current.store(Arc::new(fresh.clone()));
                        return fresh.pricing;
                    }
                    Err(e) => {
                        warn!(err = %e, "pricing refresh failed; reusing cached snapshot");
                    }
                }
            }
        }
        cached.pricing.clone()
    }
}

/// Read pricing tuple from runtime.env (+ sibling metadata.json).
///
/// Reuses the same shape as the sidecar's `bundles/contract_bundle/<id>.metadata.json`.
/// For demo path, runtime.env carries SPENDGUARD_SIDECAR_PRICING_VERSION /
/// _SNAPSHOT_HASH_HEX / _FX_RATE_VERSION / _UNIT_CONVERSION_VERSION
/// (mirroring what bundles-init populates for the sidecar). If these
/// env vars are absent, fall back to demo defaults.
fn read_pricing_snapshot(path: &std::path::Path) -> anyhow::Result<PricingSnapshot> {
    let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());

    let contents = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;

    let mut map = std::collections::HashMap::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        let body = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        if let Some((k, v)) = body.split_once('=') {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            map.insert(k.trim().to_string(), v.to_string());
        }
    }

    let pricing_version = map
        .get("SPENDGUARD_SIDECAR_PRICING_VERSION")
        .cloned()
        .unwrap_or_else(|| "demo-pricing-v1".to_string());
    let price_snapshot_hash_hex = map
        .get("SPENDGUARD_SIDECAR_PRICE_SNAPSHOT_HASH_HEX")
        .or_else(|| map.get("SPENDGUARD_SCHEMA_BUNDLE_HASH_HEX"))
        .cloned()
        .unwrap_or_default();
    let fx_rate_version = map
        .get("SPENDGUARD_SIDECAR_FX_RATE_VERSION")
        .cloned()
        .unwrap_or_else(|| "demo-fx-v1".to_string());
    let unit_conversion_version = map
        .get("SPENDGUARD_SIDECAR_UNIT_CONVERSION_VERSION")
        .cloned()
        .unwrap_or_else(|| "demo-units-v1".to_string());

    let snapshot_hash_bytes = if price_snapshot_hash_hex.is_empty() {
        Vec::new()
    } else {
        hex::decode(&price_snapshot_hash_hex).with_context(|| {
            format!(
                "price_snapshot_hash_hex not valid hex: {}",
                price_snapshot_hash_hex
            )
        })?
    };

    Ok(PricingSnapshot {
        pricing: common_pb::PricingFreeze {
            pricing_version,
            price_snapshot_hash: snapshot_hash_bytes.into(),
            fx_rate_version,
            unit_conversion_version,
        },
        mtime,
    })
}

/// Heuristic token estimate from request body. Better than nothing.
/// Spec §5.1 priority: header override > heuristic > 1024 fallback.
pub fn estimate_tokens(body: &Value, header_override: Option<i64>) -> i64 {
    if let Some(v) = header_override {
        return v.max(1);
    }
    // Simple heuristic: ~4 chars per token, 2x for completion headroom.
    if let Some(messages) = body.get("messages").and_then(|v| v.as_array()) {
        let total_chars: usize = messages
            .iter()
            .filter_map(|m| m.get("content"))
            .filter_map(|c| c.as_str())
            .map(|s| s.len())
            .sum();
        let est = ((total_chars / 4) * 2).max(64) as i64;
        return est;
    }
    1024
}

pub fn parse_model_family(body: &Value) -> String {
    body.get("model")
        .and_then(|v| v.as_str())
        .map(|s| {
            // Strip versions: "gpt-4o-mini-2024-07-18" → "gpt-4o-mini"
            // For now keep verbatim — spec leaves this open.
            s.to_string()
        })
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture_inputs(body: &[u8]) -> DecisionInputs<'_> {
        DecisionInputs {
            tenant_id: "00000000-0000-4000-8000-000000000001",
            budget_id: "44444444-4444-4444-8444-444444444444",
            window_instance_id: "55555555-5555-4555-8555-555555555555",
            run_id: "11111111-1111-7111-8111-111111111111".to_string(),
            body_bytes: body,
            model_family: "gpt-4o-mini".to_string(),
            estimated_tokens: 500,
            unit_id: "66666666-6666-4666-8666-666666666666",
        }
    }

    #[test]
    fn build_decision_request_carries_required_ids() {
        let body = br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;
        let pricing = fixture_pricing();
        let inputs = fixture_inputs(body);
        let req = build_decision_request(&inputs).unwrap();
        let ids = req.ids.unwrap();
        assert_eq!(ids.run_id, inputs.run_id);
        assert!(ids.step_id.starts_with(&format!("{}:call:", inputs.run_id)));
        assert!(uuid::Uuid::parse_str(&ids.llm_call_id).is_ok());
        assert!(uuid::Uuid::parse_str(&ids.decision_id).is_ok());
        assert_eq!(req.route, "llm.call");
        assert_eq!(req.trigger, Trigger::LlmCallPre as i32);
    }

    #[test]
    fn step_id_is_deterministic_for_same_body_and_run() {
        let body = br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;
        let pricing = fixture_pricing();
        let inputs = fixture_inputs(body);
        let req1 = build_decision_request(&inputs).unwrap();
        let req2 = build_decision_request(&inputs).unwrap();
        // Same body + same run_id → same step_id (Staff #4 convergence).
        assert_eq!(req1.ids.as_ref().unwrap().step_id, req2.ids.as_ref().unwrap().step_id);
        // But decision_id is per-attempt UUIDv7 — different.
        assert_ne!(req1.ids.as_ref().unwrap().decision_id, req2.ids.as_ref().unwrap().decision_id);
        // Idempotency key per-attempt too (nanos diff).
        assert_ne!(req1.idempotency.as_ref().unwrap().key, req2.idempotency.as_ref().unwrap().key);
    }

    #[test]
    fn step_id_uses_unified_call_discriminator_not_proxy_call() {
        // Codex r5 Staff #4 ledger-audit verdict — verify the
        // discriminator is `:call:` so cost_advisor agent grouping
        // converges across proxy + wrapper-SDK deployments.
        let body = br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;
        let inputs = fixture_inputs(body);
        let req = build_decision_request(&inputs, &fixture_pricing()).unwrap();
        let step_id = req.ids.unwrap().step_id;
        assert!(step_id.contains(":call:"), "got: {step_id}");
        assert!(!step_id.contains(":proxy-call:"), "got: {step_id}");
    }

    #[test]
    fn estimate_tokens_header_override_wins() {
        let body = json!({"messages": [{"content": "x" .repeat(100)}]});
        assert_eq!(estimate_tokens(&body, Some(42)), 42);
        assert_eq!(estimate_tokens(&body, Some(0)), 1); // clamped to ≥1
    }

    #[test]
    fn estimate_tokens_heuristic() {
        let body = json!({"messages": [{"content": "hello world"}]});
        let est = estimate_tokens(&body, None);
        assert!(est >= 64, "got: {est}");
    }

    #[test]
    fn estimate_tokens_fallback() {
        let body = json!({});
        assert_eq!(estimate_tokens(&body, None), 1024);
    }

    #[test]
    fn parse_model_family_extracts_field() {
        let body = json!({"model": "gpt-4o-mini-2024-07-18"});
        assert_eq!(parse_model_family(&body), "gpt-4o-mini-2024-07-18");
    }

    #[test]
    fn parse_model_family_missing_returns_unknown() {
        let body = json!({});
        assert_eq!(parse_model_family(&body), "unknown");
    }
}
