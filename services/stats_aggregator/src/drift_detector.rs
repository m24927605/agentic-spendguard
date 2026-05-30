//! Drift detection + signed CloudEvent emission per spec
//! stats-aggregator-spec-v1alpha1.md §7.
//!
//! ## Algorithm (spec §7.1)
//!
//! For each (tenant, model, agent_id, prompt_class) bucket whose 7d AND
//! 30d windows both have data:
//!
//! ```text
//! z_score = (mean_7d - mean_30d) / stddev_30d
//!
//! IF |z_score| > drift_z_threshold (default 2.0)
//!    AND sample_size_7d >= MIN_SAMPLES_FOR_ALERT (default 100):
//!   emit prediction_drift_alert CloudEvent
//! ```
//!
//! The threshold is configurable per spec §0.3 GA prereq #1 ("per-tenant
//! override"); SLICE_06 ships the global config; per-tenant override is
//! SLICE-extra.
//!
//! ## Audit-routed CloudEvent
//!
//! Per SLICE_05 R2 B2 + tokenizer worker convention (worker.rs:72-81),
//! drift_alert events use the `spendguard.audit.*` prefix to route into
//! canonical_ingest's ImmutableAuditLog. The fallthrough
//! `spendguard.prediction.*` (used in early drafts of the spec §7.2
//! example YAML) would route to ProfilePayloadBlob which is
//! RTBF-deletable and violates the spec §0.1 immutability claim.

use anyhow::Context;
use chrono::Utc;
use prost::Message;
use spendguard_signing::Signer;
use tonic::transport::Channel;
use tracing::{debug, info, warn};

use crate::aggregation::BucketAggregate;
use crate::proto::canonical_ingest::v1::{
    append_events_request::Route, canonical_ingest_client::CanonicalIngestClient,
    AppendEventsRequest,
};
use crate::proto::common::v1::{CloudEvent, SchemaBundleRef};

/// CloudEvent type for prediction drift alerts. Per spec §7.2 family
/// + SLICE_05 R2 B2 audit-routed prefix discipline.
pub const PREDICTION_DRIFT_ALERT_EVENT_TYPE: &str =
    "spendguard.audit.prediction_drift_alert.v1alpha1";

/// Configuration shared from main.rs into the drift detector. Pulled out
/// of the daemon config so the detector module is unit-testable without
/// the full envy + sqlx + tonic stack.
#[derive(Debug, Clone)]
pub struct DriftDetectorConfig {
    pub drift_z_threshold: f32,
    pub min_samples_for_alert: i32,
}

/// Per spec §7.3 suggested action heuristic.
fn suggest_action(z_score: f32) -> &'static str {
    if z_score > 0.0 {
        "investigate_agent_change"
    } else {
        "review_predictor_baseline"
    }
}

/// Compute z-score per spec §7.1.
///
/// R2 M2 (Software F5): baseline EXCLUDES the current 7-day window.
/// R1 shape used mean_30d (which includes the last 7d) → baseline was
/// contaminated by the very window we were comparing against, biasing
/// z toward 0. Now we read baseline_mean / baseline_stddev which were
/// computed over [now - 30d, now - 7d] (see aggregation.rs::agg_baseline).
///
/// Returns None when:
///   * 7d window is missing (no current sample to compare)
///   * baseline window is missing (new bucket; first activity in 7d)
///   * baseline_stddev == 0 (all baseline samples identical → undefined ratio)
pub fn compute_z_score(agg: &BucketAggregate) -> Option<f32> {
    let mean_7d = agg.mean_7d?;
    let baseline_mean = agg.baseline_mean?;
    let baseline_stddev = agg.baseline_stddev?;
    if baseline_stddev <= 0.0 {
        // baseline_stddev == 0 → all baseline samples identical.
        // Strict spec interpretation: z-score is undefined → no alert.
        return None;
    }
    Some((mean_7d - baseline_mean) / baseline_stddev)
}

/// Decision predicate: should this bucket emit a drift alert?
pub fn should_emit_drift_alert(agg: &BucketAggregate, cfg: &DriftDetectorConfig) -> bool {
    let Some(z) = compute_z_score(agg) else {
        return false;
    };
    let Some(n7d) = agg.sample_size_7d else {
        return false;
    };
    z.abs() > cfg.drift_z_threshold && n7d >= cfg.min_samples_for_alert
}

/// Build (but do not sign or emit) the prediction_drift_alert
/// CloudEvent for a bucket. Separated from the emission step so unit
/// tests can verify the envelope shape without a tonic Channel.
pub fn build_drift_alert(agg: &BucketAggregate, z_score: f32, cfg: &DriftDetectorConfig) -> CloudEvent {
    use bytes::Bytes;
    let now = Utc::now();
    // R2 M2: payload reports baseline_* fields (window [now-30d, now-7d]
    // — distinct from mean_30d which is the strategy-B cache baseline).
    // Calibration-report consumers want both for trend analysis.
    let data = serde_json::json!({
        "tenant_id": agg.tenant_id.to_string(),
        "model": agg.model,
        "agent_id": agg.agent_id,
        "prompt_class": agg.prompt_class,
        "baseline_mean": agg.baseline_mean,
        "baseline_stddev": agg.baseline_stddev,
        "baseline_sample_size": agg.baseline_sample_size,
        "current_mean_7d": agg.mean_7d,
        "z_score": z_score,
        "sample_size_7d": agg.sample_size_7d,
        "sample_size_30d": agg.sample_size_30d,
        "drift_z_threshold": cfg.drift_z_threshold,
        "min_samples_for_alert": cfg.min_samples_for_alert,
        "suggested_action": suggest_action(z_score),
    });
    let data_bytes = serde_json::to_vec(&data).unwrap_or_default();

    CloudEvent {
        specversion: "1.0".to_string(),
        r#type: PREDICTION_DRIFT_ALERT_EVENT_TYPE.to_string(),
        source: format!("spendguard://stats-aggregator/{}", agg.tenant_id),
        id: uuid::Uuid::now_v7().to_string(),
        time: Some(prost_types::Timestamp {
            seconds: now.timestamp(),
            nanos: now.timestamp_subsec_nanos() as i32,
        }),
        datacontenttype: "application/json".to_string(),
        data: data_bytes.into(),
        tenant_id: agg.tenant_id.to_string(),
        producer_signature: Bytes::new(),
        ..Default::default()
    }
}

/// Sink trait — abstracts the actual canonical_ingest gRPC call so
/// tests can swap in an in-memory recorder.
#[async_trait::async_trait]
pub trait DriftAlertSink: Send + Sync {
    async fn emit(&self, event: CloudEvent) -> Result<(), anyhow::Error>;
}

/// Logging-only sink for demo / development. Logs the event at INFO
/// level instead of sending to canonical_ingest. Production Helm gate
/// rejects this path when chart.profile=production.
pub struct LoggingDriftAlertSink;

#[async_trait::async_trait]
impl DriftAlertSink for LoggingDriftAlertSink {
    async fn emit(&self, event: CloudEvent) -> Result<(), anyhow::Error> {
        info!(
            event_type = %event.r#type,
            event_id = %event.id,
            tenant_id = %event.tenant_id,
            "drift_alert emitted (log sink — demo only)"
        );
        Ok(())
    }
}

/// canonical_ingest sink emitting one AppendEventsRequest per alert.
///
/// R2 B5: AppendEventsRequest envelope MUST carry producer_id +
/// schema_bundle + route or canonical_ingest's append handler rejects
/// the call with `producer_id required` / `schema_bundle required` /
/// `route is unspecified` (see
/// services/canonical_ingest/src/handlers/append_events.rs:64,73,101).
/// Defaults from R1 shape (`..Default::default()`) failed all three.
pub struct CanonicalIngestDriftAlertSink {
    client: tokio::sync::Mutex<CanonicalIngestClient<Channel>>,
    producer_id: String,
    schema_bundle_ref: SchemaBundleRef,
    signing_key_id: String,
}

impl CanonicalIngestDriftAlertSink {
    /// Build the sink with the envelope fields the canonical_ingest
    /// handler requires.
    ///
    /// * `producer_id` — typically `stats-aggregator:<region>`; matches
    ///   the `signer.producer_identity()` discipline used by tokenizer
    ///   SLICE_05.
    /// * `schema_bundle_ref` — pre-built SchemaBundleRef (id + hash +
    ///   canonical_schema_version). Built in main.rs from config + the
    ///   hex-decoded hash bytes (mirror of outbox_forwarder).
    /// * `signing_key_id` — the AppendEventsRequest batch-level
    ///   signing_key_id (transport check only; per-event signature is on
    ///   each CloudEvent). Reuse signer.key_id() for consistency.
    pub fn new(
        channel: Channel,
        producer_id: String,
        schema_bundle_ref: SchemaBundleRef,
        signing_key_id: String,
    ) -> Self {
        Self {
            client: tokio::sync::Mutex::new(CanonicalIngestClient::new(channel)),
            producer_id,
            schema_bundle_ref,
            signing_key_id,
        }
    }
}

#[async_trait::async_trait]
impl DriftAlertSink for CanonicalIngestDriftAlertSink {
    async fn emit(&self, event: CloudEvent) -> Result<(), anyhow::Error> {
        let req = AppendEventsRequest {
            producer_id: self.producer_id.clone(),
            batch_max_producer_sequence: 0,
            // batch_signature optional per Trace §13 — per-event
            // signature on each CloudEvent is the canonical truth; we
            // leave batch_signature empty (allowed in-cluster on mTLS).
            batch_signature: bytes::Bytes::new(),
            signing_key_id: self.signing_key_id.clone(),
            schema_bundle: Some(self.schema_bundle_ref.clone()),
            events: vec![event],
            // OBSERVABILITY: drift_alert is an observability event, not
            // an enforcement event. canonical_ingest's failure mode for
            // OBSERVABILITY is buffer_then_retry (per Trace §10.1).
            route: Route::Observability as i32,
        };
        let mut guard = self.client.lock().await;
        guard
            .append_events(req)
            .await
            .context("canonical_ingest AppendEvents")?;
        Ok(())
    }
}

/// Detect drift across a batch of bucket aggregates and emit signed
/// CloudEvents for those exceeding the threshold. Returns the count of
/// alerts emitted for metrics.
pub async fn detect_and_emit(
    aggregates: &[BucketAggregate],
    cfg: &DriftDetectorConfig,
    signer: &dyn Signer,
    sink: &dyn DriftAlertSink,
) -> Result<usize, anyhow::Error> {
    let mut emitted = 0;
    for agg in aggregates {
        if !should_emit_drift_alert(agg, cfg) {
            continue;
        }
        let z = match compute_z_score(agg) {
            Some(z) => z,
            None => continue,
        };
        let mut ce = build_drift_alert(agg, z, cfg);
        // Sign the canonical bytes (matches tokenizer worker.rs
        // sign_in_place pattern).
        ce.signing_key_id = signer.key_id().to_string();
        ce.producer_id = signer.producer_identity().to_string();
        ce.producer_signature = bytes::Bytes::new();
        let canonical = ce.encode_to_vec();
        let sig = signer
            .sign(&canonical)
            .await
            .map_err(|e| anyhow::anyhow!("sign drift_alert CloudEvent: {e}"))?;
        ce.producer_signature = sig.bytes.into();

        match sink.emit(ce).await {
            Ok(()) => {
                emitted += 1;
                debug!(
                    tenant_id = %agg.tenant_id,
                    model = %agg.model,
                    agent_id = %agg.agent_id,
                    prompt_class = %agg.prompt_class,
                    z_score = z,
                    "drift alert emitted"
                );
            }
            Err(e) => {
                warn!(
                    tenant_id = %agg.tenant_id,
                    error = %e,
                    "drift alert emit failed; will retry next cycle"
                );
            }
        }
    }
    Ok(emitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn fixture_cfg() -> DriftDetectorConfig {
        DriftDetectorConfig {
            drift_z_threshold: 2.0,
            min_samples_for_alert: 100,
        }
    }

    fn fixture_agg() -> BucketAggregate {
        // R2 M2: baseline_* fields drive z-score. Configure so
        // existing tests' z-values stay numerically identical:
        // current = 150 (mean_7d), baseline = 100, baseline_stddev = 20
        // → z = (150 - 100) / 20 = 2.5
        BucketAggregate {
            tenant_id: Uuid::new_v4(),
            model: "gpt-4o".into(),
            agent_id: "agent-a".into(),
            prompt_class: "chat_short".into(),
            mean_7d: Some(150.0),
            stddev_7d: Some(20.0),
            sample_size_7d: Some(200),
            mean_30d: Some(100.0),
            stddev_30d: Some(20.0),
            sample_size_30d: Some(800),
            baseline_mean: Some(100.0),
            baseline_stddev: Some(20.0),
            baseline_sample_size: Some(600),
        }
    }

    #[test]
    fn z_score_normal_path() {
        let agg = fixture_agg();
        // (150 - 100) / 20 = 2.5
        let z = compute_z_score(&agg).expect("z_score");
        assert!((z - 2.5).abs() < 1e-4);
    }

    #[test]
    fn z_score_none_when_missing_baseline_window() {
        // R2 M2: baseline (NOT mean_30d) drives the z calculation.
        let mut agg = fixture_agg();
        agg.baseline_mean = None;
        assert!(compute_z_score(&agg).is_none());
    }

    #[test]
    fn z_score_none_when_baseline_stddev_zero() {
        // R2 M2: baseline_stddev (NOT stddev_30d) is the denominator.
        let mut agg = fixture_agg();
        agg.baseline_stddev = Some(0.0);
        assert!(compute_z_score(&agg).is_none());
    }

    #[test]
    fn z_score_uses_baseline_excluding_current_window() {
        // R2 M2 regression: mean_30d INCLUDES the current 7d, baseline
        // EXCLUDES it. Verify drift_detector reads from baseline_*
        // (the excluded form), not mean_30d. Construct a case where
        // the two differ + assert the computed z reflects baseline_*.
        let mut agg = fixture_agg();
        agg.mean_30d = Some(140.0); // inclusive mean — close to mean_7d
        agg.stddev_30d = Some(100.0); // inclusive stddev — would damp z
        agg.baseline_mean = Some(100.0); // exclusive baseline
        agg.baseline_stddev = Some(20.0); // exclusive baseline
        // z must be (150 - 100) / 20 = 2.5, NOT (150 - 140) / 100 = 0.1.
        let z = compute_z_score(&agg).expect("z");
        assert!((z - 2.5).abs() < 1e-4, "z must use baseline_*, got {z}");
    }

    #[test]
    fn should_emit_true_above_threshold() {
        let agg = fixture_agg();
        // z = 2.5 > 2.0; n7d = 200 >= 100.
        assert!(should_emit_drift_alert(&agg, &fixture_cfg()));
    }

    #[test]
    fn should_emit_false_below_threshold() {
        let mut agg = fixture_agg();
        // Reduce mean_7d so z < 2.0.
        agg.mean_7d = Some(110.0); // z = 0.5
        assert!(!should_emit_drift_alert(&agg, &fixture_cfg()));
    }

    #[test]
    fn should_emit_false_below_min_samples_for_alert() {
        let mut agg = fixture_agg();
        agg.sample_size_7d = Some(50); // < 100
        assert!(!should_emit_drift_alert(&agg, &fixture_cfg()));
    }

    #[test]
    fn should_emit_true_with_negative_z_above_threshold() {
        let mut agg = fixture_agg();
        // mean_7d collapsing below baseline (vendor tokenizer drift case).
        agg.mean_7d = Some(50.0); // z = -2.5
        assert!(should_emit_drift_alert(&agg, &fixture_cfg()));
    }

    #[test]
    fn suggest_action_positive_z_is_agent_change() {
        assert_eq!(suggest_action(3.0), "investigate_agent_change");
    }

    #[test]
    fn suggest_action_negative_z_is_baseline_review() {
        assert_eq!(suggest_action(-3.0), "review_predictor_baseline");
    }

    #[test]
    fn build_drift_alert_uses_audit_routed_prefix() {
        // SLICE_05 R2 B2: drift_alert events MUST use the
        // `spendguard.audit.*` prefix to route into ImmutableAuditLog.
        let agg = fixture_agg();
        let z = compute_z_score(&agg).unwrap();
        let ce = build_drift_alert(&agg, z, &fixture_cfg());
        assert!(
            ce.r#type.starts_with("spendguard.audit."),
            "drift alert event must use audit-routed prefix; got: {}",
            ce.r#type
        );
    }

    #[test]
    fn build_drift_alert_contains_canonical_data_keys() {
        let agg = fixture_agg();
        let z = compute_z_score(&agg).unwrap();
        let ce = build_drift_alert(&agg, z, &fixture_cfg());
        let parsed: serde_json::Value = serde_json::from_slice(&ce.data).expect("data is json");
        for k in [
            "tenant_id",
            "model",
            "agent_id",
            "prompt_class",
            "baseline_mean",
            "baseline_stddev",
            "baseline_sample_size",
            "current_mean_7d",
            "z_score",
            "sample_size_7d",
            "drift_z_threshold",
            "min_samples_for_alert",
            "suggested_action",
        ] {
            assert!(parsed.get(k).is_some(), "missing key `{k}` in data: {parsed}");
        }
    }

    /// In-memory recording sink for the integration-style detect_and_emit
    /// test (no real signer required — uses DisabledSigner).
    struct RecordingSink {
        events: parking_lot::Mutex<Vec<CloudEvent>>,
    }

    #[async_trait::async_trait]
    impl DriftAlertSink for RecordingSink {
        async fn emit(&self, event: CloudEvent) -> Result<(), anyhow::Error> {
            self.events.lock().push(event);
            Ok(())
        }
    }

    #[tokio::test]
    async fn detect_and_emit_only_fires_on_breach() {
        use spendguard_signing::DisabledSigner;
        let sink = RecordingSink {
            events: parking_lot::Mutex::new(Vec::new()),
        };
        let signer = DisabledSigner::for_test("test-stats-aggregator".into());
        let mut breaching = fixture_agg();
        breaching.mean_7d = Some(200.0); // z = 5.0; far above 2.0
        let mut normal = fixture_agg();
        normal.mean_7d = Some(105.0); // z = 0.25; below 2.0
        let aggregates = vec![breaching, normal];
        let n = detect_and_emit(&aggregates, &fixture_cfg(), &signer, &sink)
            .await
            .expect("ok");
        assert_eq!(n, 1, "exactly one breach must fire");
        let events = sink.events.lock();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].r#type,
            PREDICTION_DRIFT_ALERT_EVENT_TYPE
        );
    }
}
