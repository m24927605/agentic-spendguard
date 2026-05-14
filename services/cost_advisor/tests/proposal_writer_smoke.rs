//! CA-P3 integration smoke for the proposal writer.
//!
//! Requires a Postgres instance with migrations 0000→0044 applied to
//! a `spendguard_ledger` DB reachable at the URL in
//! `SPENDGUARD_TEST_LEDGER_URL` (default
//! `postgres://postgres:test@localhost:25440/spendguard_ledger`).
//!
//! Skipped silently when the DB is unreachable so unit-test runs on
//! a bare workstation don't fail.

use serde_json::json;
use spendguard_cost_advisor::proposal_writer::{
    self, ProposalConfig, ProposalError, ProposalOutcome,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn ledger_url() -> String {
    std::env::var("SPENDGUARD_TEST_LEDGER_URL")
        .unwrap_or_else(|_| "postgres://postgres:test@localhost:25440/spendguard_ledger".to_string())
}

async fn try_connect() -> Option<sqlx::PgPool> {
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(2))
        .connect(&ledger_url())
        .await
        .ok()
}

async fn ensure_partition(pool: &sqlx::PgPool) {
    sqlx::query("SELECT cost_findings_ensure_next_month_partition()")
        .execute(pool)
        .await
        .expect("ensure partition");
}

async fn insert_finding(pool: &sqlx::PgPool, tenant: Uuid, finding_id: Uuid, fp_seed: char) {
    sqlx::query(
        r#"
        SELECT cost_findings_upsert(
            $1::uuid, $2::char(64), $3::uuid, now(),
            'r', 1, 'detected_waste', 'critical', 0.85::numeric,
            'a', 'r', 'b', '{}'::jsonb, 100::bigint, ARRAY[]::uuid[]
        )"#,
    )
    .bind(finding_id)
    .bind(fp_seed.to_string().repeat(64))
    .bind(tenant)
    .execute(pool)
    .await
    .expect("upsert finding");
}

#[tokio::test]
async fn proposal_writer_round_trip() {
    let Some(pool) = try_connect().await else {
        eprintln!("[skip] no DB reachable at {}", ledger_url());
        return;
    };
    ensure_partition(&pool).await;

    let tenant = Uuid::new_v4();
    let finding_id = Uuid::new_v4();
    insert_finding(&pool, tenant, finding_id, 'a').await;

    let patch = json!([
        {"op":"replace","path":"/spec/rules/0/then/decision","value":"REQUIRE_APPROVAL"}
    ]);
    let cfg = ProposalConfig::default();

    // TEST 1: first write → Inserted
    let r1 = proposal_writer::write_proposal(&pool, tenant, finding_id, 1, &patch, &cfg)
        .await
        .expect("first write_proposal");
    let approval_id = match r1 {
        ProposalOutcome::Inserted { approval_id, .. } => approval_id,
        other => panic!("expected Inserted, got {:?}", other),
    };

    // TEST 2: second write same finding → AlreadyExists
    let r2 = proposal_writer::write_proposal(&pool, tenant, finding_id, 1, &patch, &cfg)
        .await
        .expect("second write_proposal");
    assert!(
        matches!(r2, ProposalOutcome::AlreadyExists { .. }),
        "expected AlreadyExists on re-fire, got {:?}",
        r2
    );

    // TEST 3: different rule_version → different decision_id → new row
    let r3 = proposal_writer::write_proposal(&pool, tenant, finding_id, 2, &patch, &cfg)
        .await
        .expect("rule_version=2 write");
    let approval_id_v2 = match r3 {
        ProposalOutcome::Inserted { approval_id, .. } => approval_id,
        other => panic!("expected Inserted for rule_version=2, got {:?}", other),
    };
    assert_ne!(approval_id, approval_id_v2);

    // TEST 4: Rust validator rejects disallowed patch BEFORE DB call
    let bad_patch = json!([{"op":"replace","path":"/metadata/owner_team","value":"hack"}]);
    let r4 = proposal_writer::write_proposal(&pool, tenant, finding_id, 3, &bad_patch, &cfg).await;
    assert!(
        matches!(r4, Err(ProposalError::PatchInvalid(_))),
        "expected PatchInvalid, got {:?}",
        r4
    );
}

#[tokio::test]
async fn db_check_rejects_bypass() {
    let Some(pool) = try_connect().await else {
        eprintln!("[skip] no DB reachable at {}", ledger_url());
        return;
    };
    ensure_partition(&pool).await;

    let tenant = Uuid::new_v4();
    let finding_id = Uuid::new_v4();
    insert_finding(&pool, tenant, finding_id, 'b').await;

    // Bypass the Rust validator and call the SQL directly with a bad
    // patch. The DB-side CHECK constraint MUST reject it.
    let bad_patch = serde_json::json!([
        {"op":"add","path":"/spec/budgets/0/limit_amount_atomic","value":"99999"}
    ]);
    let result = sqlx::query(
        r#"
        INSERT INTO approval_requests (
            approval_id, tenant_id, decision_id, state,
            proposal_source, proposed_dsl_patch, proposing_finding_id,
            ttl_expires_at, created_at,
            approver_policy, requested_effect, decision_context
        ) VALUES (
            gen_random_uuid(), $1, $2, 'pending',
            'cost_advisor', $3::jsonb, $4,
            now() + interval '1 day', now(),
            '{}'::jsonb, '{}'::jsonb, '{}'::jsonb
        )
        "#,
    )
    .bind(tenant)
    .bind(Uuid::new_v4())
    .bind(&bad_patch)
    .bind(finding_id)
    .execute(&pool)
    .await;

    let Err(sqlx::Error::Database(db_err)) = result else {
        panic!(
            "expected DB CHECK violation; got {:?}",
            result.map(|_| "ok")
        );
    };
    let msg = db_err.message();
    assert!(
        msg.contains("approval_requests_cost_advisor_patch_allowlist")
            || msg.contains("check constraint"),
        "expected allowlist CHECK violation, got: {}",
        msg
    );
}

#[tokio::test]
async fn test_op_pinning_round_trip() {
    // CA-P3.1: verify the 2-op test+replace patch shape goes through
    // the SECURITY DEFINER SP + CHECK constraint cleanly.
    let Some(pool) = try_connect().await else {
        eprintln!("[skip] no DB reachable at {}", ledger_url());
        return;
    };
    ensure_partition(&pool).await;

    let tenant = Uuid::new_v4();
    let finding_id = Uuid::new_v4();
    insert_finding(&pool, tenant, finding_id, 'p').await;

    // 2-op test+replace patch — CA-P3.1 happy path.
    let patch = json!([
        {"op":"test","path":"/spec/budgets/0/id","value":"a1b2c3d4-e5f6-7890-abcd-ef0123456789"},
        {"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":60}
    ]);
    let cfg = ProposalConfig::default();

    let outcome =
        proposal_writer::write_proposal(&pool, tenant, finding_id, 1, &patch, &cfg)
            .await
            .expect("write proposal with test+replace patch");
    assert!(
        matches!(outcome, ProposalOutcome::Inserted { .. }),
        "expected Inserted, got {:?}",
        outcome
    );

    // Verify the stored patch round-trips.
    let stored: (serde_json::Value,) =
        sqlx::query_as("SELECT proposed_dsl_patch FROM approval_requests WHERE proposing_finding_id = $1")
            .bind(finding_id)
            .fetch_one(&pool)
            .await
            .expect("fetch stored patch");
    assert_eq!(stored.0, patch);
}

#[tokio::test]
async fn db_validator_value_schema_edges() {
    // Codex CA-P3 r2 P2 explicit-DB-test request: verify the
    // SQL-side cost_advisor_validate_proposed_dsl_patch agrees with
    // the Rust validator on edge cases.
    let Some(pool) = try_connect().await else {
        eprintln!("[skip] no DB reachable at {}", ledger_url());
        return;
    };

    async fn check(pool: &sqlx::PgPool, patch: serde_json::Value, expect_valid: bool, label: &str) {
        let row: (bool,) = sqlx::query_as("SELECT cost_advisor_validate_proposed_dsl_patch($1)")
            .bind(&patch)
            .fetch_one(pool)
            .await
            .expect(label);
        assert_eq!(row.0, expect_valid, "{}: patch={}", label, patch);
    }

    // Helper: build a 2-op patch with the test pin already in place,
    // so we can probe the value-schema gate (not the pinning gate).
    fn pinned_budget_replace(leaf: &str, value: serde_json::Value) -> serde_json::Value {
        json!([
            {"op":"test","path":"/spec/budgets/0/id","value":"a1b2c3d4-e5f6-7890-abcd-ef0123456789"},
            {"op":"replace","path":format!("/spec/budgets/0/{}", leaf),"value":value}
        ])
    }

    // TTL value cases (all pinned to isolate the value-schema gate).
    check(&pool, pinned_budget_replace("reservation_ttl_seconds", json!(1.5)), false,
        "ttl decimal 1.5 rejected").await;
    check(&pool, pinned_budget_replace("reservation_ttl_seconds", json!("60")), false,
        "ttl string \"60\" rejected (must be JSON number)").await;
    check(&pool, pinned_budget_replace("reservation_ttl_seconds", json!(-1)), false,
        "ttl -1 rejected (out of range)").await;
    check(&pool, pinned_budget_replace("reservation_ttl_seconds", json!(86401)), false,
        "ttl 86401 rejected (out of range)").await;
    // JSON Number with scientific notation: serde_json normalizes
    // `1e2` to `100.0` on the wire; PG's jsonb stores it as a
    // numeric and the `#>> '{}'` text extraction returns `"100.0"`,
    // which fails `::BIGINT` cast. Validator rejects.
    check(&pool, pinned_budget_replace("reservation_ttl_seconds", json!(1e2)), false,
        "ttl 1e2 rejected (normalized to 100.0 by serde_json)").await;
    check(&pool, pinned_budget_replace("reservation_ttl_seconds", json!(100)), true,
        "ttl 100 accepted (plain integer)").await;

    // Atomic amount: must be string of digits.
    check(&pool, pinned_budget_replace("limit_amount_atomic", json!(100)), false,
        "limit_amount_atomic number 100 rejected (must be string)").await;
    check(&pool, pinned_budget_replace("limit_amount_atomic", json!("1.5")), false,
        "limit_amount_atomic 1.5 rejected (must be digit-only)").await;
    check(&pool, pinned_budget_replace("limit_amount_atomic", json!("")), false,
        "limit_amount_atomic empty string rejected").await;

    // Same-index pinning invariant: budget replace WITHOUT a
    // preceding test op MUST fail (codex CA-P3.1 r1 P2).
    check(&pool,
        json!([{"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":60}]),
        false,
        "budget replace without test pin rejected (same-index invariant)",
    ).await;

    // CA-P3.1 r1 P1 regression: explicit DB-side rejection of the
    // old (pre-CA-P3.1) path shapes. The Rust validator has these
    // covered; mirror them at the SQL gate so the authoritative
    // layer is regression-protected.
    check(&pool,
        json!([{"op":"replace","path":"/budgets/0/reservation_ttl_seconds","value":60}]),
        false,
        "old shape /budgets/0/... (no /spec/) rejected",
    ).await;
    check(&pool,
        json!([{"op":"test","path":"/spec/budgets/0/budget_id","value":"a1b2c3d4-e5f6-7890-abcd-ef0123456789"}]),
        false,
        "old field name budget_id rejected (real field is `id`)",
    ).await;

    // Pinned at index 0 but replace targets index 1 — wrong-index
    // mismatch MUST fail.
    check(&pool,
        json!([
            {"op":"test","path":"/spec/budgets/0/id","value":"a1b2c3d4-e5f6-7890-abcd-ef0123456789"},
            {"op":"replace","path":"/spec/budgets/1/reservation_ttl_seconds","value":60}
        ]),
        false,
        "budget replace at index 1 with test at index 0 rejected",
    ).await;

    // Decision: must be enum.
    check(
        &pool,
        json!([{"op":"replace","path":"/spec/rules/0/then/decision","value":"WAT"}]),
        false,
        "decision WAT rejected",
    )
    .await;
    check(
        &pool,
        json!([{"op":"replace","path":"/spec/rules/0/then/decision","value":null}]),
        false,
        "decision null rejected",
    )
    .await;
    check(
        &pool,
        json!([{"op":"replace","path":"/spec/rules/0/then/decision","value":"STOP"}]),
        true,
        "decision STOP accepted",
    )
    .await;

    // Leading zero in array index — RFC 6901 rejects.
    check(
        &pool,
        json!([{"op":"replace","path":"/spec/rules/01/then/decision","value":"STOP"}]),
        false,
        "leading-zero index 01 rejected",
    )
    .await;
    check(
        &pool,
        json!([{"op":"replace","path":"/spec/rules/0/then/decision","value":"STOP"}]),
        true,
        "single-zero index 0 accepted",
    )
    .await;
}

#[tokio::test]
async fn notify_fires_on_state_change() {
    let Some(pool) = try_connect().await else {
        eprintln!("[skip] no DB reachable at {}", ledger_url());
        return;
    };
    ensure_partition(&pool).await;

    // Subscribe via a dedicated connection.
    let mut listener = sqlx::postgres::PgListener::connect(&ledger_url())
        .await
        .expect("connect listener");
    listener
        .listen("approval_requests_state_change")
        .await
        .expect("listen");

    let tenant = Uuid::new_v4();
    let finding_id = Uuid::new_v4();
    insert_finding(&pool, tenant, finding_id, 'c').await;

    let patch = json!([{"op":"replace","path":"/spec/rules/0/then/decision","value":"STOP"}]);
    let outcome =
        proposal_writer::write_proposal(&pool, tenant, finding_id, 1, &patch, &ProposalConfig::default())
            .await
            .expect("write proposal");
    let approval_id = match outcome {
        ProposalOutcome::Inserted { approval_id, .. } => approval_id,
        other => panic!("expected Inserted, got {:?}", other),
    };

    // Transition pending → approved via the SP.
    sqlx::query(
        "SELECT * FROM resolve_approval_request($1::uuid, 'approved', 'test-sub', 'test-iss', 'r')",
    )
    .bind(approval_id)
    .execute(&pool)
    .await
    .expect("resolve_approval_request");

    // Wait for the NOTIFY (up to 2s).
    let notif = tokio::time::timeout(std::time::Duration::from_secs(2), listener.recv())
        .await
        .expect("notify timeout")
        .expect("listener channel");
    let payload: serde_json::Value =
        serde_json::from_str(notif.payload()).expect("payload json");
    assert_eq!(payload["proposal_source"], "cost_advisor");
    assert_eq!(payload["new_state"], "approved");
    assert_eq!(payload["old_state"], "pending");
    assert_eq!(payload["approval_id"], approval_id.to_string());
}
