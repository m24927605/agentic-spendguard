//! Slice 05 chaos test surface — spec §11.1 scenarios.
//!
//! Each test exercises one of the chaos scenarios called out in the
//! spec:
//!   * `tier1_endpoint_outage` — Anthropic 503 → breaker opens
//!   * `tier1_endpoint_recovery` — outage → recovery → half-open
//!     probe success → breaker closes
//!   * `drift_alert_cool_down` — inject drift → alert fires → 100%
//!     sampling for 1h → revert
//!
//! These are integration tests against the shadow worker's `process_one`
//! entry point — they exercise the same code path the spawned channel
//! loop does without taking on tokio task scheduling.

use std::sync::Arc;
use std::time::Duration;

use spendguard_signing::DisabledSigner;
use spendguard_tokenizer::encoders::EncoderKind;
use spendguard_tokenizer_service::shadow::{
    circuit_breaker::{CircuitBreakerConfig, CircuitBreakerState},
    provider_clients::anthropic::AnthropicClient,
    sample_rate_state::{SampleRateConfig, SampleRateState, ShadowKey},
    security::{LocalCountTokensQuota, StaticShadowSecurityStore},
    worker::{
        process_one, DriftAlertSink, InMemoryDriftAlertSink, InMemorySamplePersister,
        ProviderRoster, SamplePersister, ShadowEvent, ShadowOutcome, ShadowWorkerDeps,
    },
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn deps_with(
    sample_rate: Arc<SampleRateState>,
    cb: Arc<CircuitBreakerState>,
    providers: ProviderRoster,
    persister: Arc<dyn SamplePersister>,
    alert_sink: Arc<dyn DriftAlertSink>,
) -> ShadowWorkerDeps {
    ShadowWorkerDeps {
        sample_rate,
        circuit_breaker: cb,
        providers,
        persister,
        alert_sink,
        sample_rate_overrides: None,
        security: Arc::new(StaticShadowSecurityStore::allow_all_for_tests(1_000)),
        count_tokens_quota: Arc::new(LocalCountTokensQuota::default()),
        signer: Arc::new(DisabledSigner::for_test("tokenizer-chaos:test".into())),
        event_source: "spendguard://tokenizer-service/chaos".into(),
        channel_capacity: 16,
    }
}

/// R2 B5: tenant_id is UUID in the schema; tests share one fixed UUID
/// so the in-memory ShadowKey for assertion stays predictable.
fn chaos_tenant_id() -> uuid::Uuid {
    uuid::Uuid::parse_str("01918000-0000-7c10-8c10-0000000000ce").unwrap()
}

fn chaos_tenant_id_string() -> String {
    chaos_tenant_id().to_string()
}

fn ev() -> ShadowEvent {
    ShadowEvent {
        tenant_id: chaos_tenant_id(),
        model: "claude-3-5-sonnet-20241022".into(),
        encoder_kind: EncoderKind::Anthropic,
        t2_input_tokens: 100,
        t2_tokenizer_version_id: "01918000-0000-7c10-8c10-000000000010".into(),
        raw_text: "chaos test input".into(),
    }
}

#[tokio::test]
async fn tier1_endpoint_outage_opens_the_breaker() {
    // §11.1 tier1_endpoint_outage: simulate Anthropic 503 → 10
    // consecutive failures → breaker opens → subsequent samples
    // SkipOpen.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages/count_tokens"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let client = AnthropicClient::with_base_url("k", server.uri()).unwrap();
    let sample_rate = SampleRateState::new(SampleRateConfig {
        default_rate: 1.0, // always-sample so we drive the failure count
        cool_down: Duration::from_secs(3600),
        cool_down_rate: 1.0,
    });
    let cb = CircuitBreakerState::new(CircuitBreakerConfig {
        failure_threshold: 10,
        open_duration: Duration::from_millis(200),
    });
    let providers = ProviderRoster {
        anthropic: Some(client),
        gemini: None,
    };
    let persister = Arc::new(InMemorySamplePersister::default());
    let alert_sink = Arc::new(InMemoryDriftAlertSink::default());
    let deps = deps_with(
        sample_rate.clone(),
        cb.clone(),
        providers,
        persister.clone(),
        alert_sink.clone(),
    );

    let key = ShadowKey {
        tenant_id: chaos_tenant_id_string(),
        model: "claude-3-5-sonnet-20241022".into(),
    };

    // 10 failures to trip.
    for _ in 0..10 {
        let out = process_one(&ev(), &deps).await;
        assert_eq!(out, ShadowOutcome::ProviderFailed);
    }
    assert_eq!(cb.consecutive_failures(&key), 10);

    // 11th call must skip because the breaker is Open.
    let out = process_one(&ev(), &deps).await;
    assert_eq!(out, ShadowOutcome::Skipped);

    // No persisted samples — the breaker prevented even an attempt.
    assert_eq!(persister.rows.lock().len(), 0);
    assert_eq!(alert_sink.events.lock().len(), 0);
}

#[tokio::test]
async fn tier1_endpoint_recovery_closes_the_breaker_via_probe() {
    // §11.1 tier1_endpoint_recovery: after the outage, recovery + a
    // single probe success transitions HalfOpen → Closed.
    let outage_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages/count_tokens"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&outage_server)
        .await;

    let recovery_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages/count_tokens"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "input_tokens": 100 })),
        )
        .mount(&recovery_server)
        .await;

    let sample_rate = SampleRateState::new(SampleRateConfig {
        default_rate: 1.0,
        cool_down: Duration::from_secs(3600),
        cool_down_rate: 1.0,
    });
    let cb = CircuitBreakerState::new(CircuitBreakerConfig {
        failure_threshold: 10,
        // 100ms so the test can observe the half-open transition cheaply.
        open_duration: Duration::from_millis(100),
    });
    let persister = Arc::new(InMemorySamplePersister::default());
    let alert_sink = Arc::new(InMemoryDriftAlertSink::default());
    let key = ShadowKey {
        tenant_id: chaos_tenant_id_string(),
        model: "claude-3-5-sonnet-20241022".into(),
    };

    // Phase 1: outage trips the breaker.
    {
        let outage_client = AnthropicClient::with_base_url("k", outage_server.uri()).unwrap();
        let providers = ProviderRoster {
            anthropic: Some(outage_client),
            gemini: None,
        };
        let deps = deps_with(
            sample_rate.clone(),
            cb.clone(),
            providers,
            persister.clone(),
            alert_sink.clone(),
        );
        for _ in 0..10 {
            let _ = process_one(&ev(), &deps).await;
        }
        assert_eq!(cb.consecutive_failures(&key), 10);
    }

    // Phase 2: provider recovers; wait for open_duration to elapse so
    // the next attempt triggers HalfOpen → probe success → Closed.
    tokio::time::sleep(Duration::from_millis(150)).await;

    {
        let recovery_client = AnthropicClient::with_base_url("k", recovery_server.uri()).unwrap();
        let providers = ProviderRoster {
            anthropic: Some(recovery_client),
            gemini: None,
        };
        let deps = deps_with(
            sample_rate.clone(),
            cb.clone(),
            providers,
            persister.clone(),
            alert_sink.clone(),
        );
        let out = process_one(&ev(), &deps).await;
        assert_eq!(out, ShadowOutcome::Sampled);
        assert_eq!(cb.consecutive_failures(&key), 0);
    }
}

#[tokio::test]
async fn drift_alert_cool_down_lifts_sampling_to_one_hundred_percent() {
    // §11.1 drift_alert_cool_down: a single sample with drift > 0.01
    // triggers the cool-down window; subsequent snapshot reports 100%.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages/count_tokens"))
        // Tier 1 reports 110 vs Tier 2's 100 → drift 9.1% ≫ 1%.
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "input_tokens": 110 })),
        )
        .mount(&server)
        .await;

    let client = AnthropicClient::with_base_url("k", server.uri()).unwrap();
    let sample_rate = SampleRateState::new(SampleRateConfig {
        default_rate: 1.0,
        cool_down: Duration::from_secs(3600),
        cool_down_rate: 1.0,
    });
    let cb = CircuitBreakerState::new(CircuitBreakerConfig::default());
    let providers = ProviderRoster {
        anthropic: Some(client),
        gemini: None,
    };
    let persister = Arc::new(InMemorySamplePersister::default());
    let alert_sink = Arc::new(InMemoryDriftAlertSink::default());
    let deps = deps_with(
        sample_rate.clone(),
        cb,
        providers,
        persister.clone(),
        alert_sink.clone(),
    );

    let out = process_one(&ev(), &deps).await;
    assert_eq!(out, ShadowOutcome::Alerted);

    let key = ShadowKey {
        tenant_id: chaos_tenant_id_string(),
        model: "claude-3-5-sonnet-20241022".into(),
    };
    let snap = sample_rate.snapshot(&key);
    assert!(snap.in_cool_down);
    assert!((snap.effective_rate - 1.0).abs() < f64::EPSILON);

    let rows = persister.rows.lock();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].drift_alert_decided);

    let events = alert_sink.events.lock();
    assert_eq!(events.len(), 1);
    let ce = &events[0];
    // R2 B1: spendguard.audit.* prefix routes to ImmutableAuditLog.
    assert_eq!(ce.r#type, "spendguard.audit.tokenizer_drift_alert.v1alpha1");
    let data: serde_json::Value = serde_json::from_slice(&ce.data).unwrap();
    assert_eq!(data["tier1_count"], 110);
    assert_eq!(data["tier2_count"], 100);
    assert_eq!(data["encoder_kind"], "ANTHROPIC_BPE");
}
