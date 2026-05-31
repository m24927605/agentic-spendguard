//! HARDEN_03 R2: real Postgres coverage for the out-of-order
//! audit.outcome path. Quarantined outcomes must retain the SLICE_06
//! aggregator mirror columns through release into canonical_events.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use spendguard_canonical_ingest::domain::event_routing::StorageClass;
use spendguard_canonical_ingest::persistence::append::{
    append_event, quarantine_audit_outcome, release_quarantined_outcomes, AppendInput,
    PredictionColumns,
};
use spendguard_canonical_ingest::verify_chain_lib::{verify_chain, VerifyChainArgs};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

const SCHEMA_BUNDLE_ID: &str = "22222222-2222-4222-8222-222222222222";
const SCHEMA_BUNDLE_HASH: &[u8] = &[0xcc; 32];
const TEST_EVENT_HASH: &[u8] = &[0x42; 32];

struct PgFixture {
    pool: PgPool,
}

async fn setup_postgres() -> Option<PgFixture> {
    let container = match Postgres::default().with_tag("16-alpine").start().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[canonical_ingest quarantine-release] Postgres not available: {e}");
            return None;
        }
    };
    let host_port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("postgres host port");
    let url =
        format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres?sslmode=disable");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("connect owner pool");

    apply_canonical_migrations(&pool).await;
    seed_schema_bundle(&pool).await;

    Box::leak(Box::new(container));
    Some(PgFixture { pool })
}

async fn apply_canonical_migrations(pool: &PgPool) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
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
        let sql = sql.replace("CREATE INDEX CONCURRENTLY", "CREATE INDEX");
        sqlx::raw_sql(&sql)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("apply migration {}: {e}", path.display()));
    }
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

fn base_append_input<'a>(
    event_id: Uuid,
    tenant_id: Uuid,
    decision_id: Uuid,
    run_id: Uuid,
    event_type: &'a str,
    producer_sequence: i64,
) -> AppendInput<'a> {
    AppendInput {
        event_id,
        tenant_id,
        decision_id: Some(decision_id),
        run_id: Some(run_id),
        event_type,
        storage_class: StorageClass::ImmutableAuditLog,
        producer_id: "canonical-ingest-test",
        producer_sequence,
        producer_signature: &[0x51, 0x52, 0x53, 0x54],
        signing_key_id: "test-key",
        event_hash: TEST_EVENT_HASH,
        schema_bundle_id: Uuid::parse_str(SCHEMA_BUNDLE_ID).expect("schema bundle uuid"),
        schema_bundle_hash: SCHEMA_BUNDLE_HASH,
        specversion: "1.0",
        source: "spendguard://canonical-ingest-test",
        event_time: Utc
            .with_ymd_and_hms(2026, 5, 31, 12, 0, 0)
            .single()
            .expect("fixed timestamp"),
        datacontenttype: "application/json",
        payload_json: serde_json::json!({"data": {}}),
        payload_blob_ref: None,
        region_id: "test-region",
        ingest_shard_id: "test-shard",
        failure_class: None,
        model: None,
        agent_id: None,
        run_id_mirror: None,
        prompt_class: None,
        prompt_class_fingerprint: None,
        prediction: PredictionColumns::default(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replay_dedup_same_producer_event_id_does_not_append_twice() {
    let Some(fx) = setup_postgres().await else {
        return;
    };
    let tenant_id = Uuid::new_v4();
    let decision_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();

    let first = base_append_input(
        event_id,
        tenant_id,
        decision_id,
        run_id,
        "spendguard.audit.decision",
        1,
    );
    let first_outcome = append_event(&fx.pool, first)
        .await
        .expect("append first event");
    assert!(matches!(
        first_outcome,
        spendguard_canonical_ingest::persistence::append::AppendOutcome::Appended { .. }
    ));

    let retry = base_append_input(
        event_id,
        tenant_id,
        decision_id,
        run_id,
        "spendguard.audit.decision",
        1,
    );
    let retry_outcome = append_event(&fx.pool, retry)
        .await
        .expect("dedupe replay retry");
    assert!(matches!(
        retry_outcome,
        spendguard_canonical_ingest::persistence::append::AppendOutcome::Deduped
    ));

    let canonical_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM canonical_events WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(&fx.pool)
            .await
            .expect("count canonical rows");
    assert_eq!(canonical_count, 1);

    let replay_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM canonical_event_replay_dedup WHERE producer_id = $1 AND event_id = $2")
            .bind("canonical-ingest-test")
            .bind(event_id)
            .fetch_one(&fx.pool)
            .await
            .expect("count replay ledger rows");
    assert_eq!(replay_count, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replay_dedup_rejects_same_producer_event_id_hash_mismatch() {
    let Some(fx) = setup_postgres().await else {
        return;
    };
    let tenant_id = Uuid::new_v4();
    let decision_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();

    let first = base_append_input(
        event_id,
        tenant_id,
        decision_id,
        run_id,
        "spendguard.audit.decision",
        1,
    );
    append_event(&fx.pool, first)
        .await
        .expect("append first event");

    let mut replay = base_append_input(
        event_id,
        tenant_id,
        decision_id,
        run_id,
        "spendguard.audit.decision",
        1,
    );
    replay.event_hash = &[0x99; 32];
    let err = append_event(&fx.pool, replay)
        .await
        .expect_err("hash mismatch replay is rejected");
    assert!(
        err.to_string().contains("replay hash mismatch"),
        "unexpected error: {err}"
    );

    let canonical_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM canonical_events WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(&fx.pool)
            .await
            .expect("count canonical rows");
    assert_eq!(canonical_count, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quarantined_outcome_release_preserves_aggregator_mirrors() {
    let Some(fx) = setup_postgres().await else {
        return;
    };
    let tenant_id = Uuid::new_v4();
    let decision_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let decision_event_id = Uuid::new_v4();
    let outcome_event_id = Uuid::new_v4();
    let prompt_fingerprint = "v1:chat_short|gpt-4o-mini|1";

    let mut outcome = base_append_input(
        outcome_event_id,
        tenant_id,
        decision_id,
        run_id,
        "spendguard.audit.outcome",
        2,
    );
    outcome.payload_json = serde_json::json!({
        "data": {
            "actual_input_tokens": 17,
            "actual_output_tokens": 29,
            "model": "gpt-4o-mini",
            "agent_id": "agent-alpha",
            "run_id": run_id.to_string(),
            "prompt_class": "chat_short",
            "prompt_class_fingerprint": prompt_fingerprint
        }
    });
    outcome.model = Some("gpt-4o-mini");
    outcome.agent_id = Some("agent-alpha");
    outcome.run_id_mirror = Some(run_id);
    outcome.prompt_class = Some("chat_short");
    outcome.prompt_class_fingerprint = Some(prompt_fingerprint);
    outcome.prediction.actual_input_tokens = Some(17);
    outcome.prediction.actual_output_tokens = Some(29);

    quarantine_audit_outcome(
        &fx.pool,
        outcome,
        Utc::now() + chrono::Duration::seconds(30),
    )
    .await
    .expect("quarantine outcome");

    let row = sqlx::query(
        r#"
        SELECT model, agent_id, run_id_mirror, prompt_class, prompt_class_fingerprint
        FROM audit_outcome_quarantine
        WHERE event_id = $1
        "#,
    )
    .bind(outcome_event_id)
    .fetch_one(&fx.pool)
    .await
    .expect("read quarantine row");
    assert_eq!(row.get::<String, _>("model"), "gpt-4o-mini");
    assert_eq!(row.get::<String, _>("agent_id"), "agent-alpha");
    assert_eq!(row.get::<Uuid, _>("run_id_mirror"), run_id);
    assert_eq!(row.get::<String, _>("prompt_class"), "chat_short");
    assert_eq!(
        row.get::<String, _>("prompt_class_fingerprint"),
        prompt_fingerprint
    );

    let mut decision = base_append_input(
        decision_event_id,
        tenant_id,
        decision_id,
        run_id,
        "spendguard.audit.decision",
        1,
    );
    decision.prediction.predicted_a_tokens = Some(64);
    decision.prediction.reserved_strategy = Some("A");
    decision.prediction.prediction_strategy_used = Some("A");
    decision.prediction.prediction_policy_used = Some("STRICT_CEILING");
    decision.prediction.tokenizer_tier = Some("T2");
    decision.prediction.run_projection_at_decision_atomic = Some(bigdecimal::BigDecimal::from(64));
    decision.prediction.run_steps_completed_so_far = Some(0);
    append_event(&fx.pool, decision)
        .await
        .expect("append decision");

    let released = release_quarantined_outcomes(
        &fx.pool,
        tenant_id,
        decision_id,
        "test-region",
        "test-shard",
    )
    .await
    .expect("release quarantine row");
    assert_eq!(released, 1);

    let row = sqlx::query(
        r#"
        SELECT model, agent_id, run_id_mirror, prompt_class,
               prompt_class_fingerprint, actual_output_tokens
        FROM canonical_events
        WHERE event_id = $1
          AND event_type = 'spendguard.audit.outcome'
        "#,
    )
    .bind(outcome_event_id)
    .fetch_one(&fx.pool)
    .await
    .expect("read released canonical outcome");
    assert_eq!(row.get::<String, _>("model"), "gpt-4o-mini");
    assert_eq!(row.get::<String, _>("agent_id"), "agent-alpha");
    assert_eq!(row.get::<Uuid, _>("run_id_mirror"), run_id);
    assert_eq!(row.get::<String, _>("prompt_class"), "chat_short");
    assert_eq!(
        row.get::<String, _>("prompt_class_fingerprint"),
        prompt_fingerprint
    );
    assert_eq!(row.get::<i64, _>("actual_output_tokens"), 29);

    let summary = verify_chain(
        &fx.pool,
        &VerifyChainArgs {
            tenant_id: Some(tenant_id),
            check_prediction_mirror: true,
            from: None,
            to: None,
        },
    )
    .await
    .expect("verify chain summary");
    assert_eq!(summary.rows_scanned, 2);
    assert_eq!(summary.rows_skipped_legacy, 0);
}
