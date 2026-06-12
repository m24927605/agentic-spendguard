use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Timelike, Utc};
use serde_json::{json, Value};
use spendguard_ledger::session_reservations::{
    commit_session_delta, expire_session, release_session, reserve_session,
    CommitSessionDeltaLedgerRequest, ExpireSessionLedgerRequest, PricingFreezeRef,
    ReleaseSessionLedgerRequest, ReserveSessionLedgerRequest, SessionReservationError,
};
use sqlx::postgres::PgPoolOptions;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

const SESSION_RESERVATIONS_SQL: &str = include_str!("../migrations/0062_session_reservations.sql");
static TEST_AUDIT_SEQUENCE: AtomicI64 = AtomicI64::new(1);
const MINIMAL_DEPENDENCIES_SQL: &str = r#"
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE ledger_units (
    unit_id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    unit_kind TEXT NOT NULL,
    unit_name TEXT,
    scale INT NOT NULL,
    rounding_mode TEXT NOT NULL,
    token_kind TEXT,
    model_family TEXT
);

CREATE TABLE budget_window_instances (
    window_instance_id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    budget_id UUID NOT NULL,
    window_type TEXT NOT NULL,
    timezone TEXT,
    tzdb_version TEXT NOT NULL,
    boundary_start TIMESTAMPTZ,
    boundary_end TIMESTAMPTZ,
    computed_from_snapshot_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE ledger_shards (
    ledger_shard_id SMALLINT PRIMARY KEY,
    shard_generation BIGINT NOT NULL,
    status TEXT NOT NULL
);

CREATE TABLE ledger_sequence_allocators (
    ledger_shard_id SMALLINT PRIMARY KEY REFERENCES ledger_shards(ledger_shard_id),
    last_sequence BIGINT NOT NULL DEFAULT 0
);

INSERT INTO ledger_shards (ledger_shard_id, shard_generation, status)
VALUES (1, 1, 'active');
INSERT INTO ledger_sequence_allocators (ledger_shard_id, last_sequence)
VALUES (1, 0);

CREATE TABLE ledger_accounts (
    ledger_account_id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    budget_id UUID NOT NULL,
    window_instance_id UUID NOT NULL REFERENCES budget_window_instances(window_instance_id),
    account_kind TEXT NOT NULL,
    unit_id UUID NOT NULL REFERENCES ledger_units(unit_id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, budget_id, window_instance_id, account_kind, unit_id)
);

CREATE TABLE pricing_snapshots (
    pricing_version TEXT PRIMARY KEY,
    price_snapshot_hash BYTEA NOT NULL,
    fx_rate_version TEXT NOT NULL,
    unit_conversion_version TEXT NOT NULL,
    schema_json JSONB NOT NULL,
    signature BYTEA NOT NULL,
    signing_key_id TEXT NOT NULL,
    deployed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deployed_by TEXT NOT NULL
);

CREATE TABLE ledger_transactions (
    ledger_transaction_id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    operation_kind TEXT NOT NULL,
    posting_state TEXT NOT NULL DEFAULT 'pending',
    posted_at TIMESTAMPTZ,
    idempotency_key TEXT NOT NULL,
    request_hash BYTEA NOT NULL,
    minimal_replay_response JSONB NOT NULL DEFAULT '{}'::JSONB,
    audit_decision_event_id UUID,
    decision_id UUID,
    effective_at TIMESTAMPTZ NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    lock_order_token TEXT NOT NULL,
    UNIQUE (tenant_id, operation_kind, idempotency_key)
);

CREATE TABLE audit_outbox (
    audit_outbox_id UUID PRIMARY KEY,
    audit_decision_event_id UUID NOT NULL,
    decision_id UUID NOT NULL,
    tenant_id UUID NOT NULL,
    ledger_transaction_id UUID NOT NULL REFERENCES ledger_transactions(ledger_transaction_id),
    event_type TEXT NOT NULL,
    cloudevent_payload JSONB NOT NULL,
    cloudevent_payload_signature BYTEA NOT NULL,
    ledger_fencing_epoch BIGINT NOT NULL,
    workload_instance_id TEXT NOT NULL,
    pending_forward BOOLEAN NOT NULL DEFAULT TRUE,
    forward_attempts INT NOT NULL DEFAULT 0,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    recorded_month DATE NOT NULL,
    producer_sequence BIGINT NOT NULL,
    idempotency_key TEXT NOT NULL
);

CREATE TABLE audit_outbox_global_keys (
    audit_decision_event_id UUID NOT NULL PRIMARY KEY,
    tenant_id UUID NOT NULL,
    decision_id UUID NOT NULL,
    event_type TEXT NOT NULL,
    operation_kind TEXT NOT NULL,
    workload_instance_id TEXT NOT NULL,
    producer_sequence BIGINT NOT NULL,
    idempotency_key TEXT NOT NULL,
    recorded_month DATE NOT NULL,
    audit_outbox_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE UNIQUE INDEX audit_outbox_global_idempotency_uq
    ON audit_outbox_global_keys (tenant_id, operation_kind, idempotency_key);

CREATE TABLE ledger_entries (
    ledger_entry_id UUID PRIMARY KEY,
    ledger_transaction_id UUID NOT NULL REFERENCES ledger_transactions(ledger_transaction_id),
    ledger_account_id UUID NOT NULL REFERENCES ledger_accounts(ledger_account_id),
    tenant_id UUID NOT NULL,
    budget_id UUID NOT NULL,
    window_instance_id UUID,
    unit_id UUID NOT NULL REFERENCES ledger_units(unit_id),
    direction TEXT NOT NULL,
    amount_atomic NUMERIC(38,0) NOT NULL CHECK (amount_atomic >= 0),
    pricing_version TEXT NOT NULL,
    price_snapshot_hash BYTEA NOT NULL,
    fx_rate_version TEXT,
    unit_conversion_version TEXT,
    reservation_id UUID,
    commit_event_kind TEXT,
    invoice_line_item_ref TEXT,
    ledger_shard_id SMALLINT NOT NULL REFERENCES ledger_shards(ledger_shard_id),
    ledger_sequence BIGINT NOT NULL,
    effective_at TIMESTAMPTZ NOT NULL,
    effective_month DATE NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    recorded_month DATE NOT NULL
);

CREATE OR REPLACE FUNCTION nextval_per_shard(p_shard_id SMALLINT)
RETURNS BIGINT AS $$
DECLARE v_next BIGINT;
BEGIN
    UPDATE ledger_sequence_allocators
       SET last_sequence = last_sequence + 1
     WHERE ledger_shard_id = p_shard_id
    RETURNING last_sequence INTO v_next;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'ledger_shard_id % has no sequence allocator', p_shard_id;
    END IF;
    RETURN v_next;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION assert_per_unit_balance_now(p_tx_id UUID)
RETURNS VOID AS $$
DECLARE v_imbalanced TEXT;
BEGIN
    SELECT string_agg(unit_id::TEXT || ':' || diff::TEXT, ', ')
      INTO v_imbalanced
      FROM (
          SELECT unit_id,
                 SUM(CASE WHEN direction = 'debit' THEN amount_atomic
                          WHEN direction = 'credit' THEN -amount_atomic END) AS diff
            FROM ledger_entries
           WHERE ledger_transaction_id = p_tx_id
           GROUP BY unit_id
      ) per_unit
     WHERE diff <> 0;
    IF v_imbalanced IS NOT NULL THEN
        RAISE EXCEPTION 'per-unit balance violation: %', v_imbalanced
            USING ERRCODE = '23514';
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION assert_ledger_transaction_has_audit()
RETURNS TRIGGER AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM audit_outbox
         WHERE ledger_transaction_id = NEW.ledger_transaction_id
    ) THEN
        RAISE EXCEPTION
            'AUDIT_INVARIANT_VIOLATED: ledger_transaction % posted with no audit_outbox row',
            NEW.ledger_transaction_id
            USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER ledger_transactions_must_have_audit
    AFTER INSERT ON ledger_transactions
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION assert_ledger_transaction_has_audit();
"#;

#[derive(Clone, Debug)]
struct Fixture {
    tenant_id: Uuid,
    budget_id: Uuid,
    window_instance_id: Uuid,
    unit_id: Uuid,
    pricing: PricingFreezeRef,
}

async fn setup_pool() -> Option<sqlx::PgPool> {
    let container = match Postgres::default().start().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[d41s02-test] testcontainers Postgres not available: {e}");
            return None;
        }
    };
    let host_port = container.get_host_port_ipv4(5432).await.expect("host port");
    let url =
        format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres?sslmode=disable");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&url)
        .await
        .expect("connect postgres");

    for sql in [MINIMAL_DEPENDENCIES_SQL, SESSION_RESERVATIONS_SQL] {
        sqlx::raw_sql(sql)
            .execute(&pool)
            .await
            .expect("apply migration");
    }

    Box::leak(Box::new(container));
    Some(pool)
}

async fn insert_fixture(pool: &sqlx::PgPool) -> Fixture {
    let tenant_id = Uuid::new_v4();
    let budget_id = Uuid::new_v4();
    let window_instance_id = Uuid::new_v4();
    let unit_id = Uuid::new_v4();
    let available_account_id = Uuid::new_v4();
    let reserved_account_id = Uuid::new_v4();
    let committed_account_id = Uuid::new_v4();
    let adjustment_account_id = Uuid::new_v4();
    let seed_tx_id = Uuid::new_v4();
    let pricing = PricingFreezeRef {
        pricing_version: "price-v1".into(),
        price_snapshot_hash_hex: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .into(),
        fx_rate_version: "fx-v1".into(),
        unit_conversion_version: "uc-v1".into(),
    };
    sqlx::query(
        r#"
        INSERT INTO ledger_units (
            unit_id, tenant_id, unit_kind, unit_name, scale, rounding_mode,
            token_kind, model_family
        ) VALUES ($1, $2, 'token', 'voice-token', 0, 'half_even', 'llm_token', 'd41-test')
        "#,
    )
    .bind(unit_id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("insert unit");

    sqlx::query(
        r#"
        INSERT INTO budget_window_instances (
            window_instance_id, tenant_id, budget_id, window_type, timezone,
            tzdb_version, boundary_start, boundary_end, computed_from_snapshot_at
        ) VALUES (
            $1, $2, $3, 'rolling', 'UTC', '2026a',
            clock_timestamp(), clock_timestamp() + INTERVAL '1 hour',
            clock_timestamp()
        )
        "#,
    )
    .bind(window_instance_id)
    .bind(tenant_id)
    .bind(budget_id)
    .execute(pool)
    .await
    .expect("insert window");

    sqlx::query(
        r#"
        INSERT INTO pricing_snapshots (
            pricing_version, price_snapshot_hash, fx_rate_version,
            unit_conversion_version, schema_json, signature, signing_key_id, deployed_by
        ) VALUES ($1, decode($2, 'hex'), $3, $4, '{}'::jsonb, decode('00', 'hex'), 'test-key', 'd41-test')
        "#,
    )
    .bind(&pricing.pricing_version)
    .bind(&pricing.price_snapshot_hash_hex)
    .bind(&pricing.fx_rate_version)
    .bind(&pricing.unit_conversion_version)
    .execute(pool)
    .await
    .expect("insert pricing");

    for (account_id, kind) in [
        (available_account_id, "available_budget"),
        (reserved_account_id, "reserved_hold"),
        (committed_account_id, "committed_spend"),
        (adjustment_account_id, "adjustment"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO ledger_accounts (
                ledger_account_id, tenant_id, budget_id, window_instance_id,
                account_kind, unit_id
            ) VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(account_id)
        .bind(tenant_id)
        .bind(budget_id)
        .bind(window_instance_id)
        .bind(kind)
        .bind(unit_id)
        .execute(pool)
        .await
        .expect("insert account");
    }

    sqlx::query(
        r#"
        WITH tx AS (
            INSERT INTO ledger_transactions (
                ledger_transaction_id, tenant_id, operation_kind, posting_state,
                posted_at, idempotency_key, request_hash, minimal_replay_response,
                audit_decision_event_id, decision_id,
                effective_at, recorded_at, lock_order_token
            ) VALUES (
                $1, $2, 'adjustment', 'posted', clock_timestamp(),
                'seed-available', digest('seed-available', 'sha256'), '{}'::jsonb,
                gen_random_uuid(), gen_random_uuid(),
                clock_timestamp(), clock_timestamp(), 'seed'
            )
            RETURNING ledger_transaction_id, tenant_id, audit_decision_event_id,
                      decision_id, idempotency_key, operation_kind
        ), audit AS (
            INSERT INTO audit_outbox (
                audit_outbox_id, audit_decision_event_id, decision_id,
                tenant_id, ledger_transaction_id, event_type,
                cloudevent_payload, cloudevent_payload_signature,
                ledger_fencing_epoch, workload_instance_id,
                pending_forward, forward_attempts,
                recorded_at, recorded_month, producer_sequence, idempotency_key
            )
            SELECT
                gen_random_uuid(), audit_decision_event_id, decision_id,
                tenant_id, ledger_transaction_id, 'spendguard.audit.outcome',
                '{}'::jsonb, digest('seed-audit', 'sha256'),
                1, 'session-reservation-ledger',
                TRUE, 0,
                clock_timestamp(), date_trunc('month', clock_timestamp())::date,
                nextval_per_shard(1::smallint), idempotency_key
            FROM tx
            RETURNING audit_outbox_id, audit_decision_event_id, decision_id,
                      tenant_id, recorded_month, idempotency_key, producer_sequence
        )
        INSERT INTO audit_outbox_global_keys (
            audit_decision_event_id, tenant_id, decision_id, event_type,
            operation_kind, workload_instance_id, producer_sequence,
            idempotency_key, recorded_month, audit_outbox_id
        )
        SELECT
            audit.audit_decision_event_id, audit.tenant_id, audit.decision_id,
            'spendguard.audit.outcome', tx.operation_kind,
            'session-reservation-ledger', audit.producer_sequence,
            audit.idempotency_key, audit.recorded_month, audit.audit_outbox_id
        FROM audit
        CROSS JOIN tx
        "#,
    )
    .bind(seed_tx_id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("insert seed tx");

    sqlx::query(
        r#"
        INSERT INTO ledger_entries (
            ledger_entry_id, ledger_transaction_id, ledger_account_id,
            tenant_id, budget_id, window_instance_id, unit_id,
            direction, amount_atomic,
            pricing_version, price_snapshot_hash, fx_rate_version, unit_conversion_version,
            ledger_shard_id, ledger_sequence,
            effective_at, effective_month, recorded_at, recorded_month
        ) VALUES (
            $1, $2, $3, $5, $6, $7, $8,
            'debit', 200000,
            $9, decode($10, 'hex'), $11, $12,
            1, nextval_per_shard(1::smallint),
            clock_timestamp(), date_trunc('month', clock_timestamp())::date,
            clock_timestamp(), date_trunc('month', clock_timestamp())::date
        ), (
            $4, $2, $13, $5, $6, $7, $8,
            'credit', 200000,
            $9, decode($10, 'hex'), $11, $12,
            1, nextval_per_shard(1::smallint),
            clock_timestamp(), date_trunc('month', clock_timestamp())::date,
            clock_timestamp(), date_trunc('month', clock_timestamp())::date
        )
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(seed_tx_id)
    .bind(adjustment_account_id)
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(budget_id)
    .bind(window_instance_id)
    .bind(unit_id)
    .bind(&pricing.pricing_version)
    .bind(&pricing.price_snapshot_hash_hex)
    .bind(&pricing.fx_rate_version)
    .bind(&pricing.unit_conversion_version)
    .bind(available_account_id)
    .execute(pool)
    .await
    .expect("seed available balance");

    Fixture {
        tenant_id,
        budget_id,
        window_instance_id,
        unit_id,
        pricing,
    }
}

fn utc_now_seconds() -> DateTime<Utc> {
    Utc::now().with_nanosecond(0).expect("truncate nanos")
}

fn format_event_time(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
}

fn tuple_outcome(mut outcome: Value, f: &Fixture, event_time: Option<DateTime<Utc>>) -> Value {
    let object = outcome.as_object_mut().expect("object outcome");
    object.insert("tenant_id".into(), json!(f.tenant_id.to_string()));
    object.insert("budget_id".into(), json!(f.budget_id.to_string()));
    object.insert(
        "window_instance_id".into(),
        json!(f.window_instance_id.to_string()),
    );
    object.insert("unit".into(), json!({ "unit_id": f.unit_id.to_string() }));
    object.insert("unit_id".into(), json!(f.unit_id.to_string()));
    object.insert(
        "pricing_version".into(),
        json!(f.pricing.pricing_version.clone()),
    );
    object.insert(
        "price_snapshot_hash_hex".into(),
        json!(f.pricing.price_snapshot_hash_hex.clone()),
    );
    object.insert(
        "fx_rate_version".into(),
        json!(f.pricing.fx_rate_version.clone()),
    );
    object.insert(
        "unit_conversion_version".into(),
        json!(f.pricing.unit_conversion_version.clone()),
    );
    if let Some(event_time) = event_time {
        object.insert("event_time".into(), json!(format_event_time(event_time)));
    }
    outcome
}

fn test_audit_context(
    session_event_type: &str,
    session_reservation_id: Uuid,
    event_outcome: Value,
) -> Value {
    let decision_sequence = TEST_AUDIT_SEQUENCE.fetch_add(1, Ordering::SeqCst);
    let outcome_sequence = TEST_AUDIT_SEQUENCE.fetch_add(1, Ordering::SeqCst);
    let recorded_at = utc_now_seconds();
    let decision_id = Uuid::new_v4().to_string();

    let signed_part = |phase: &str, cloud_event_type: &str, producer_sequence: i64| {
        let data = json!({
            "event_outcome": event_outcome.clone(),
            "phase": phase,
            "session_event_type": session_event_type,
            "session_reservation_id": session_reservation_id.to_string(),
        });
        json!({
            "audit_event_id": Uuid::new_v4().to_string(),
            "audit_outbox_id": Uuid::new_v4().to_string(),
            "data_b64": general_purpose::STANDARD.encode(serde_json::to_vec(&data).unwrap()),
            "producer_sequence": producer_sequence,
            "signature_hex": "11".repeat(64),
            "type": cloud_event_type,
        })
    };

    json!({
        "decision": signed_part("decision", "spendguard.audit.decision", decision_sequence),
        "decision_id": decision_id,
        "outcome": signed_part("outcome", "spendguard.audit.outcome", outcome_sequence),
        "producer_id": "ledger:session-reservation-ledger",
        "recorded_at": format_event_time(recorded_at),
        "signing_key_id": "ed25519:test-session-reservations",
        "time_nanos": 0,
        "time_seconds": recorded_at.timestamp(),
    })
}

fn reserve_req(f: &Fixture, session_id: &str, amount: &str) -> ReserveSessionLedgerRequest {
    let session_reservation_id = Uuid::new_v4();
    let ttl_expires_at = format_event_time(utc_now_seconds() + chrono::Duration::seconds(60));
    let outcome = tuple_outcome(
        json!({
            "status": "accepted",
            "session_reservation_id": session_reservation_id.to_string(),
            "reserved_amount_atomic": amount,
            "committed_amount_atomic": "0",
            "remaining_amount_atomic": amount,
            "released_amount_atomic": "0",
            "ttl_expires_at": ttl_expires_at,
        }),
        f,
        None,
    );
    ReserveSessionLedgerRequest {
        tenant_id: f.tenant_id,
        budget_id: f.budget_id,
        window_instance_id: f.window_instance_id,
        unit_id: f.unit_id,
        pricing: f.pricing.clone(),
        session_id: session_id.into(),
        route: "livekit/agent".into(),
        estimated_amount_atomic: amount.into(),
        ttl_seconds: 60,
        idempotency_key: format!("{session_id}-reserve"),
        server_mint: Some(json!({
            "session_reservation_id": session_reservation_id.to_string(),
            "ttl_expires_at": ttl_expires_at,
        })),
        audit_context: Some(test_audit_context(
            "spendguard.audit.session.reserve",
            session_reservation_id,
            outcome,
        )),
    }
}

fn denied_reserve_req(f: &Fixture, session_id: &str, amount: &str) -> ReserveSessionLedgerRequest {
    let session_reservation_id = Uuid::new_v4();
    let ttl_expires_at = format_event_time(utc_now_seconds() + chrono::Duration::seconds(60));
    let audit_outcome = tuple_outcome(
        json!({
            "status": "denied",
            "reason": "INSUFFICIENT_AVAILABLE_BUDGET",
            "session_reservation_id": session_reservation_id.to_string(),
            "session_id": session_id,
            "route": "livekit/agent",
            "requested_amount_atomic": amount,
            "committed_amount_atomic": "0",
            "remaining_amount_atomic": "0",
            "released_amount_atomic": "0",
            "ttl_expires_at": ttl_expires_at,
        }),
        f,
        None,
    );
    ReserveSessionLedgerRequest {
        tenant_id: f.tenant_id,
        budget_id: f.budget_id,
        window_instance_id: f.window_instance_id,
        unit_id: f.unit_id,
        pricing: f.pricing.clone(),
        session_id: session_id.into(),
        route: "livekit/agent".into(),
        estimated_amount_atomic: amount.into(),
        ttl_seconds: 60,
        idempotency_key: format!("{session_id}-reserve"),
        server_mint: Some(json!({
            "session_reservation_id": session_reservation_id.to_string(),
            "ttl_expires_at": ttl_expires_at,
        })),
        audit_context: Some(test_audit_context(
            "spendguard.audit.session.denied",
            session_reservation_id,
            audit_outcome,
        )),
    }
}

fn commit_req(
    f: &Fixture,
    session_reservation_id: Uuid,
    streaming_commit_id: &str,
    amount: &str,
    committed_after: &str,
    remaining_after: &str,
) -> CommitSessionDeltaLedgerRequest {
    let event_time = utc_now_seconds();
    let outcome = tuple_outcome(
        json!({
            "status": "accepted",
            "session_reservation_id": session_reservation_id.to_string(),
            "streaming_commit_id": streaming_commit_id,
            "amount_atomic_delta": amount,
            "committed_amount_atomic": committed_after,
            "remaining_amount_atomic": remaining_after,
        }),
        f,
        Some(event_time),
    );
    CommitSessionDeltaLedgerRequest {
        session_reservation_id,
        streaming_commit_id: streaming_commit_id.into(),
        amount_atomic_delta: amount.into(),
        outcome: "estimated".into(),
        event_time,
        idempotency_key: format!("{streaming_commit_id}-idem"),
        tenant_id: Some(f.tenant_id),
        budget_id: Some(f.budget_id),
        window_instance_id: Some(f.window_instance_id),
        unit_id: Some(f.unit_id),
        pricing_version: Some(f.pricing.pricing_version.clone()),
        price_snapshot_hash_hex: Some(f.pricing.price_snapshot_hash_hex.clone()),
        fx_rate_version: Some(f.pricing.fx_rate_version.clone()),
        unit_conversion_version: Some(f.pricing.unit_conversion_version.clone()),
        audit_context: Some(test_audit_context(
            "spendguard.audit.session.commit_delta",
            session_reservation_id,
            outcome,
        )),
    }
}

fn denied_commit_req(
    f: &Fixture,
    session_reservation_id: Uuid,
    streaming_commit_id: &str,
    amount: &str,
    committed_after: &str,
    remaining_after: &str,
) -> CommitSessionDeltaLedgerRequest {
    let event_time = utc_now_seconds();
    let outcome = tuple_outcome(
        json!({
            "status": "denied",
            "reason": "OVERRUN_RESERVATION",
            "session_reservation_id": session_reservation_id.to_string(),
            "attempted_amount_atomic_delta": amount,
            "committed_amount_atomic": committed_after,
            "remaining_amount_atomic": remaining_after,
        }),
        f,
        Some(event_time),
    );
    CommitSessionDeltaLedgerRequest {
        session_reservation_id,
        streaming_commit_id: streaming_commit_id.into(),
        amount_atomic_delta: amount.into(),
        outcome: "estimated".into(),
        event_time,
        idempotency_key: format!("{streaming_commit_id}-idem"),
        tenant_id: Some(f.tenant_id),
        budget_id: Some(f.budget_id),
        window_instance_id: Some(f.window_instance_id),
        unit_id: Some(f.unit_id),
        pricing_version: Some(f.pricing.pricing_version.clone()),
        price_snapshot_hash_hex: Some(f.pricing.price_snapshot_hash_hex.clone()),
        fx_rate_version: Some(f.pricing.fx_rate_version.clone()),
        unit_conversion_version: Some(f.pricing.unit_conversion_version.clone()),
        audit_context: Some(test_audit_context(
            "spendguard.audit.session.denied",
            session_reservation_id,
            outcome,
        )),
    }
}

fn release_req(
    f: &Fixture,
    session_reservation_id: Uuid,
    key: &str,
    committed_amount: &str,
    released_amount: &str,
) -> ReleaseSessionLedgerRequest {
    let event_time = utc_now_seconds();
    let outcome = tuple_outcome(
        json!({
            "status": "accepted",
            "session_reservation_id": session_reservation_id.to_string(),
            "reason_code": "EXPLICIT",
            "session_status": "released",
            "released_this_call_atomic": released_amount,
            "released_amount_atomic": released_amount,
            "committed_amount_atomic": committed_amount,
            "remaining_amount_atomic": "0",
        }),
        f,
        Some(event_time),
    );
    ReleaseSessionLedgerRequest {
        session_reservation_id,
        reason_code: "EXPLICIT".into(),
        event_time,
        idempotency_key: key.into(),
        audit_context: Some(test_audit_context(
            "spendguard.audit.session.release",
            session_reservation_id,
            outcome,
        )),
    }
}

fn early_expire_req(
    f: &Fixture,
    session_reservation_id: Uuid,
    key: &str,
    ttl_expires_at: &str,
    committed_amount: &str,
    remaining_amount: &str,
) -> ExpireSessionLedgerRequest {
    let event_time = utc_now_seconds();
    let outcome = tuple_outcome(
        json!({
            "status": "denied",
            "reason": "SESSION_TTL_NOT_EXPIRED",
            "session_reservation_id": session_reservation_id.to_string(),
            "ttl_expires_at": ttl_expires_at,
            "committed_amount_atomic": committed_amount,
            "remaining_amount_atomic": remaining_amount,
        }),
        f,
        Some(event_time),
    );
    ExpireSessionLedgerRequest {
        session_reservation_id,
        event_time,
        idempotency_key: key.into(),
        audit_context: Some(test_audit_context(
            "spendguard.audit.session.denied",
            session_reservation_id,
            outcome,
        )),
    }
}

fn expire_req(
    f: &Fixture,
    session_reservation_id: Uuid,
    key: &str,
    committed_amount: &str,
    released_amount: &str,
) -> ExpireSessionLedgerRequest {
    let event_time = utc_now_seconds();
    let outcome = tuple_outcome(
        json!({
            "status": "accepted",
            "session_reservation_id": session_reservation_id.to_string(),
            "session_status": "expired",
            "released_this_call_atomic": released_amount,
            "released_amount_atomic": released_amount,
            "committed_amount_atomic": committed_amount,
            "remaining_amount_atomic": "0",
        }),
        f,
        Some(event_time),
    );
    ExpireSessionLedgerRequest {
        session_reservation_id,
        event_time,
        idempotency_key: key.into(),
        audit_context: Some(test_audit_context(
            "spendguard.audit.session.expired",
            session_reservation_id,
            outcome,
        )),
    }
}

fn json_str<'a>(value: &'a Value, field: &str) -> &'a str {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string field {field}: {value}"))
}

fn assert_sqlstate(err: SessionReservationError, code: &str) {
    match err {
        SessionReservationError::Db(sqlx::Error::Database(db_err)) => {
            assert_eq!(db_err.code().as_deref(), Some(code), "{db_err}");
        }
        other => panic!("expected sqlstate {code}, got {other:?}"),
    }
}

async fn account_balance(pool: &sqlx::PgPool, f: &Fixture, account_kind: &str) -> String {
    sqlx::query_scalar(
        r#"
        SELECT COALESCE(
            SUM(CASE le.direction
                WHEN 'credit' THEN le.amount_atomic
                WHEN 'debit' THEN -le.amount_atomic
            END),
            0
        )::TEXT
          FROM ledger_accounts la
          LEFT JOIN ledger_entries le
            ON le.ledger_account_id = la.ledger_account_id
         WHERE la.tenant_id = $1
           AND la.budget_id = $2
           AND la.window_instance_id = $3
           AND la.unit_id = $4
           AND la.account_kind = $5
        "#,
    )
    .bind(f.tenant_id)
    .bind(f.budget_id)
    .bind(f.window_instance_id)
    .bind(f.unit_id)
    .bind(account_kind)
    .fetch_one(pool)
    .await
    .expect("account balance")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_reservation_lifecycle_enforces_invariants() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let fixture = insert_fixture(&pool).await;

    let reserve = reserve_req(&fixture, "voice-session-1", "100000");
    let reserve_outcome = reserve_session(&pool, &reserve).await.expect("reserve");
    assert_eq!(json_str(&reserve_outcome, "status"), "accepted");
    assert_eq!(
        json_str(&reserve_outcome, "reserved_amount_atomic"),
        "100000"
    );
    assert_eq!(
        json_str(&reserve_outcome, "remaining_amount_atomic"),
        "100000"
    );
    assert!(reserve_outcome.get("ledger_transaction_id").is_some());
    let session_reservation_id =
        Uuid::parse_str(json_str(&reserve_outcome, "session_reservation_id")).unwrap();
    assert_eq!(
        account_balance(&pool, &fixture, "available_budget").await,
        "100000"
    );
    assert_eq!(
        account_balance(&pool, &fixture, "reserved_hold").await,
        "100000"
    );

    let (reserved, committed, released, status): (String, String, String, String) = sqlx::query_as(
        r#"
            SELECT reserved_amount_atomic::TEXT, committed_amount_atomic::TEXT,
                   released_amount_atomic::TEXT, status
              FROM session_reservations
             WHERE session_reservation_id = $1
            "#,
    )
    .bind(session_reservation_id)
    .fetch_one(&pool)
    .await
    .expect("reservation row");
    assert_eq!(
        (reserved, committed, released, status),
        ("100000".into(), "0".into(), "0".into(), "active".into(),)
    );

    let reserve_replay = reserve_session(&pool, &reserve)
        .await
        .expect("reserve replay");
    assert_eq!(reserve_replay, reserve_outcome);

    let mut conflicting_reserve = reserve.clone();
    conflicting_reserve.estimated_amount_atomic = "90000".into();
    let err = reserve_session(&pool, &conflicting_reserve)
        .await
        .expect_err("same reserve key with different payload conflicts");
    assert_sqlstate(err, "40P03");

    let denied_reserve = denied_reserve_req(&fixture, "voice-session-denied", "999999");
    let denied_reserve_outcome = reserve_session(&pool, &denied_reserve)
        .await
        .expect("reserve denial");
    assert_eq!(json_str(&denied_reserve_outcome, "status"), "denied");
    assert_eq!(
        json_str(&denied_reserve_outcome, "reason"),
        "INSUFFICIENT_AVAILABLE_BUDGET"
    );
    assert_eq!(
        json_str(&denied_reserve_outcome, "available_amount_atomic"),
        "100000"
    );
    assert!(denied_reserve_outcome
        .get("ledger_transaction_id")
        .is_some());
    assert_eq!(
        account_balance(&pool, &fixture, "available_budget").await,
        "100000"
    );
    assert_eq!(
        account_balance(&pool, &fixture, "reserved_hold").await,
        "100000"
    );

    let denied_reserve_replay = reserve_session(&pool, &denied_reserve)
        .await
        .expect("reserve denial replay");
    assert_eq!(denied_reserve_replay, denied_reserve_outcome);
    let denied_session_reservation_id =
        Uuid::parse_str(json_str(&denied_reserve_outcome, "session_reservation_id")).unwrap();
    let reserve_denied_audit_rows: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
          FROM audit_outbox ao
          CROSS JOIN LATERAL (
              SELECT convert_from(decode(ao.cloudevent_payload->>'data_b64', 'base64'), 'UTF8')::jsonb AS data
          ) decoded
         WHERE decoded.data->>'session_reservation_id' = $1
           AND decoded.data->>'session_event_type' = 'spendguard.audit.session.denied'
        "#,
    )
    .bind(denied_session_reservation_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("reserve denied audit rows");
    assert_eq!(reserve_denied_audit_rows, 2);
    let mut conflicting_denied_reserve = denied_reserve.clone();
    conflicting_denied_reserve.estimated_amount_atomic = "999998".into();
    let err = reserve_session(&pool, &conflicting_denied_reserve)
        .await
        .expect_err("same denied reserve key with different payload conflicts");
    assert_sqlstate(err, "40P03");

    let commit = commit_req(
        &fixture,
        session_reservation_id,
        "delta-1",
        "1000",
        "1000",
        "99000",
    );
    let commit_outcome = commit_session_delta(&pool, &commit)
        .await
        .expect("commit delta");
    assert_eq!(json_str(&commit_outcome, "status"), "accepted");
    assert_eq!(json_str(&commit_outcome, "committed_amount_atomic"), "1000");
    assert_eq!(
        json_str(&commit_outcome, "remaining_amount_atomic"),
        "99000"
    );
    assert!(commit_outcome.get("ledger_transaction_id").is_some());
    assert_eq!(
        account_balance(&pool, &fixture, "reserved_hold").await,
        "99000"
    );
    assert_eq!(
        account_balance(&pool, &fixture, "committed_spend").await,
        "1000"
    );

    let commit_replay = commit_session_delta(&pool, &commit)
        .await
        .expect("commit replay");
    assert_eq!(commit_replay, commit_outcome);
    let committed_after_replay: String = sqlx::query_scalar(
        "SELECT committed_amount_atomic::TEXT FROM session_reservations WHERE session_reservation_id = $1",
    )
    .bind(session_reservation_id)
    .fetch_one(&pool)
    .await
    .expect("committed after replay");
    assert_eq!(committed_after_replay, "1000");

    let mut conflicting_commit = commit.clone();
    conflicting_commit.amount_atomic_delta = "2000".into();
    let err = commit_session_delta(&pool, &conflicting_commit)
        .await
        .expect_err("same streaming commit id with different payload conflicts");
    assert_sqlstate(err, "40P03");

    let over_budget = denied_commit_req(
        &fixture,
        session_reservation_id,
        "delta-over",
        "100000",
        "1000",
        "99000",
    );
    let over_budget_outcome = commit_session_delta(&pool, &over_budget)
        .await
        .expect("over-budget denial outcome");
    assert_eq!(json_str(&over_budget_outcome, "status"), "denied");
    assert_eq!(
        json_str(&over_budget_outcome, "reason"),
        "OVERRUN_RESERVATION"
    );
    let committed_after_denial: String = sqlx::query_scalar(
        "SELECT committed_amount_atomic::TEXT FROM session_reservations WHERE session_reservation_id = $1",
    )
    .bind(session_reservation_id)
    .fetch_one(&pool)
    .await
    .expect("committed after denial");
    assert_eq!(committed_after_denial, "1000");

    let denied_events: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM session_reservation_events
         WHERE session_reservation_id = $1
           AND event_type = 'spendguard.audit.session.denied'
        "#,
    )
    .bind(session_reservation_id)
    .fetch_one(&pool)
    .await
    .expect("denied event count");
    assert_eq!(denied_events, 1);

    let mut mismatched_tuple = commit_req(
        &fixture,
        session_reservation_id,
        "delta-mismatch",
        "1000",
        "2000",
        "98000",
    );
    mismatched_tuple.unit_id = Some(Uuid::new_v4());
    let err = commit_session_delta(&pool, &mismatched_tuple)
        .await
        .expect_err("tuple mismatch rejects commit");
    assert_sqlstate(err, "P0001");

    let release = release_req(
        &fixture,
        session_reservation_id,
        "release-1",
        "1000",
        "99000",
    );
    let release_outcome = release_session(&pool, &release).await.expect("release");
    assert_eq!(json_str(&release_outcome, "status"), "accepted");
    assert_eq!(
        json_str(&release_outcome, "released_this_call_atomic"),
        "99000"
    );
    assert_eq!(
        json_str(&release_outcome, "released_amount_atomic"),
        "99000"
    );
    assert_eq!(json_str(&release_outcome, "remaining_amount_atomic"), "0");
    assert!(release_outcome.get("ledger_transaction_id").is_some());
    assert_eq!(
        account_balance(&pool, &fixture, "available_budget").await,
        "199000"
    );
    assert_eq!(account_balance(&pool, &fixture, "reserved_hold").await, "0");
    assert_eq!(
        account_balance(&pool, &fixture, "committed_spend").await,
        "1000"
    );

    let release_replay = release_session(&pool, &release)
        .await
        .expect("release replay");
    assert_eq!(release_replay, release_outcome);

    let release_after_settled = release_session(
        &pool,
        &release_req(&fixture, session_reservation_id, "release-2", "1000", "0"),
    )
    .await
    .expect("release after settled");
    assert_eq!(
        json_str(&release_after_settled, "released_this_call_atomic"),
        "0"
    );

    let expiry_reserve = reserve_session(
        &pool,
        &reserve_req(&fixture, "voice-session-expire", "5000"),
    )
    .await
    .expect("reserve expiring session");
    let expiry_session_id =
        Uuid::parse_str(json_str(&expiry_reserve, "session_reservation_id")).unwrap();
    let expiry_commit = commit_req(
        &fixture,
        expiry_session_id,
        "delta-expire",
        "2000",
        "2000",
        "3000",
    );
    commit_session_delta(&pool, &expiry_commit)
        .await
        .expect("pre-expiry commit");

    let early_expire = early_expire_req(
        &fixture,
        expiry_session_id,
        "expire-early",
        json_str(&expiry_reserve, "ttl_expires_at"),
        "2000",
        "3000",
    );
    let early_expire_outcome = expire_session(&pool, &early_expire)
        .await
        .expect("early expire denial");
    assert_eq!(json_str(&early_expire_outcome, "status"), "denied");
    assert_eq!(
        json_str(&early_expire_outcome, "reason"),
        "SESSION_TTL_NOT_EXPIRED"
    );
    let early_expire_replay = expire_session(&pool, &early_expire)
        .await
        .expect("early expire replay");
    assert_eq!(early_expire_replay, early_expire_outcome);
    let mut conflicting_early_expire = early_expire.clone();
    conflicting_early_expire.event_time =
        conflicting_early_expire.event_time + chrono::Duration::seconds(1);
    let err = expire_session(&pool, &conflicting_early_expire)
        .await
        .expect_err("early expire same key with different payload conflicts");
    assert_sqlstate(err, "40P03");

    sqlx::query(
        "UPDATE session_reservations SET ttl_expires_at = clock_timestamp() - INTERVAL '1 second' WHERE session_reservation_id = $1",
    )
    .bind(expiry_session_id)
    .execute(&pool)
    .await
    .expect("force ttl expiry");

    let expire_outcome = expire_session(
        &pool,
        &expire_req(&fixture, expiry_session_id, "expire-1", "2000", "3000"),
    )
    .await
    .expect("expire session");
    assert_eq!(json_str(&expire_outcome, "status"), "accepted");
    assert_eq!(json_str(&expire_outcome, "session_status"), "expired");
    assert_eq!(
        json_str(&expire_outcome, "released_this_call_atomic"),
        "3000"
    );
    assert_eq!(json_str(&expire_outcome, "remaining_amount_atomic"), "0");
    assert!(expire_outcome.get("ledger_transaction_id").is_some());

    let unaudited_tx_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
          FROM ledger_transactions lt
         WHERE NOT EXISTS (
             SELECT 1
               FROM audit_outbox ao
              WHERE ao.ledger_transaction_id = lt.ledger_transaction_id
         )
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("unaudited tx count");
    assert_eq!(unaudited_tx_count, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_idempotent_replays_return_original_outcome() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let fixture = insert_fixture(&pool).await;

    let reserve = reserve_req(&fixture, "voice-session-race", "10000");
    let (a, b) = tokio::join!(
        reserve_session(&pool, &reserve),
        reserve_session(&pool, &reserve)
    );
    let reserve_a = a.expect("reserve a");
    let reserve_b = b.expect("reserve b");
    assert_eq!(reserve_a, reserve_b);
    let session_reservation_id =
        Uuid::parse_str(json_str(&reserve_a, "session_reservation_id")).unwrap();

    let reserve_entry_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ledger_entries WHERE reservation_id = $1 AND commit_event_kind = 'session_reserve'",
    )
    .bind(session_reservation_id)
    .fetch_one(&pool)
    .await
    .expect("reserve entry count");
    assert_eq!(reserve_entry_count, 2);

    let release = release_req(
        &fixture,
        session_reservation_id,
        "release-race",
        "0",
        "10000",
    );
    let (a, b) = tokio::join!(
        release_session(&pool, &release),
        release_session(&pool, &release)
    );
    let release_a = a.expect("release a");
    let release_b = b.expect("release b");
    assert_eq!(release_a, release_b);
    let release_event_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM session_reservation_events
         WHERE session_reservation_id = $1
           AND event_type = 'spendguard.audit.session.release'
           AND idempotency_key = 'release-race'
        "#,
    )
    .bind(session_reservation_id)
    .fetch_one(&pool)
    .await
    .expect("release event count");
    assert_eq!(release_event_count, 1);

    let expiry_reserve = reserve_session(
        &pool,
        &reserve_req(&fixture, "voice-session-expire-race", "9000"),
    )
    .await
    .expect("reserve expiring race session");
    let expiry_session_id =
        Uuid::parse_str(json_str(&expiry_reserve, "session_reservation_id")).unwrap();
    sqlx::query(
        "UPDATE session_reservations SET ttl_expires_at = clock_timestamp() - INTERVAL '1 second' WHERE session_reservation_id = $1",
    )
    .bind(expiry_session_id)
    .execute(&pool)
    .await
    .expect("force ttl expiry");

    let expire = expire_req(&fixture, expiry_session_id, "expire-race", "0", "9000");
    let (a, b) = tokio::join!(
        expire_session(&pool, &expire),
        expire_session(&pool, &expire)
    );
    let expire_a = a.expect("expire a");
    let expire_b = b.expect("expire b");
    assert_eq!(expire_a, expire_b);
    let expire_event_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM session_reservation_events
         WHERE session_reservation_id = $1
           AND event_type = 'spendguard.audit.session.expired'
           AND idempotency_key = 'expire-race'
        "#,
    )
    .bind(expiry_session_id)
    .fetch_one(&pool)
    .await
    .expect("expire event count");
    assert_eq!(expire_event_count, 1);
}
