//! Regression test for the Critical "no BUDGET_EXHAUSTED hard cap" finding.
//!
//! The main reserve path (`post_ledger_transaction`) must reject a reserve
//! that drives the cumulative `available_budget` account balance below zero —
//! even when each individual reserve fits, but two reserves JOINTLY exceed the
//! funded budget. Migration 0063 adds the non-negative floor; this test pins
//! it against the REAL migration chain (0000-0012 + 0063) on a throwaway
//! Postgres container so the funding model and credit-positive orientation are
//! exercised exactly as production does.
//!
//! Funding model (verified): budgets are pre-funded by an operator opening
//! deposit that CREDITS available_budget (see
//! deploy/demo/init/migrations/30_seed_demo_state.sh). The floor therefore
//! never fires on a normal under-budget reserve; it only fires when reserves
//! would exceed the funded ceiling.

use std::time::Duration;

use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

const CORE_MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0000_extensions.sql"),
    include_str!("../migrations/0001_ledger_units.sql"),
    include_str!("../migrations/0002_ledger_shards.sql"),
    include_str!("../migrations/0003_budget_window_instances.sql"),
    include_str!("../migrations/0004_ledger_accounts.sql"),
    include_str!("../migrations/0005_pricing_snapshots.sql"),
    include_str!("../migrations/0006_fencing_scopes.sql"),
    include_str!("../migrations/0007_ledger_transactions.sql"),
    include_str!("../migrations/0008_ledger_entries.sql"),
    include_str!("../migrations/0009_audit_outbox.sql"),
    include_str!("../migrations/0010_projections.sql"),
    include_str!("../migrations/0011_immutability_triggers.sql"),
    include_str!("../migrations/0012_post_ledger_transaction.sql"),
    include_str!("../migrations/0063_post_ledger_transaction_budget_floor.sql"),
];

struct Fixture {
    tenant_id: Uuid,
    budget_id: Uuid,
    window_instance_id: Uuid,
    unit_id: Uuid,
    budget_window_scope_id: Uuid,
    pricing_version: String,
    price_snapshot_hash_hex: String,
    fx_rate_version: String,
    unit_conversion_version: String,
}

async fn setup_pool() -> Option<sqlx::PgPool> {
    // Production targets Postgres 16; the migrations use `NULLS NOT DISTINCT`
    // (Postgres 15+), so pin the container tag rather than the module default
    // (11-alpine), which would fail to apply 0001.
    let container = match Postgres::default().with_tag("16-alpine").start().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[budget-floor-test] testcontainers Postgres not available: {e}");
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

    for sql in CORE_MIGRATIONS {
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
    let pricing_version = "price-v1".to_string();
    let price_snapshot_hash_hex =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
    let fx_rate_version = "fx-v1".to_string();
    let unit_conversion_version = "uc-v1".to_string();

    // Shard 1 + its sequence allocator (the migrations create the tables but
    // do not seed rows; nextval_per_shard requires the allocator to exist).
    sqlx::query(
        "INSERT INTO ledger_shards (ledger_shard_id, shard_generation, status) \
         VALUES (1, 1, 'active') ON CONFLICT DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("seed shard");
    sqlx::query(
        "INSERT INTO ledger_sequence_allocators (ledger_shard_id, last_sequence) \
         VALUES (1, 0) ON CONFLICT DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("seed sequence allocator");

    sqlx::query(
        r#"
        INSERT INTO ledger_units (
            unit_id, tenant_id, unit_kind, unit_name, scale, rounding_mode,
            token_kind, model_family
        ) VALUES ($1, $2, 'token', 'demo-token', 0, 'half_even', 'llm_token', 'budget-floor-test')
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
        ) VALUES ($1, decode($2, 'hex'), $3, $4, '{}'::jsonb, decode('00', 'hex'), 'test-key', 'budget-floor-test')
        "#,
    )
    .bind(&pricing_version)
    .bind(&price_snapshot_hash_hex)
    .bind(&fx_rate_version)
    .bind(&unit_conversion_version)
    .execute(pool)
    .await
    .expect("insert pricing");

    for kind in [
        "available_budget",
        "reserved_hold",
        "committed_spend",
        "adjustment",
    ] {
        sqlx::query(
            r#"
            INSERT INTO ledger_accounts (
                ledger_account_id, tenant_id, budget_id, window_instance_id,
                account_kind, unit_id
            ) VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(budget_id)
        .bind(window_instance_id)
        .bind(kind)
        .bind(unit_id)
        .execute(pool)
        .await
        .expect("insert account");
    }

    // control_plane_writer scope for the funding adjustment.
    let control_scope_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO fencing_scopes (
            fencing_scope_id, scope_type, tenant_id, workload_kind,
            current_epoch, active_owner_instance_id, ttl_expires_at,
            epoch_source_authority
        ) VALUES ($1, 'control_plane_writer', $2, 'seed-runner',
                  1, 'seed-runner', 'infinity'::timestamptz, 'ledger_lease')
        "#,
    )
    .bind(control_scope_id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("insert control-plane scope");

    // budget_window scope for reserve ops.
    let budget_window_scope_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO fencing_scopes (
            fencing_scope_id, scope_type, tenant_id, budget_id,
            window_instance_id, current_epoch, active_owner_instance_id,
            ttl_expires_at, epoch_source_authority
        ) VALUES ($1, 'budget_window', $2, $3, $4,
                  1, 'reserve-runner', 'infinity'::timestamptz, 'ledger_lease')
        "#,
    )
    .bind(budget_window_scope_id)
    .bind(tenant_id)
    .bind(budget_id)
    .bind(window_instance_id)
    .execute(pool)
    .await
    .expect("insert budget-window scope");

    let fixture = Fixture {
        tenant_id,
        budget_id,
        window_instance_id,
        unit_id,
        budget_window_scope_id,
        pricing_version,
        price_snapshot_hash_hex,
        fx_rate_version,
        unit_conversion_version,
    };

    // Operator opening deposit: credit available_budget by `amount`, debit
    // adjustment by `amount` — routed through the SP exactly like production.
    fund_available_budget(pool, &fixture, control_scope_id, 100).await;

    fixture
}

/// Credit `amount` into available_budget (debit adjustment) via the SP.
async fn fund_available_budget(
    pool: &sqlx::PgPool,
    f: &Fixture,
    control_scope_id: Uuid,
    amount: i64,
) {
    let key = format!("seed-deposit-{amount}-{}", Uuid::new_v4());
    let entries = json!([
        entry(f, "available_budget", "credit", amount),
        entry(f, "adjustment", "debit", amount),
    ]);
    let tx_id = call_post_ledger_transaction(
        pool,
        f,
        "adjustment",
        control_scope_id,
        &key,
        entries,
    )
    .await
    .expect("funding deposit should succeed");
    assert_ne!(tx_id, Uuid::nil());
}

/// Issue a reserve for `amount` via the SP; returns Ok(tx_id) or the SQL error.
async fn reserve(
    pool: &sqlx::PgPool,
    f: &Fixture,
    amount: i64,
    label: &str,
) -> Result<Uuid, sqlx::Error> {
    let key = format!("reserve-{label}");
    let entries = json!([
        entry(f, "available_budget", "debit", amount),
        entry(f, "reserved_hold", "credit", amount),
    ]);
    call_post_ledger_transaction(
        pool,
        f,
        "reserve",
        f.budget_window_scope_id,
        &key,
        entries,
    )
    .await
}

fn entry(f: &Fixture, account_kind: &str, direction: &str, amount: i64) -> Value {
    json!({
        "ledger_entry_id":   Uuid::new_v4().to_string(),
        "budget_id":         f.budget_id.to_string(),
        "window_instance_id": f.window_instance_id.to_string(),
        "unit_id":           f.unit_id.to_string(),
        "account_kind":      account_kind,
        "direction":         direction,
        "amount_atomic":     amount.to_string(),
        "ledger_shard_id":   1,
        "pricing_version":         f.pricing_version,
        "price_snapshot_hash_hex": f.price_snapshot_hash_hex,
        "fx_rate_version":         f.fx_rate_version,
        "unit_conversion_version": f.unit_conversion_version,
    })
}

async fn call_post_ledger_transaction(
    pool: &sqlx::PgPool,
    f: &Fixture,
    operation_kind: &str,
    scope_id: Uuid,
    idempotency_key: &str,
    entries: Value,
) -> Result<Uuid, sqlx::Error> {
    let tx_id = Uuid::now_v7();
    let decision_id = Uuid::new_v4();
    let audit_event_id = Uuid::new_v4();
    let audit_outbox_id = Uuid::new_v4();

    let transaction = json!({
        "tenant_id":                f.tenant_id.to_string(),
        "operation_kind":           operation_kind,
        "idempotency_key":          idempotency_key,
        "request_hash_hex":         hex_sha256(idempotency_key),
        "decision_id":              decision_id.to_string(),
        "audit_decision_event_id":  audit_event_id.to_string(),
        "fencing_scope_id":         scope_id.to_string(),
        "fencing_epoch":            1,
        "workload_instance_id":     workload_for(operation_kind),
        "effective_at":             chrono::Utc::now().to_rfc3339(),
        "ledger_transaction_id":    tx_id.to_string(),
        "minimal_replay_response":  json!({}),
    });

    let audit_outbox = json!({
        "audit_outbox_id":                  audit_outbox_id.to_string(),
        "event_type":                       "spendguard.audit.decision",
        "cloudevent_payload":               json!({
            "specversion": "1.0",
            "type":        "spendguard.audit.decision",
            "id":          audit_event_id.to_string(),
            "source":      workload_for(operation_kind),
            "tenantid":    f.tenant_id.to_string(),
        }),
        "cloudevent_payload_signature_hex": "",
        "producer_sequence":                next_seq(),
    });

    let row = sqlx::query(
        "SELECT post_ledger_transaction($1::JSONB, $2::JSONB, NULL::JSONB, $3::JSONB, NULL)",
    )
    .bind(transaction)
    .bind(entries)
    .bind(audit_outbox)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<Uuid, _>(0))
}

fn workload_for(operation_kind: &str) -> &'static str {
    match operation_kind {
        "reserve" => "reserve-runner",
        _ => "seed-runner",
    }
}

fn hex_sha256(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    hex::encode(h.finalize())
}

fn next_seq() -> i64 {
    use std::sync::atomic::{AtomicI64, Ordering};
    static SEQ: AtomicI64 = AtomicI64::new(1);
    SEQ.fetch_add(1, Ordering::SeqCst)
}

#[tokio::test]
async fn jointly_over_budget_reserves_are_denied_by_ledger_hard_cap() {
    let Some(pool) = setup_pool().await else {
        eprintln!("[budget-floor-test] skipped: no Postgres container");
        return;
    };

    let f = insert_fixture(&pool).await;
    // Funded ceiling = 100 (operator deposit credited available_budget 100).

    // First reserve of 60 fits comfortably (post-balance = 40).
    let first = reserve(&pool, &f, 60, "first").await;
    assert!(
        first.is_ok(),
        "first reserve of 60 against a funded budget of 100 must succeed, got {:?}",
        first.err()
    );

    // Second reserve of 60 individually fits the original ceiling, but
    // JOINTLY (60 + 60 = 120 > 100) exceeds it. The ledger hard cap MUST
    // deny it — this is the fail-open the finding describes.
    let second = reserve(&pool, &f, 60, "second").await;
    let err = second.expect_err(
        "second reserve of 60 must be DENIED: 60 + 60 > funded budget of 100",
    );
    let db_err = err
        .as_database_error()
        .expect("BUDGET_EXHAUSTED should surface as a database error");
    assert!(
        db_err.message().contains("BUDGET_EXHAUSTED"),
        "expected BUDGET_EXHAUSTED, got: {}",
        db_err.message()
    );

    // The denied reserve must NOT have moved the ledger: available_budget
    // balance stays at 40 (100 funded - 60 first reserve), proving the
    // second reserve rolled back atomically and did not over-debit.
    let balance: String = sqlx::query(
        "SELECT COALESCE(SUM(CASE direction WHEN 'credit' THEN amount_atomic \
                                            WHEN 'debit'  THEN -amount_atomic END), 0)::TEXT \
           FROM ledger_entries le \
           JOIN ledger_accounts la ON la.ledger_account_id = le.ledger_account_id \
          WHERE la.account_kind = 'available_budget' AND la.tenant_id = $1",
    )
    .bind(f.tenant_id)
    .fetch_one(&pool)
    .await
    .expect("balance query")
    .get(0);
    assert_eq!(
        balance, "40",
        "available_budget must remain 40 (100 funded - 60 reserved); denied reserve must not have committed"
    );
}

#[tokio::test]
async fn under_budget_reserve_is_allowed() {
    let Some(pool) = setup_pool().await else {
        eprintln!("[budget-floor-test] skipped: no Postgres container");
        return;
    };

    let f = insert_fixture(&pool).await;
    // A single reserve that fits the funded ceiling (100) must NOT trip the
    // floor — the floor must only fire on genuine over-spend.
    let ok = reserve(&pool, &f, 100, "exact").await;
    assert!(
        ok.is_ok(),
        "a reserve that exactly drains the funded budget (post-balance 0) must succeed, got {:?}",
        ok.err()
    );
}
