//! HARDEN_03 / issue #160: real Postgres coverage for the
//! stats_aggregator cycle, cache RLS, and audit-routed drift alert rows.
//!
//! These tests use testcontainers Postgres, apply the canonical_ingest
//! migrations in order, seed canonical_events as the DB owner, then run
//! stats_aggregator queries as a non-superuser member of the canonical
//! application/reader roles. If Docker is unavailable, tests skip with a
//! clear message instead of producing a false negative in local shells.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use pretty_assertions::assert_eq;
use spendguard_stats_aggregator::aggregation::{
    aggregate_output_distribution, discover_active_tenants,
};
use spendguard_stats_aggregator::run_length::aggregate_run_length;
use spendguard_stats_aggregator::drift_detector::{
    DriftAlertCooldown, DriftAlertCooldownDecision, DriftAlertEmissionAttempt,
    DriftAlertEmissionDecision, DriftAlertKey, PostgresDriftAlertCooldownStore,
};
use spendguard_stats_aggregator::proto::common::v1::CloudEvent;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

const SCHEMA_BUNDLE_ID: &str = "22222222-2222-4222-8222-222222222222";
const SCHEMA_BUNDLE_HASH: &[u8] = &[0xcc; 32];

struct PgFixture {
    owner_pool: PgPool,
    app_pool: PgPool,
}

async fn setup_postgres() -> Option<PgFixture> {
    let container = match Postgres::default().with_tag("16-alpine").start().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[stats-aggregator #160] testcontainers Postgres not available: {e}");
            return None;
        }
    };
    let host_port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("postgres host port");
    let owner_url =
        format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres?sslmode=disable");
    let app_url =
        format!("postgres://stats_test:stats_test@127.0.0.1:{host_port}/postgres?sslmode=disable");

    let owner_pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&owner_url)
        .await
        .expect("connect owner pool");

    apply_canonical_migrations(&owner_pool).await;
    create_stats_test_role(&owner_pool).await;
    seed_schema_bundle(&owner_pool).await;

    let app_pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&app_url)
        .await
        .expect("connect app pool");

    // Keep the container alive for the duration of the test.
    Box::leak(Box::new(container));

    Some(PgFixture {
        owner_pool,
        app_pool,
    })
}

async fn apply_canonical_migrations(pool: &PgPool) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../canonical_ingest/migrations");
    let mut migrations = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read migrations dir {}: {e}", dir.display()))
        .filter_map(|entry| {
            let path = entry.expect("migration entry").path();
            if path.extension().and_then(|s| s.to_str()) == Some("sql") {
                Some(path)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    migrations.sort();

    for path in migrations {
        let sql = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read migration {}: {e}", path.display()));
        // sqlx sends a multi-statement migration string as one simple
        // query, which Postgres treats as a single transaction block.
        // Migration 0011 intentionally uses CREATE INDEX CONCURRENTLY
        // for production online DDL; the test DB is empty and can use a
        // normal CREATE INDEX while preserving the final schema shape.
        let sql = sql.replace("CREATE INDEX CONCURRENTLY", "CREATE INDEX");
        sqlx::raw_sql(&sql)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("apply migration {}: {e}", path.display()));
    }
}

async fn create_stats_test_role(pool: &PgPool) {
    sqlx::raw_sql(
        r#"
        CREATE ROLE stats_test LOGIN PASSWORD 'stats_test';
        GRANT canonical_ingest_application_role TO stats_test;
        GRANT canonical_ingest_reader_role TO stats_test;
        "#,
    )
    .execute(pool)
    .await
    .expect("create stats_test role");
}

async fn seed_schema_bundle(pool: &PgPool) {
    sqlx::query(
        r#"
        INSERT INTO schema_bundles (
          schema_bundle_id, schema_bundle_hash, canonical_schema_version
        )
        VALUES ($1, $2, 'spendguard.v1alpha1')
        "#,
    )
    .bind(Uuid::parse_str(SCHEMA_BUNDLE_ID).expect("schema bundle uuid"))
    .bind(SCHEMA_BUNDLE_HASH)
    .execute(pool)
    .await
    .expect("seed schema bundle");
}

fn pending_attempt(key: &DriftAlertKey, event_id: Uuid, z_score: f32) -> DriftAlertEmissionAttempt {
    let now = Utc::now();
    DriftAlertEmissionAttempt {
        event: CloudEvent {
            specversion: "1.0".to_string(),
            r#type: "spendguard.audit.prediction_drift_alert.v1alpha1".to_string(),
            source: format!("spendguard://stats-aggregator/{}", key.tenant_id),
            id: event_id.to_string(),
            time: Some(prost_types::Timestamp {
                seconds: now.timestamp(),
                nanos: now.timestamp_subsec_nanos() as i32,
            }),
            datacontenttype: "application/json".to_string(),
            data: bytes::Bytes::from_static(br#"{"z_score":2.5}"#),
            tenant_id: key.tenant_id.to_string(),
            producer_id: "stats-aggregator:test".to_string(),
            signing_key_id: "test-key".to_string(),
            producer_signature: bytes::Bytes::from_static(b"test-signature"),
            ..Default::default()
        },
        z_score,
    }
}

async fn seed_outcome_pair(
    pool: &PgPool,
    tenant_id: Uuid,
    seq: i64,
    age_days: i32,
    actual_output_tokens: i64,
) {
    let decision_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let decision_event_id = Uuid::new_v4();
    let outcome_event_id = Uuid::new_v4();

    insert_global_key(
        pool,
        decision_event_id,
        tenant_id,
        Some(decision_id),
        "spendguard.audit.decision",
        age_days,
    )
    .await;
    insert_canonical_event(CanonicalSeed {
        pool,
        event_id: decision_event_id,
        tenant_id,
        decision_id: Some(decision_id),
        run_id: Some(run_id),
        event_type: "spendguard.audit.decision",
        seq: seq * 2,
        age_days,
        actual_input_tokens: None,
        actual_output_tokens: None,
        model: None,
        agent_id: None,
        prompt_class: None,
    })
    .await;

    insert_global_key(
        pool,
        outcome_event_id,
        tenant_id,
        Some(decision_id),
        "spendguard.audit.outcome",
        age_days,
    )
    .await;
    insert_canonical_event(CanonicalSeed {
        pool,
        event_id: outcome_event_id,
        tenant_id,
        decision_id: Some(decision_id),
        run_id: Some(run_id),
        event_type: "spendguard.audit.outcome",
        seq: seq * 2 + 1,
        age_days,
        actual_input_tokens: Some(42),
        actual_output_tokens: Some(actual_output_tokens),
        model: Some("gpt-4o-mini"),
        agent_id: Some("agent-alpha"),
        prompt_class: Some("chat_short"),
    })
    .await;
}

async fn seed_sparse_outcome_pair(
    pool: &PgPool,
    tenant_id: Uuid,
    seq: i64,
    age_days: i32,
    actual_output_tokens: i64,
) {
    let decision_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let decision_event_id = Uuid::new_v4();
    let outcome_event_id = Uuid::new_v4();

    insert_global_key(
        pool,
        decision_event_id,
        tenant_id,
        Some(decision_id),
        "spendguard.audit.decision",
        age_days,
    )
    .await;
    insert_canonical_event(CanonicalSeed {
        pool,
        event_id: decision_event_id,
        tenant_id,
        decision_id: Some(decision_id),
        run_id: Some(run_id),
        event_type: "spendguard.audit.decision",
        seq: seq * 2,
        age_days,
        actual_input_tokens: None,
        actual_output_tokens: None,
        model: Some("gpt-4o-mini"),
        agent_id: Some("agent-alpha"),
        prompt_class: Some("chat_short"),
    })
    .await;

    insert_global_key(
        pool,
        outcome_event_id,
        tenant_id,
        Some(decision_id),
        "spendguard.audit.outcome",
        age_days,
    )
    .await;
    insert_canonical_event(CanonicalSeed {
        pool,
        event_id: outcome_event_id,
        tenant_id,
        decision_id: Some(decision_id),
        run_id: Some(run_id),
        event_type: "spendguard.audit.outcome",
        seq: seq * 2 + 1,
        age_days,
        actual_input_tokens: Some(42),
        actual_output_tokens: Some(actual_output_tokens),
        model: None,
        agent_id: None,
        prompt_class: None,
    })
    .await;
}

/// Seed a single DECISION event with no paired outcome — models a
/// decision-heavy / outcome-light tenant (e.g. heavy DENY traffic or
/// runs that never complete an outcome). Such a tenant must still be
/// discovered so aggregate_run_length populates
/// run_length_distribution_cache; otherwise run_cost_projector Signal 1
/// is stuck on the cold-start default.
async fn seed_decision_only(
    pool: &PgPool,
    tenant_id: Uuid,
    seq: i64,
    age_days: i32,
    run_id: Uuid,
) {
    let decision_id = Uuid::new_v4();
    let decision_event_id = Uuid::new_v4();

    insert_global_key(
        pool,
        decision_event_id,
        tenant_id,
        Some(decision_id),
        "spendguard.audit.decision",
        age_days,
    )
    .await;
    insert_canonical_event(CanonicalSeed {
        pool,
        event_id: decision_event_id,
        tenant_id,
        decision_id: Some(decision_id),
        run_id: Some(run_id),
        event_type: "spendguard.audit.decision",
        seq,
        age_days,
        actual_input_tokens: None,
        actual_output_tokens: None,
        model: Some("gpt-4o-mini"),
        agent_id: Some("agent-alpha"),
        prompt_class: Some("chat_short"),
    })
    .await;
}

async fn insert_global_key(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: Uuid,
    decision_id: Option<Uuid>,
    event_type: &str,
    age_days: i32,
) {
    sqlx::query(
        r#"
        INSERT INTO canonical_events_global_keys (
          event_id, tenant_id, decision_id, event_type, recorded_month, ingest_at
        )
        VALUES (
          $1, $2, $3, $4,
          DATE_TRUNC('month', now() - ($5::INT * interval '1 day'))::DATE,
          now() - ($5::INT * interval '1 day')
        )
        "#,
    )
    .bind(event_id)
    .bind(tenant_id)
    .bind(decision_id)
    .bind(event_type)
    .bind(age_days)
    .execute(pool)
    .await
    .expect("insert canonical_events_global_keys");
}

struct CanonicalSeed<'a> {
    pool: &'a PgPool,
    event_id: Uuid,
    tenant_id: Uuid,
    decision_id: Option<Uuid>,
    run_id: Option<Uuid>,
    event_type: &'a str,
    seq: i64,
    age_days: i32,
    actual_input_tokens: Option<i64>,
    actual_output_tokens: Option<i64>,
    model: Option<&'a str>,
    agent_id: Option<&'a str>,
    prompt_class: Option<&'a str>,
}

async fn insert_canonical_event(seed: CanonicalSeed<'_>) {
    sqlx::query(
        r#"
        INSERT INTO canonical_events (
          event_id, tenant_id, decision_id, run_id, event_type, storage_class,
          producer_id, producer_sequence, producer_signature, signing_key_id,
          schema_bundle_id, schema_bundle_hash,
          specversion, source, event_time, datacontenttype, payload_json,
          region_id, ingest_shard_id, ingest_log_offset, ingest_at,
          recorded_month,
          actual_input_tokens, actual_output_tokens,
          model, agent_id, prompt_class, prompt_class_fingerprint, run_id_mirror
        )
        VALUES (
          $1, $2, $3, $4, $5, 'immutable_audit_log',
          'stats-aggregator-test', $6, $7, 'stats-test-key',
          $8, $9,
          '1.0', 'spendguard://stats-aggregator-test',
          now() - ($10::INT * interval '1 day'),
          'application/json', '{}'::jsonb,
          'test-region', 'test-shard', $11,
          now() - ($10::INT * interval '1 day'),
          DATE_TRUNC('month', now() - ($10::INT * interval '1 day'))::DATE,
          $12, $13, $14, $15, $16, $17, $4
        )
        "#,
    )
    .bind(seed.event_id)
    .bind(seed.tenant_id)
    .bind(seed.decision_id)
    .bind(seed.run_id)
    .bind(seed.event_type)
    .bind(seed.seq)
    .bind(vec![0x51_u8, 0x60, 0x61, 0x62])
    .bind(Uuid::parse_str(SCHEMA_BUNDLE_ID).expect("schema bundle uuid"))
    .bind(SCHEMA_BUNDLE_HASH)
    .bind(seed.age_days)
    .bind(seed.seq)
    .bind(seed.actual_input_tokens)
    .bind(seed.actual_output_tokens)
    .bind(seed.model)
    .bind(seed.agent_id)
    .bind(seed.prompt_class)
    .bind(
        seed.prompt_class
            .map(|class| format!("v1:{class}|gpt-4o-mini|1")),
    )
    .execute(seed.pool)
    .await
    .unwrap_or_else(|e| panic!("insert canonical_events {}: {e}", seed.event_type));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cycle_e2e_postgres_populates_output_distribution_cache() {
    let Some(fx) = setup_postgres().await else {
        return;
    };
    let tenant_id = Uuid::new_v4();

    for idx in 0..8 {
        seed_outcome_pair(&fx.owner_pool, tenant_id, idx, 1, 100 + idx).await;
    }
    for idx in 8..16 {
        seed_outcome_pair(&fx.owner_pool, tenant_id, idx, 10, 180 + idx).await;
    }

    let aggregates = aggregate_output_distribution(&fx.app_pool, tenant_id)
        .await
        .expect("aggregate output distribution");
    assert_eq!(aggregates.len(), 1);
    assert_eq!(aggregates[0].sample_size_7d, Some(8));
    assert_eq!(aggregates[0].sample_size_30d, Some(16));
    assert_eq!(aggregates[0].baseline_sample_size, Some(8));

    let mut tx = fx.app_pool.begin().await.expect("begin RLS read tx");
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await
        .expect("set RLS tenant");
    let row = sqlx::query(
        r#"
        SELECT sample_size_7d, sample_size_30d
        FROM output_distribution_cache
        WHERE tenant_id = $1
          AND model = 'gpt-4o-mini'
          AND agent_id = 'agent-alpha'
          AND prompt_class = 'chat_short'
        "#,
    )
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await
    .expect("read cache row through RLS");
    assert_eq!(row.get::<i32, _>("sample_size_7d"), 8);
    assert_eq!(row.get::<i32, _>("sample_size_30d"), 16);
    tx.commit().await.expect("commit RLS read tx");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cycle_e2e_postgres_joins_sparse_outcomes_to_decision_mirrors() {
    let Some(fx) = setup_postgres().await else {
        return;
    };
    let tenant_id = Uuid::new_v4();

    for idx in 0..4 {
        seed_sparse_outcome_pair(&fx.owner_pool, tenant_id, idx, 1, 100 + idx).await;
    }
    for idx in 4..8 {
        seed_sparse_outcome_pair(&fx.owner_pool, tenant_id, idx, 10, 180 + idx).await;
    }

    let aggregates = aggregate_output_distribution(&fx.app_pool, tenant_id)
        .await
        .expect("aggregate sparse outcome distribution");
    assert_eq!(aggregates.len(), 1);
    assert_eq!(aggregates[0].model, "gpt-4o-mini");
    assert_eq!(aggregates[0].agent_id, "agent-alpha");
    assert_eq!(aggregates[0].prompt_class, "chat_short");
    assert_eq!(aggregates[0].sample_size_7d, Some(4));
    assert_eq!(aggregates[0].sample_size_30d, Some(8));
    assert_eq!(aggregates[0].baseline_sample_size, Some(4));

    let mut tx = fx.app_pool.begin().await.expect("begin RLS read tx");
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await
        .expect("set RLS tenant");
    let rows: i64 = sqlx::query_scalar(
        r#"
        SELECT count(*)
        FROM output_distribution_cache
        WHERE tenant_id = $1
          AND model = 'gpt-4o-mini'
          AND agent_id = 'agent-alpha'
          AND prompt_class = 'chat_short'
        "#,
    )
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await
    .expect("read joined cache row through RLS");
    assert_eq!(rows, 1);
    tx.commit().await.expect("commit RLS read tx");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rls_injection_blocks_cross_tenant_cache_reads_and_writes() {
    let Some(fx) = setup_postgres().await else {
        return;
    };
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();

    for idx in 0..4 {
        seed_outcome_pair(&fx.owner_pool, tenant_a, idx, 1, 100 + idx).await;
    }
    aggregate_output_distribution(&fx.app_pool, tenant_a)
        .await
        .expect("aggregate tenant a");

    let mut tx = fx.app_pool.begin().await.expect("begin tenant b tx");
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(tenant_b.to_string())
        .execute(&mut *tx)
        .await
        .expect("set RLS tenant b");

    let visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM output_distribution_cache WHERE tenant_id = $1")
            .bind(tenant_a)
            .fetch_one(&mut *tx)
            .await
            .expect("cross-tenant read count");
    assert_eq!(visible, 0, "tenant B must not see tenant A cache rows");

    let err = sqlx::query(
        r#"
        INSERT INTO output_distribution_cache (
          tenant_id, model, agent_id, prompt_class,
          sample_size_30d, computed_at, aggregation_version
        )
        VALUES ($1, 'gpt-4o-mini', 'agent-alpha', 'rag', 1, now(), 'v1alpha1')
        "#,
    )
    .bind(tenant_a)
    .execute(&mut *tx)
    .await
    .expect_err("RLS WITH CHECK must reject mismatched tenant write");
    let msg = err.to_string();
    assert!(
        msg.contains("row-level security") || msg.contains("violates row-level security policy"),
        "unexpected RLS error: {msg}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rls_missing_tenant_setting_does_not_match_nil_uuid_rows() {
    let Some(fx) = setup_postgres().await else {
        return;
    };
    let nil_tenant = Uuid::nil();

    sqlx::query(
        r#"
        INSERT INTO output_distribution_cache (
          tenant_id, model, agent_id, prompt_class,
          sample_size_30d, computed_at, aggregation_version
        )
        VALUES ($1, 'gpt-4o-mini', 'agent-alpha', 'rag', 1, now(), 'v1alpha1')
        "#,
    )
    .bind(nil_tenant)
    .execute(&fx.owner_pool)
    .await
    .expect("owner seeds nil tenant output cache row");

    sqlx::query(
        r#"
        INSERT INTO run_length_distribution_cache (
          tenant_id, agent_id, sample_size_30d, computed_at, aggregation_version
        )
        VALUES ($1, 'agent-alpha', 1, now(), 'v1alpha1')
        "#,
    )
    .bind(nil_tenant)
    .execute(&fx.owner_pool)
    .await
    .expect("owner seeds nil tenant run-length cache row");

    let mut tx = fx
        .app_pool
        .begin()
        .await
        .expect("begin app tx without tenant setting");

    let output_visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM output_distribution_cache WHERE tenant_id = $1")
            .bind(nil_tenant)
            .fetch_one(&mut *tx)
            .await
            .expect("nil output cache read without RLS setting");
    assert_eq!(
        output_visible, 0,
        "missing app.current_tenant_id must not fall back to nil UUID"
    );

    let run_visible: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_length_distribution_cache WHERE tenant_id = $1",
    )
    .bind(nil_tenant)
    .fetch_one(&mut *tx)
    .await
    .expect("nil run-length cache read without RLS setting");
    assert_eq!(
        run_visible, 0,
        "missing app.current_tenant_id must not fall back to nil UUID"
    );

    tx.commit()
        .await
        .expect("commit nil sentinel regression tx");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drift_alert_audit_row_can_land_without_prediction_mirror_columns() {
    let Some(fx) = setup_postgres().await else {
        return;
    };
    let tenant_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();

    insert_global_key(
        &fx.owner_pool,
        event_id,
        tenant_id,
        None,
        "spendguard.audit.prediction_drift_alert.v1alpha1",
        0,
    )
    .await;
    insert_canonical_event(CanonicalSeed {
        pool: &fx.owner_pool,
        event_id,
        tenant_id,
        decision_id: None,
        run_id: None,
        event_type: "spendguard.audit.prediction_drift_alert.v1alpha1",
        seq: 9001,
        age_days: 0,
        actual_input_tokens: None,
        actual_output_tokens: None,
        model: None,
        agent_id: None,
        prompt_class: None,
    })
    .await;

    let row = sqlx::query(
        r#"
        SELECT
          producer_signature,
          predicted_a_tokens, predicted_b_tokens, predicted_c_tokens,
          actual_input_tokens, actual_output_tokens,
          model, agent_id, prompt_class
        FROM canonical_events
        WHERE event_id = $1
        "#,
    )
    .bind(event_id)
    .fetch_one(&fx.owner_pool)
    .await
    .expect("read drift alert row");

    assert!(!row.get::<Vec<u8>, _>("producer_signature").is_empty());
    assert!(row
        .try_get::<Option<i64>, _>("predicted_a_tokens")
        .unwrap()
        .is_none());
    assert!(row
        .try_get::<Option<i64>, _>("predicted_b_tokens")
        .unwrap()
        .is_none());
    assert!(row
        .try_get::<Option<i64>, _>("predicted_c_tokens")
        .unwrap()
        .is_none());
    assert!(row
        .try_get::<Option<i64>, _>("actual_input_tokens")
        .unwrap()
        .is_none());
    assert!(row
        .try_get::<Option<i64>, _>("actual_output_tokens")
        .unwrap()
        .is_none());
    assert!(row.try_get::<Option<String>, _>("model").unwrap().is_none());
    assert!(row
        .try_get::<Option<String>, _>("agent_id")
        .unwrap()
        .is_none());
    assert!(row
        .try_get::<Option<String>, _>("prompt_class")
        .unwrap()
        .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drift_alert_cooldown_postgres_is_key_and_tenant_scoped() {
    let Some(fx) = setup_postgres().await else {
        return;
    };
    let store = PostgresDriftAlertCooldownStore::new(fx.app_pool.clone());
    let tenant_id = Uuid::new_v4();
    let now = Utc::now();
    let key = DriftAlertKey {
        tenant_id,
        model: "gpt-4o-mini".into(),
        agent_id: "agent-alpha".into(),
        prompt_class: "chat_short".into(),
    };

    let first = store.check(&key, now).await.expect("first");
    assert!(matches!(first, DriftAlertCooldownDecision::Allowed { .. }));
    store
        .record_emitted(&key, now, 2.5)
        .await
        .expect("record first");

    let duplicate = store
        .check(&key, now + ChronoDuration::hours(1))
        .await
        .expect("duplicate");
    assert!(matches!(
        duplicate,
        DriftAlertCooldownDecision::Suppressed { .. }
    ));

    let mut different_prompt = key.clone();
    different_prompt.prompt_class = "rag".into();
    let prompt_decision = store
        .check(&different_prompt, now + ChronoDuration::hours(1))
        .await
        .expect("different prompt");
    assert!(matches!(
        prompt_decision,
        DriftAlertCooldownDecision::Allowed { .. }
    ));

    let mut different_model = key.clone();
    different_model.model = "claude-3-5-sonnet".into();
    let model_decision = store
        .check(&different_model, now + ChronoDuration::hours(1))
        .await
        .expect("different model");
    assert!(matches!(
        model_decision,
        DriftAlertCooldownDecision::Allowed { .. }
    ));

    let mut different_agent = key.clone();
    different_agent.agent_id = "agent-beta".into();
    let agent_decision = store
        .check(&different_agent, now + ChronoDuration::hours(1))
        .await
        .expect("different agent");
    assert!(matches!(
        agent_decision,
        DriftAlertCooldownDecision::Allowed { .. }
    ));

    let mut different_tenant = key.clone();
    different_tenant.tenant_id = Uuid::new_v4();
    let tenant_decision = store
        .check(&different_tenant, now + ChronoDuration::hours(1))
        .await
        .expect("different tenant");
    assert!(matches!(
        tenant_decision,
        DriftAlertCooldownDecision::Allowed { .. }
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drift_alert_cooldown_postgres_reuses_pending_event_until_recorded() {
    let Some(fx) = setup_postgres().await else {
        return;
    };
    let store = PostgresDriftAlertCooldownStore::new(fx.app_pool.clone());
    let key = DriftAlertKey {
        tenant_id: Uuid::new_v4(),
        model: "gpt-4o-mini".into(),
        agent_id: "agent-alpha".into(),
        prompt_class: "chat_short".into(),
    };
    let now = Utc::now();
    let first_event_id = Uuid::new_v4();
    let second_event_id = Uuid::new_v4();

    let first = store
        .reserve_emission(&key, now, pending_attempt(&key, first_event_id, 2.5))
        .await
        .expect("first reserve");
    let DriftAlertEmissionDecision::Allowed { attempt: first } = first else {
        panic!("first reserve must be allowed");
    };
    assert_eq!(first.event.id, first_event_id.to_string());

    let second = store
        .reserve_emission(
            &key,
            now + ChronoDuration::minutes(1),
            pending_attempt(&key, second_event_id, 3.5),
        )
        .await
        .expect("second reserve reuses pending");
    let DriftAlertEmissionDecision::Allowed { attempt: second } = second else {
        panic!("second reserve must reuse pending");
    };
    assert_eq!(second.event.id, first_event_id.to_string());
    assert_eq!(second.z_score, 2.5);

    store
        .record_emitted(&key, now + ChronoDuration::minutes(2), second.z_score)
        .await
        .expect("record emitted clears pending");

    let duplicate = store
        .reserve_emission(
            &key,
            now + ChronoDuration::minutes(3),
            pending_attempt(&key, Uuid::new_v4(), 4.5),
        )
        .await
        .expect("active cooldown suppresses");
    assert!(matches!(
        duplicate,
        DriftAlertEmissionDecision::Suppressed { .. }
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drift_alert_cooldown_postgres_rls_blocks_missing_or_mismatched_tenant() {
    let Some(fx) = setup_postgres().await else {
        return;
    };
    let store = PostgresDriftAlertCooldownStore::new(fx.app_pool.clone());
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let key = DriftAlertKey {
        tenant_id: tenant_a,
        model: "gpt-4o-mini".into(),
        agent_id: "agent-alpha".into(),
        prompt_class: "chat_short".into(),
    };

    store
        .record_emitted(&key, Utc::now(), 2.5)
        .await
        .expect("seed tenant A cooldown");

    let mut missing_tenant_tx = fx.app_pool.begin().await.expect("begin missing tenant tx");
    let visible_without_tenant = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT count(*)
        FROM prediction_drift_alert_cooldowns
        WHERE tenant_id = $1
        "#,
    )
    .bind(tenant_a)
    .fetch_one(&mut *missing_tenant_tx)
    .await
    .expect("select without tenant context");
    assert_eq!(visible_without_tenant, 0);

    let missing_write_err = sqlx::query(
        r#"
        INSERT INTO prediction_drift_alert_cooldowns (
          tenant_id, model, agent_id, prompt_class,
          last_emitted_at, suppress_until, last_z_score
        )
        VALUES (
          $1, 'gpt-4o-mini', 'agent-beta', 'chat_short',
          clock_timestamp(), clock_timestamp() + interval '24 hours', 2.5
        )
        "#,
    )
    .bind(tenant_a)
    .execute(&mut *missing_tenant_tx)
    .await
    .expect_err("missing tenant context must fail WITH CHECK");
    assert!(
        missing_write_err.to_string().contains("row-level security"),
        "unexpected missing-tenant write error: {missing_write_err}"
    );
    let _ = missing_tenant_tx.rollback().await;

    let mut mismatched_tx = fx.app_pool.begin().await.expect("begin mismatched tx");
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(tenant_b.to_string())
        .execute(&mut *mismatched_tx)
        .await
        .expect("set mismatched tenant");

    let visible_with_mismatch = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT count(*)
        FROM prediction_drift_alert_cooldowns
        WHERE tenant_id = $1
        "#,
    )
    .bind(tenant_a)
    .fetch_one(&mut *mismatched_tx)
    .await
    .expect("select with mismatched tenant context");
    assert_eq!(visible_with_mismatch, 0);

    let updated = sqlx::query(
        r#"
        UPDATE prediction_drift_alert_cooldowns
        SET suppress_until = clock_timestamp() + interval '48 hours'
        WHERE tenant_id = $1
        "#,
    )
    .bind(tenant_a)
    .execute(&mut *mismatched_tx)
    .await
    .expect("mismatched tenant update is filtered by RLS");
    assert_eq!(updated.rows_affected(), 0);

    let mismatched_write_err = sqlx::query(
        r#"
        INSERT INTO prediction_drift_alert_cooldowns (
          tenant_id, model, agent_id, prompt_class,
          last_emitted_at, suppress_until, last_z_score
        )
        VALUES (
          $1, 'gpt-4o-mini', 'agent-gamma', 'chat_short',
          clock_timestamp(), clock_timestamp() + interval '24 hours', 2.5
        )
        "#,
    )
    .bind(tenant_a)
    .execute(&mut *mismatched_tx)
    .await
    .expect_err("mismatched tenant context must fail WITH CHECK");
    assert!(
        mismatched_write_err
            .to_string()
            .contains("row-level security"),
        "unexpected mismatched-tenant write error: {mismatched_write_err}"
    );
    let _ = mismatched_tx.rollback().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drift_alert_cooldown_postgres_accepts_canonical_multibyte_agent_id() {
    let Some(fx) = setup_postgres().await else {
        return;
    };
    let store = PostgresDriftAlertCooldownStore::new(fx.app_pool.clone());
    let key = DriftAlertKey {
        tenant_id: Uuid::new_v4(),
        model: "gpt-4o-mini".into(),
        agent_id: "客".repeat(128),
        prompt_class: "chat_short".into(),
    };

    let decision = store
        .check(&key, Utc::now())
        .await
        .expect("128-character multibyte agent_id is valid per canonical_events");
    assert!(matches!(
        decision,
        DriftAlertCooldownDecision::Allowed { .. }
    ));
    store
        .record_emitted(&key, Utc::now(), 2.5)
        .await
        .expect("record multibyte agent_id");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drift_alert_cooldown_postgres_rejects_non_finite_z_scores() {
    let Some(fx) = setup_postgres().await else {
        return;
    };
    let tenant_id = Uuid::new_v4();

    for value in ["'NaN'::REAL", "'Infinity'::REAL", "'-Infinity'::REAL"] {
        let mut tx = fx.app_pool.begin().await.expect("begin tx");
        sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
            .bind(tenant_id.to_string())
            .execute(&mut *tx)
            .await
            .expect("set tenant");
        let query = format!(
            r#"
            INSERT INTO prediction_drift_alert_cooldowns (
              tenant_id, model, agent_id, prompt_class,
              last_emitted_at, suppress_until, last_z_score
            )
            VALUES (
              $1, 'gpt-4o-mini', 'agent-alpha', $2,
              clock_timestamp(), clock_timestamp() + interval '24 hours', {value}
            )
            "#
        );
        let err = sqlx::query(&query)
            .bind(tenant_id)
            .bind("chat_short")
            .execute(&mut *tx)
            .await
            .expect_err("non-finite z-score must violate CHECK");
        assert!(
            err.to_string()
                .contains("prediction_drift_alert_cooldowns_last_z_score_check"),
            "unexpected error for {value}: {err}"
        );
        let _ = tx.rollback().await;
    }
}

/// Regression: a decision-heavy / outcome-light tenant (decisions but no
/// completed outcomes — e.g. heavy DENY traffic) must still be discovered
/// by discover_active_tenants and have its run-length cache populated by
/// aggregate_run_length. Before the fix, discovery was outcome-only, so
/// such tenants were never in the cycle's tenant list and
/// run_length_distribution_cache stayed permanently empty, pinning
/// run_cost_projector Signal 1 to the cold-start default.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discover_includes_decision_only_tenant_and_run_length_populates() {
    let Some(fx) = setup_postgres().await else {
        return;
    };
    let decision_only_tenant = Uuid::new_v4();

    // Seed two runs, each with several decision steps, and NO outcomes.
    let run_a = Uuid::new_v4();
    let run_b = Uuid::new_v4();
    for step in 0..5 {
        seed_decision_only(&fx.owner_pool, decision_only_tenant, step, 3, run_a).await;
    }
    for step in 5..8 {
        seed_decision_only(&fx.owner_pool, decision_only_tenant, step, 3, run_b).await;
    }

    // discover_active_tenants must now surface this tenant despite the
    // complete absence of outcome events.
    let tenants = discover_active_tenants(&fx.app_pool)
        .await
        .expect("discover active tenants");
    assert!(
        tenants.contains(&decision_only_tenant),
        "decision-only tenant missing from discovery: {tenants:?}"
    );

    // aggregate_run_length must populate the run-length cache for it.
    aggregate_run_length(&fx.app_pool, decision_only_tenant)
        .await
        .expect("aggregate run length for decision-only tenant");

    let mut tx = fx.app_pool.begin().await.expect("begin RLS read tx");
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(decision_only_tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("set RLS tenant");
    let row = sqlx::query(
        r#"
        SELECT sample_size_30d, p50_steps_30d
        FROM run_length_distribution_cache
        WHERE tenant_id = $1
          AND agent_id = 'agent-alpha'
        "#,
    )
    .bind(decision_only_tenant)
    .fetch_one(&mut *tx)
    .await
    .expect("run_length cache row must exist for decision-only tenant");
    // Two distinct run_ids → sample_size_30d (count of runs) == 2.
    assert_eq!(row.get::<i32, _>("sample_size_30d"), 2);
    // p50 over run lengths {5, 3} is finite and positive.
    assert!(row.get::<f32, _>("p50_steps_30d") > 0.0);
    tx.commit().await.expect("commit RLS read tx");
}
