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
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use prost::Message;
use spendguard_signing::Signer;
use sqlx::postgres::PgPool;
use tonic::transport::Channel;
use tracing::{debug, info, warn};

use crate::aggregation::BucketAggregate;
use crate::proto::canonical_ingest::v1::{
    append_events_request::Route, canonical_ingest_client::CanonicalIngestClient,
    event_result::Status as EventStatus, AppendEventsRequest, AppendEventsResponse,
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

pub const DRIFT_ALERT_COOLDOWN_HOURS: i64 = 24;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DriftAlertKey {
    pub tenant_id: uuid::Uuid,
    pub model: String,
    pub agent_id: String,
    pub prompt_class: String,
}

impl From<&BucketAggregate> for DriftAlertKey {
    fn from(agg: &BucketAggregate) -> Self {
        Self {
            tenant_id: agg.tenant_id,
            model: agg.model.clone(),
            agent_id: agg.agent_id.clone(),
            prompt_class: agg.prompt_class.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DriftAlertCooldownDecision {
    Allowed { suppress_until: DateTime<Utc> },
    Suppressed { suppress_until: DateTime<Utc> },
}

#[async_trait::async_trait]
pub trait DriftAlertCooldown: Send + Sync {
    async fn check(
        &self,
        key: &DriftAlertKey,
        now: DateTime<Utc>,
    ) -> Result<DriftAlertCooldownDecision, anyhow::Error>;

    async fn record_emitted(
        &self,
        key: &DriftAlertKey,
        emitted_at: DateTime<Utc>,
        z_score: f32,
    ) -> Result<DateTime<Utc>, anyhow::Error>;
}

pub struct PostgresDriftAlertCooldownStore {
    pool: PgPool,
}

impl PostgresDriftAlertCooldownStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl DriftAlertCooldown for PostgresDriftAlertCooldownStore {
    async fn check(
        &self,
        key: &DriftAlertKey,
        now: DateTime<Utc>,
    ) -> Result<DriftAlertCooldownDecision, anyhow::Error> {
        let mut tx = self.pool.begin().await.context("begin drift cooldown tx")?;
        sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
            .bind(key.tenant_id.to_string())
            .execute(&mut *tx)
            .await
            .context("set RLS tenant_id for prediction_drift_alert_cooldowns")?;

        let active_until = sqlx::query_scalar::<_, DateTime<Utc>>(
            r#"
            SELECT suppress_until
            FROM prediction_drift_alert_cooldowns
            WHERE tenant_id = $1
              AND model = $2
              AND agent_id = $3
              AND prompt_class = $4
            "#,
        )
        .bind(key.tenant_id)
        .bind(&key.model)
        .bind(&key.agent_id)
        .bind(&key.prompt_class)
        .fetch_optional(&mut *tx)
        .await
        .context("read prediction_drift_alert cooldown")?;

        let decision = match active_until {
            Some(until) if until > now => DriftAlertCooldownDecision::Suppressed {
                suppress_until: until,
            },
            _ => DriftAlertCooldownDecision::Allowed {
                suppress_until: now + ChronoDuration::hours(DRIFT_ALERT_COOLDOWN_HOURS),
            },
        };

        tx.commit()
            .await
            .context("commit drift cooldown check tx")?;
        Ok(decision)
    }

    async fn record_emitted(
        &self,
        key: &DriftAlertKey,
        emitted_at: DateTime<Utc>,
        z_score: f32,
    ) -> Result<DateTime<Utc>, anyhow::Error> {
        if !z_score.is_finite() {
            anyhow::bail!("non-finite z_score cannot enter drift alert cooldown store");
        }

        let suppress_until = emitted_at + ChronoDuration::hours(DRIFT_ALERT_COOLDOWN_HOURS);
        let mut tx = self.pool.begin().await.context("begin drift cooldown tx")?;
        sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
            .bind(key.tenant_id.to_string())
            .execute(&mut *tx)
            .await
            .context("set RLS tenant_id for prediction_drift_alert_cooldowns")?;

        let recorded_until = sqlx::query_scalar::<_, DateTime<Utc>>(
            r#"
            INSERT INTO prediction_drift_alert_cooldowns (
              tenant_id, model, agent_id, prompt_class,
              last_emitted_at, suppress_until, last_z_score, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, clock_timestamp())
            ON CONFLICT (tenant_id, model, agent_id, prompt_class)
              DO UPDATE SET
                last_emitted_at = EXCLUDED.last_emitted_at,
                suppress_until = EXCLUDED.suppress_until,
                last_z_score = EXCLUDED.last_z_score,
                updated_at = clock_timestamp()
            RETURNING suppress_until
            "#,
        )
        .bind(key.tenant_id)
        .bind(&key.model)
        .bind(&key.agent_id)
        .bind(&key.prompt_class)
        .bind(emitted_at)
        .bind(suppress_until)
        .bind(z_score)
        .fetch_one(&mut *tx)
        .await
        .context("record prediction_drift_alert cooldown")?;

        tx.commit().await.context("commit drift cooldown tx")?;
        Ok(recorded_until)
    }
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
    if !mean_7d.is_finite() || !baseline_mean.is_finite() || !baseline_stddev.is_finite() {
        return None;
    }
    if baseline_stddev <= 0.0 {
        // baseline_stddev == 0 → all baseline samples identical.
        // Strict spec interpretation: z-score is undefined → no alert.
        return None;
    }
    let z = (mean_7d - baseline_mean) / baseline_stddev;
    z.is_finite().then_some(z)
}

/// Decision predicate: should this bucket emit a drift alert?
pub fn should_emit_drift_alert(agg: &BucketAggregate, cfg: &DriftDetectorConfig) -> bool {
    if !cfg.drift_z_threshold.is_finite() || cfg.drift_z_threshold <= 0.0 {
        return false;
    }
    let Some(z) = compute_z_score(agg) else {
        return false;
    };
    let Some(n7d) = agg.sample_size_7d else {
        return false;
    };
    z.abs() > cfg.drift_z_threshold && n7d >= cfg.min_samples_for_alert
}

fn numeric_guard_suppression_reason(
    agg: &BucketAggregate,
    cfg: &DriftDetectorConfig,
) -> Option<&'static str> {
    if !cfg.drift_z_threshold.is_finite() {
        return Some("non-finite drift_z_threshold");
    }
    if cfg.drift_z_threshold <= 0.0 {
        return Some("non-positive drift_z_threshold");
    }

    let Some(mean_7d) = agg.mean_7d else {
        return None;
    };
    if !mean_7d.is_finite() {
        return Some("non-finite mean_7d");
    }

    let Some(baseline_mean) = agg.baseline_mean else {
        return None;
    };
    if !baseline_mean.is_finite() {
        return Some("non-finite baseline_mean");
    }

    let Some(baseline_stddev) = agg.baseline_stddev else {
        return None;
    };
    if !baseline_stddev.is_finite() {
        return Some("non-finite baseline_stddev");
    }
    if baseline_stddev <= 0.0 {
        return Some("non-positive baseline_stddev");
    }

    let z = (mean_7d - baseline_mean) / baseline_stddev;
    (!z.is_finite()).then_some("non-finite z_score")
}

/// Build (but do not sign or emit) the prediction_drift_alert
/// CloudEvent for a bucket. Separated from the emission step so unit
/// tests can verify the envelope shape without a tonic Channel.
pub fn build_drift_alert(
    agg: &BucketAggregate,
    z_score: f32,
    cfg: &DriftDetectorConfig,
) -> Option<CloudEvent> {
    use bytes::Bytes;
    if !drift_alert_payload_is_finite(agg, z_score, cfg) {
        return None;
    }
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

    Some(CloudEvent {
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
    })
}

fn drift_alert_payload_is_finite(
    agg: &BucketAggregate,
    z_score: f32,
    cfg: &DriftDetectorConfig,
) -> bool {
    z_score.is_finite()
        && cfg.drift_z_threshold.is_finite()
        && cfg.drift_z_threshold > 0.0
        && agg.baseline_mean.is_some_and(f32::is_finite)
        && agg
            .baseline_stddev
            .is_some_and(|stddev| stddev.is_finite() && stddev > 0.0)
        && agg.mean_7d.is_some_and(f32::is_finite)
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
        let resp = guard
            .append_events(req)
            .await
            .context("canonical_ingest AppendEvents")?;
        ensure_append_accepted(resp.into_inner())
    }
}

pub(crate) fn ensure_append_accepted(resp: AppendEventsResponse) -> Result<(), anyhow::Error> {
    if resp.results.len() != 1 {
        return Err(anyhow::anyhow!(
            "AppendEvents prediction_drift_alert returned {} results for one event",
            resp.results.len()
        ));
    }

    let result = &resp.results[0];
    let status = EventStatus::try_from(result.status).unwrap_or(EventStatus::Unspecified);
    match status {
        EventStatus::Appended | EventStatus::Deduped => Ok(()),
        other => {
            let error_message = result
                .error
                .as_ref()
                .map(|err| err.message.as_str())
                .filter(|msg| !msg.is_empty())
                .unwrap_or("canonical_ingest returned no error detail");
            warn!(
                event_id = %result.event_id,
                status = ?other,
                err = %error_message,
                "canonical_ingest rejected prediction_drift_alert"
            );
            Err(anyhow::anyhow!(
                "AppendEvents prediction_drift_alert rejected event_id={} status={:?}: {}",
                result.event_id,
                other,
                error_message
            ))
        }
    }
}

/// Detect drift across a batch of bucket aggregates and emit signed
/// CloudEvents for those exceeding the threshold. Returns the count of
/// alerts emitted for metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DriftDetectionOutcome {
    pub emitted: usize,
    pub suppressed: usize,
}

pub async fn detect_and_emit(
    aggregates: &[BucketAggregate],
    cfg: &DriftDetectorConfig,
    signer: &dyn Signer,
    sink: &dyn DriftAlertSink,
    cooldown: &dyn DriftAlertCooldown,
) -> Result<DriftDetectionOutcome, anyhow::Error> {
    let mut outcome = DriftDetectionOutcome::default();
    for agg in aggregates {
        if let Some(reason) = numeric_guard_suppression_reason(agg, cfg) {
            outcome.suppressed += 1;
            warn!(
                tenant_id = %agg.tenant_id,
                model = %agg.model,
                agent_id = %agg.agent_id,
                prompt_class = %agg.prompt_class,
                reason,
                "drift alert suppressed by numeric safety guard"
            );
            continue;
        }
        let z = match compute_z_score(agg) {
            Some(z) => z,
            None => continue,
        };
        let Some(n7d) = agg.sample_size_7d else {
            continue;
        };
        if z.abs() <= cfg.drift_z_threshold || n7d < cfg.min_samples_for_alert {
            continue;
        }
        let key = DriftAlertKey::from(agg);
        match cooldown.check(&key, Utc::now()).await {
            Ok(DriftAlertCooldownDecision::Allowed { .. }) => {}
            Ok(DriftAlertCooldownDecision::Suppressed { suppress_until }) => {
                outcome.suppressed += 1;
                info!(
                    tenant_id = %agg.tenant_id,
                    model = %agg.model,
                    agent_id = %agg.agent_id,
                    prompt_class = %agg.prompt_class,
                    suppress_until = %suppress_until,
                    "drift alert suppressed by cooldown"
                );
                continue;
            }
            Err(e) => {
                outcome.suppressed += 1;
                warn!(
                    tenant_id = %agg.tenant_id,
                    model = %agg.model,
                    agent_id = %agg.agent_id,
                    prompt_class = %agg.prompt_class,
                    error = %e,
                    "drift alert cooldown unavailable; suppressing alert to avoid immutable audit spam"
                );
                continue;
            }
        }
        let Some(mut ce) = build_drift_alert(agg, z, cfg) else {
            outcome.suppressed += 1;
            warn!(
                tenant_id = %agg.tenant_id,
                model = %agg.model,
                agent_id = %agg.agent_id,
                prompt_class = %agg.prompt_class,
                z_score = z,
                "drift alert payload had non-finite numeric field; suppressing alert"
            );
            continue;
        };
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
                outcome.emitted += 1;
                if let Err(e) = cooldown.record_emitted(&key, Utc::now(), z).await {
                    warn!(
                        tenant_id = %agg.tenant_id,
                        model = %agg.model,
                        agent_id = %agg.agent_id,
                        prompt_class = %agg.prompt_class,
                        error = %e,
                        "drift alert emitted but cooldown record failed; future duplicate suppression may be unavailable"
                    );
                }
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
                    "drift alert emit failed before cooldown record; will retry next cycle"
                );
            }
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::canonical_ingest::v1::EventResult;
    use crate::proto::common::v1::Error as ProtoError;
    use std::collections::HashMap;
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
    fn z_score_none_when_inputs_are_non_finite() {
        let mut agg = fixture_agg();
        agg.mean_7d = Some(f32::NAN);
        assert!(compute_z_score(&agg).is_none());

        let mut agg = fixture_agg();
        agg.baseline_mean = Some(f32::INFINITY);
        assert!(compute_z_score(&agg).is_none());

        let mut agg = fixture_agg();
        agg.baseline_stddev = Some(f32::NEG_INFINITY);
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
    fn should_emit_false_when_threshold_is_non_finite() {
        let agg = fixture_agg();
        let mut cfg = fixture_cfg();
        cfg.drift_z_threshold = f32::NAN;
        assert!(!should_emit_drift_alert(&agg, &cfg));

        cfg.drift_z_threshold = f32::INFINITY;
        assert!(!should_emit_drift_alert(&agg, &cfg));
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
        let ce = build_drift_alert(&agg, z, &fixture_cfg()).expect("finite alert");
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
        let ce = build_drift_alert(&agg, z, &fixture_cfg()).expect("finite alert");
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
            assert!(
                parsed.get(k).is_some(),
                "missing key `{k}` in data: {parsed}"
            );
        }
    }

    #[test]
    fn build_drift_alert_rejects_non_finite_payload_values() {
        let agg = fixture_agg();
        assert!(build_drift_alert(&agg, f32::NAN, &fixture_cfg()).is_none());

        let mut cfg = fixture_cfg();
        cfg.drift_z_threshold = f32::INFINITY;
        assert!(build_drift_alert(&agg, 2.5, &cfg).is_none());

        cfg.drift_z_threshold = 0.0;
        assert!(build_drift_alert(&agg, 2.5, &cfg).is_none());

        let mut agg = fixture_agg();
        agg.mean_7d = Some(f32::INFINITY);
        assert!(build_drift_alert(&agg, 2.5, &fixture_cfg()).is_none());

        let mut agg = fixture_agg();
        agg.baseline_stddev = Some(0.0);
        assert!(build_drift_alert(&agg, 2.5, &fixture_cfg()).is_none());
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

    struct FailingSink;

    #[async_trait::async_trait]
    impl DriftAlertSink for FailingSink {
        async fn emit(&self, _event: CloudEvent) -> Result<(), anyhow::Error> {
            Err(anyhow::anyhow!("canonical_ingest rejected append"))
        }
    }

    #[derive(Default)]
    struct MemoryCooldown {
        entries: parking_lot::Mutex<HashMap<DriftAlertKey, DateTime<Utc>>>,
    }

    #[async_trait::async_trait]
    impl DriftAlertCooldown for MemoryCooldown {
        async fn check(
            &self,
            key: &DriftAlertKey,
            now: DateTime<Utc>,
        ) -> Result<DriftAlertCooldownDecision, anyhow::Error> {
            let entries = self.entries.lock();
            if let Some(existing) = entries.get(key) {
                if *existing > now {
                    return Ok(DriftAlertCooldownDecision::Suppressed {
                        suppress_until: *existing,
                    });
                }
            }
            let suppress_until = now + ChronoDuration::hours(DRIFT_ALERT_COOLDOWN_HOURS);
            Ok(DriftAlertCooldownDecision::Allowed { suppress_until })
        }

        async fn record_emitted(
            &self,
            key: &DriftAlertKey,
            emitted_at: DateTime<Utc>,
            _z_score: f32,
        ) -> Result<DateTime<Utc>, anyhow::Error> {
            let suppress_until = emitted_at + ChronoDuration::hours(DRIFT_ALERT_COOLDOWN_HOURS);
            let mut entries = self.entries.lock();
            entries.insert(key.clone(), suppress_until);
            Ok(suppress_until)
        }
    }

    struct FailingCooldown;

    #[async_trait::async_trait]
    impl DriftAlertCooldown for FailingCooldown {
        async fn check(
            &self,
            _key: &DriftAlertKey,
            _now: DateTime<Utc>,
        ) -> Result<DriftAlertCooldownDecision, anyhow::Error> {
            Err(anyhow::anyhow!("cooldown store unavailable"))
        }

        async fn record_emitted(
            &self,
            _key: &DriftAlertKey,
            _emitted_at: DateTime<Utc>,
            _z_score: f32,
        ) -> Result<DateTime<Utc>, anyhow::Error> {
            Err(anyhow::anyhow!("cooldown store unavailable"))
        }
    }

    #[tokio::test]
    async fn memory_cooldown_suppresses_same_key_until_expiry() {
        let store = MemoryCooldown::default();
        let key = DriftAlertKey::from(&fixture_agg());
        let now = Utc::now();
        let first = store.check(&key, now).await.expect("first");
        assert!(matches!(first, DriftAlertCooldownDecision::Allowed { .. }));
        store
            .record_emitted(&key, now, 2.5)
            .await
            .expect("record first");

        let second = store
            .check(&key, now + ChronoDuration::hours(1))
            .await
            .expect("second");
        assert!(matches!(
            second,
            DriftAlertCooldownDecision::Suppressed { .. }
        ));

        let after_expiry = store
            .check(&key, now + ChronoDuration::hours(25))
            .await
            .expect("after expiry");
        assert!(matches!(
            after_expiry,
            DriftAlertCooldownDecision::Allowed { .. }
        ));
    }

    #[tokio::test]
    async fn memory_cooldown_key_is_tenant_model_agent_prompt_scoped() {
        let store = MemoryCooldown::default();
        let agg = fixture_agg();
        let key = DriftAlertKey::from(&agg);
        let now = Utc::now();
        store.record_emitted(&key, now, 2.5).await.expect("seed");

        let mut different_prompt = key.clone();
        different_prompt.prompt_class = "rag".into();
        assert!(matches!(
            store
                .check(&different_prompt, now + ChronoDuration::hours(1))
                .await
                .expect("different prompt"),
            DriftAlertCooldownDecision::Allowed { .. }
        ));

        let mut different_tenant = key.clone();
        different_tenant.tenant_id = Uuid::new_v4();
        assert!(matches!(
            store
                .check(&different_tenant, now + ChronoDuration::hours(1))
                .await
                .expect("different tenant"),
            DriftAlertCooldownDecision::Allowed { .. }
        ));
    }

    fn append_response(status: EventStatus) -> AppendEventsResponse {
        AppendEventsResponse {
            results: vec![EventResult {
                event_id: "evt-1".to_string(),
                status: status as i32,
                ingest_position: None,
                error: Some(ProtoError {
                    code: 0,
                    message: format!("{status:?} for test"),
                    details: Default::default(),
                }),
            }],
        }
    }

    #[test]
    fn append_response_validation_accepts_durable_statuses() {
        ensure_append_accepted(append_response(EventStatus::Appended)).expect("appended ok");
        ensure_append_accepted(append_response(EventStatus::Deduped)).expect("deduped ok");
    }

    #[test]
    fn append_response_validation_rejects_quarantined_status() {
        let err = ensure_append_accepted(append_response(EventStatus::Quarantined))
            .expect_err("quarantined is not durable success");
        assert!(
            err.to_string().contains("Quarantined"),
            "error should include status, got {err}"
        );
    }

    #[test]
    fn append_response_validation_rejects_empty_or_multiple_results() {
        let empty_err = ensure_append_accepted(AppendEventsResponse { results: vec![] })
            .expect_err("missing per-event result must fail");
        assert!(empty_err.to_string().contains("0 results"));

        let multi_err = ensure_append_accepted(AppendEventsResponse {
            results: vec![
                EventResult {
                    event_id: "evt-1".to_string(),
                    status: EventStatus::Appended as i32,
                    ingest_position: None,
                    error: None,
                },
                EventResult {
                    event_id: "evt-2".to_string(),
                    status: EventStatus::Appended as i32,
                    ingest_position: None,
                    error: None,
                },
            ],
        })
        .expect_err("multiple results for one event must fail");
        assert!(multi_err.to_string().contains("2 results"));
    }

    #[tokio::test]
    async fn detect_and_emit_only_fires_on_breach() {
        use spendguard_signing::DisabledSigner;
        let sink = RecordingSink {
            events: parking_lot::Mutex::new(Vec::new()),
        };
        let cooldown = MemoryCooldown::default();
        let signer = DisabledSigner::for_test("test-stats-aggregator".into());
        let mut breaching = fixture_agg();
        breaching.mean_7d = Some(200.0); // z = 5.0; far above 2.0
        let mut normal = fixture_agg();
        normal.mean_7d = Some(105.0); // z = 0.25; below 2.0
        let aggregates = vec![breaching, normal];
        let outcome = detect_and_emit(&aggregates, &fixture_cfg(), &signer, &sink, &cooldown)
            .await
            .expect("ok");
        assert_eq!(outcome.emitted, 1, "exactly one breach must fire");
        assert_eq!(outcome.suppressed, 0);
        let events = sink.events.lock();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].r#type, PREDICTION_DRIFT_ALERT_EVENT_TYPE);
    }

    #[tokio::test]
    async fn detect_and_emit_suppresses_duplicate_same_key() {
        use spendguard_signing::DisabledSigner;
        let sink = RecordingSink {
            events: parking_lot::Mutex::new(Vec::new()),
        };
        let cooldown = MemoryCooldown::default();
        let signer = DisabledSigner::for_test("test-stats-aggregator".into());
        let mut first = fixture_agg();
        first.mean_7d = Some(200.0);
        let second = first.clone();

        let outcome = detect_and_emit(&[first, second], &fixture_cfg(), &signer, &sink, &cooldown)
            .await
            .expect("ok");

        assert_eq!(outcome.emitted, 1);
        assert_eq!(outcome.suppressed, 1);
        assert_eq!(sink.events.lock().len(), 1);
    }

    #[tokio::test]
    async fn detect_and_emit_tenant_isolation_keeps_cooldowns_independent() {
        use spendguard_signing::DisabledSigner;
        let sink = RecordingSink {
            events: parking_lot::Mutex::new(Vec::new()),
        };
        let cooldown = MemoryCooldown::default();
        let signer = DisabledSigner::for_test("test-stats-aggregator".into());
        let mut tenant_a = fixture_agg();
        tenant_a.mean_7d = Some(200.0);
        let mut tenant_b = tenant_a.clone();
        tenant_b.tenant_id = Uuid::new_v4();

        let outcome = detect_and_emit(
            &[tenant_a, tenant_b],
            &fixture_cfg(),
            &signer,
            &sink,
            &cooldown,
        )
        .await
        .expect("ok");

        assert_eq!(outcome.emitted, 2);
        assert_eq!(outcome.suppressed, 0);
        assert_eq!(sink.events.lock().len(), 2);
    }

    #[tokio::test]
    async fn detect_and_emit_suppresses_when_cooldown_store_unavailable() {
        use spendguard_signing::DisabledSigner;
        let sink = RecordingSink {
            events: parking_lot::Mutex::new(Vec::new()),
        };
        let signer = DisabledSigner::for_test("test-stats-aggregator".into());
        let mut breaching = fixture_agg();
        breaching.mean_7d = Some(200.0);

        let outcome = detect_and_emit(
            &[breaching],
            &fixture_cfg(),
            &signer,
            &sink,
            &FailingCooldown,
        )
        .await
        .expect("cooldown failure is fail-safe suppression, not cycle failure");

        assert_eq!(outcome.emitted, 0);
        assert_eq!(outcome.suppressed, 1);
        assert!(sink.events.lock().is_empty());
    }

    #[tokio::test]
    async fn detect_and_emit_counts_numeric_guard_suppression() {
        use spendguard_signing::DisabledSigner;
        let sink = RecordingSink {
            events: parking_lot::Mutex::new(Vec::new()),
        };
        let cooldown = MemoryCooldown::default();
        let signer = DisabledSigner::for_test("test-stats-aggregator".into());
        let mut non_finite = fixture_agg();
        non_finite.mean_7d = Some(f32::INFINITY);

        let outcome = detect_and_emit(&[non_finite], &fixture_cfg(), &signer, &sink, &cooldown)
            .await
            .expect("non-finite aggregate suppression should not fail the cycle");

        assert_eq!(outcome.emitted, 0);
        assert_eq!(outcome.suppressed, 1);
        assert!(sink.events.lock().is_empty());

        let sink = RecordingSink {
            events: parking_lot::Mutex::new(Vec::new()),
        };
        let mut invalid_threshold_cfg = fixture_cfg();
        invalid_threshold_cfg.drift_z_threshold = f32::NAN;

        let outcome = detect_and_emit(
            &[fixture_agg()],
            &invalid_threshold_cfg,
            &signer,
            &sink,
            &cooldown,
        )
        .await
        .expect("invalid threshold suppression should not fail the cycle");

        assert_eq!(outcome.emitted, 0);
        assert_eq!(outcome.suppressed, 1);
        assert!(sink.events.lock().is_empty());
    }

    #[tokio::test]
    async fn detect_and_emit_does_not_count_failed_append() {
        use spendguard_signing::DisabledSigner;
        let signer = DisabledSigner::for_test("test-stats-aggregator".into());
        let cooldown = MemoryCooldown::default();
        let mut breaching = fixture_agg();
        breaching.mean_7d = Some(200.0);
        let key = DriftAlertKey::from(&breaching);

        let outcome = detect_and_emit(
            &[breaching],
            &fixture_cfg(),
            &signer,
            &FailingSink,
            &cooldown,
        )
        .await
        .expect("emit failures are logged for retry, not fatal to cycle");

        assert_eq!(
            outcome.emitted, 0,
            "failed canonical append must not count as emitted"
        );
        assert_eq!(outcome.suppressed, 0);
        assert!(matches!(
            cooldown
                .check(&key, Utc::now() + ChronoDuration::hours(1))
                .await
                .expect("cooldown remains open after failed append"),
            DriftAlertCooldownDecision::Allowed { .. }
        ));
    }
}
