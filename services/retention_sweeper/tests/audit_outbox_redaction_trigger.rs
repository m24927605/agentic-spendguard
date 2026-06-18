//! Regression / quality gate for the audit_outbox prompt-redaction path.
//!
//! Background (the bug this guards against): `audit_outbox.cloudevent_payload`
//! is protected by the `audit_outbox_immutability` trigger (migration 0011,
//! re-asserted in 0046), whose function rejects ANY change to that column
//! with errcode 42P10. The original retention sweeper issued a RAW
//! `UPDATE audit_outbox SET cloudevent_payload = ...`, which is therefore
//! UNCONDITIONALLY rejected — and the per-row error was swallowed, so the
//! prompt-redaction compliance control silently never ran. There was no
//! migrated-DB test, so the bug-class could recur invisibly.
//!
//! This test runs the redaction path against a migrated DB carrying the
//! production immutability trigger and locks two invariants:
//!
//!   1. A raw `UPDATE ... SET cloudevent_payload = ...` IS rejected
//!      (errcode 42P10). This proves the bug exists and that the trigger
//!      is the load-bearing audit-immutability control — a future
//!      "just UPDATE directly" regression in the sweeper would fail here.
//!
//!   2. The SECURITY DEFINER `redact_audit_outbox_data(...)` SP is the
//!      ONLY sanctioned redaction path: when present in the schema it
//!      succeeds and leaves the row redacted (data -> marker, plus the
//!      `_data_sha256_hex` digest), while the immutability invariant for
//!      every OTHER column still holds. The SP itself lives in a ledger
//!      migration (cross-cutting); until that migration lands the SP is
//!      absent and the structural assertion is skipped with a clear
//!      message rather than failing — but invariant (1) is always checked.
//!
//! Requires Docker. If the test runner cannot reach the daemon the test
//! is skipped with a clear message rather than failing.

use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use std::time::Duration;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

/// Minimal but faithful reproduction of the production audit_outbox
/// immutability surface plus the sanctioned-redaction exemption, mirroring
/// migration 0064:
///   * `audit_outbox` with the column the redaction touches
///     (`cloudevent_payload`).
///   * `reject_audit_outbox_immutable_columns` (0011 / 0046, tightened in
///     0064) — for the columns present here. The essential invariants under
///     test are "any UNSANCTIONED change to cloudevent_payload is rejected"
///     and "the sanctioned redaction shape under the per-row GUC is
///     permitted".
///   * `redact_audit_outbox_data` SECURITY DEFINER SP (0064) — the SOLE
///     sanctioned path; sets the per-row GUC, performs the bounded
///     jsonb_set, clears the GUC.
const SCHEMA_SQL: &str = r#"
CREATE TABLE audit_outbox (
    audit_outbox_id          UUID PRIMARY KEY,
    tenant_id                UUID NOT NULL,
    event_type               TEXT NOT NULL,
    cloudevent_payload       JSONB NOT NULL,
    recorded_at              TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    -- forwarder-state columns: intentionally UPDATE-able (excluded below)
    pending_forward          BOOLEAN NOT NULL DEFAULT TRUE,
    forwarded_at             TIMESTAMPTZ
);

-- Mirrors reject_audit_outbox_immutable_columns (0011 §audit_outbox /
-- re-asserted 0046 §step 5, tightened 0064): immutable columns cannot
-- change; only the forwarder-state columns are UPDATE-able. A change to
-- cloudevent_payload is rejected UNLESS the per-row redaction GUC is set
-- and the sole delta is data -> redaction marker + _data_sha256_hex.
CREATE OR REPLACE FUNCTION reject_audit_outbox_immutable_columns()
RETURNS TRIGGER
SECURITY INVOKER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    v_sanctioned BOOLEAN := FALSE;
    v_redaction_ctx TEXT;
    v_expected JSONB;
BEGIN
    v_redaction_ctx := current_setting('spendguard.redaction_audit_id', true);
    IF v_redaction_ctx IS NOT NULL
       AND v_redaction_ctx <> ''
       AND v_redaction_ctx = NEW.audit_outbox_id::TEXT
       AND OLD.cloudevent_payload IS DISTINCT FROM NEW.cloudevent_payload THEN
        IF COALESCE((NEW.cloudevent_payload->'data'->>'_redacted')::BOOLEAN, FALSE) = TRUE
           AND (NEW.cloudevent_payload ? '_data_sha256_hex') THEN
            v_expected := jsonb_set(
                jsonb_set(OLD.cloudevent_payload,
                          '{data}', NEW.cloudevent_payload->'data', true),
                '{_data_sha256_hex}', NEW.cloudevent_payload->'_data_sha256_hex', true);
            IF v_expected = NEW.cloudevent_payload THEN
                v_sanctioned := TRUE;
            END IF;
        END IF;
    END IF;

    IF (OLD.audit_outbox_id, OLD.tenant_id, OLD.event_type,
        OLD.cloudevent_payload, OLD.recorded_at)
       IS DISTINCT FROM
       (NEW.audit_outbox_id, NEW.tenant_id, NEW.event_type,
        CASE WHEN v_sanctioned THEN OLD.cloudevent_payload
             ELSE NEW.cloudevent_payload END,
        NEW.recorded_at) THEN
        RAISE EXCEPTION 'audit_outbox immutable columns cannot be changed'
            USING ERRCODE = '42P10';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_outbox_immutability
    BEFORE UPDATE ON audit_outbox
    FOR EACH ROW EXECUTE FUNCTION reject_audit_outbox_immutable_columns();

-- Sanctioned redaction SP (mirror of migration 0064). SECURITY DEFINER in
-- production; here the test connects as the owner so the privilege aspect
-- is moot — what we exercise is the GUC-gated trigger exemption.
CREATE OR REPLACE FUNCTION redact_audit_outbox_data(
    p_audit_id   UUID,
    p_marker     JSONB,
    p_digest_hex TEXT
) RETURNS VOID
LANGUAGE plpgsql
AS $$
DECLARE
    v_rows INT;
BEGIN
    IF p_marker IS NULL
       OR COALESCE((p_marker->>'_redacted')::BOOLEAN, FALSE) IS DISTINCT FROM TRUE THEN
        RAISE EXCEPTION 'redact_audit_outbox_data: marker must be the redaction marker'
            USING ERRCODE = '22023';
    END IF;
    IF p_digest_hex IS NULL OR p_digest_hex !~ '^[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'redact_audit_outbox_data: digest must be a lowercase 64-char hex SHA-256'
            USING ERRCODE = '22023';
    END IF;
    PERFORM pg_catalog.set_config('spendguard.redaction_audit_id', p_audit_id::TEXT, true);
    UPDATE audit_outbox
       SET cloudevent_payload =
               jsonb_set(
                   jsonb_set(cloudevent_payload, '{data}', p_marker, true),
                   '{_data_sha256_hex}', to_jsonb(p_digest_hex), true)
     WHERE audit_outbox_id = p_audit_id
       AND COALESCE((cloudevent_payload->'data'->>'_redacted')::BOOLEAN, FALSE) = FALSE;
    GET DIAGNOSTICS v_rows = ROW_COUNT;
    PERFORM pg_catalog.set_config('spendguard.redaction_audit_id', '', true);
END;
$$;

INSERT INTO audit_outbox (audit_outbox_id, tenant_id, event_type, cloudevent_payload)
VALUES (
    '11111111-1111-1111-1111-111111111111',
    '22222222-2222-2222-2222-222222222222',
    'agent.decision',
    '{"data": {"prompt": "secret prompt text"}}'::jsonb
);
"#;

/// A well-formed 64-char lowercase hex SHA-256 digest for the SP's input
/// validation (the production sweeper passes `hex::encode(Sha256...)`).
const VALID_DIGEST: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

async fn setup_pool() -> Option<sqlx::PgPool> {
    let container = match Postgres::default().start().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[retention-redaction-test] testcontainers Postgres not available: {e}");
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
        .expect("connect");

    sqlx::query("CREATE EXTENSION IF NOT EXISTS pgcrypto")
        .execute(&pool)
        .await
        .expect("pgcrypto");
    sqlx::raw_sql(SCHEMA_SQL)
        .execute(&pool)
        .await
        .expect("apply schema + immutability trigger");

    // Keep container alive for the duration of the test by leaking.
    Box::leak(Box::new(container));

    Some(pool)
}

const AUDIT_ID: &str = "11111111-1111-1111-1111-111111111111";

/// Invariant (1): a RAW UPDATE to cloudevent_payload — exactly what the
/// original sweeper did — is rejected by the immutability trigger. This is
/// the bug-class lock: if anyone reintroduces a direct UPDATE, this fails.
#[tokio::test]
async fn raw_cloudevent_payload_update_is_rejected_by_immutability_trigger() {
    let Some(pool) = setup_pool().await else {
        eprintln!("[retention-redaction-test] skipped (no Docker)");
        return;
    };

    let marker = r#"{"_redacted": true, "redacted_at": "2026-06-18T00:00:00Z"}"#;
    let res = sqlx::query(
        r#"
        UPDATE audit_outbox
           SET cloudevent_payload =
                   jsonb_set(
                       jsonb_set(cloudevent_payload, '{data}', $2::JSONB, true),
                       '{_data_sha256_hex}', to_jsonb($3::TEXT), true)
         WHERE audit_outbox_id = $1
        "#,
    )
    .bind(Uuid::parse_str(AUDIT_ID).unwrap())
    .bind(sqlx::types::Json(serde_json::from_str::<serde_json::Value>(marker).unwrap()))
    .bind("deadbeef")
    .execute(&pool)
    .await;

    let err = res.expect_err("raw cloudevent_payload UPDATE must be rejected by the trigger");
    let db_err = err
        .as_database_error()
        .expect("expected a database error from the immutability trigger");
    assert_eq!(
        db_err.code().as_deref(),
        Some("42P10"),
        "expected errcode 42P10 (audit immutability), got: {db_err}"
    );

    // And the row is unchanged: the prompt is still present, NOT redacted.
    let row = sqlx::query("SELECT cloudevent_payload->'data'->>'_redacted' AS r FROM audit_outbox WHERE audit_outbox_id = $1")
        .bind(Uuid::parse_str(AUDIT_ID).unwrap())
        .fetch_one(&pool)
        .await
        .expect("fetch row");
    let redacted: Option<String> = row.get("r");
    assert!(
        redacted.is_none(),
        "row must remain unredacted after a rejected raw UPDATE (was: {redacted:?})"
    );
}

/// Invariant (2): the sanctioned SECURITY DEFINER SP
/// `redact_audit_outbox_data(...)` (migration 0064, mirrored into the
/// embedded schema above) is the ONLY path that redacts the row in place.
/// It sets the per-row GUC the trigger checks, performs the bounded
/// jsonb_set (data -> marker + _data_sha256_hex), and leaves every other
/// column untouched. This is the structural half of the fix: it proves the
/// redaction control actually persists, not just that raw UPDATEs fail.
#[tokio::test]
async fn sanctioned_redaction_sp_redacts_row() {
    let Some(pool) = setup_pool().await else {
        eprintln!("[retention-redaction-test] skipped (no Docker)");
        return;
    };

    let marker = r#"{"_redacted": true, "redacted_at": "2026-06-18T00:00:00Z"}"#;
    sqlx::query("SELECT redact_audit_outbox_data($1, $2::JSONB, $3::TEXT)")
        .bind(Uuid::parse_str(AUDIT_ID).unwrap())
        .bind(sqlx::types::Json(serde_json::from_str::<serde_json::Value>(marker).unwrap()))
        .bind(VALID_DIGEST)
        .execute(&pool)
        .await
        .expect("sanctioned redaction SP must succeed");

    let row = sqlx::query(
        "SELECT cloudevent_payload->'data'->>'_redacted' AS r, \
                cloudevent_payload->>'_data_sha256_hex' AS h, \
                cloudevent_payload->'data'->>'prompt' AS p \
           FROM audit_outbox WHERE audit_outbox_id = $1",
    )
    .bind(Uuid::parse_str(AUDIT_ID).unwrap())
    .fetch_one(&pool)
    .await
    .expect("fetch redacted row");
    let redacted: Option<String> = row.get("r");
    let digest: Option<String> = row.get("h");
    let prompt: Option<String> = row.get("p");
    assert_eq!(
        redacted.as_deref(),
        Some("true"),
        "row must be redacted after the sanctioned SP runs"
    );
    assert_eq!(
        digest.as_deref(),
        Some(VALID_DIGEST),
        "the digest must be preserved alongside the redaction marker"
    );
    assert!(
        prompt.is_none(),
        "the raw prompt text must be gone after redaction (was: {prompt:?})"
    );
}

/// Invariant (3): the exemption is SURGICAL — it does NOT reopen the
/// audit-immutability hole. A direct UPDATE that mimics the exact redaction
/// shape but WITHOUT going through the SP (so the per-row GUC is unset) is
/// still rejected with 42P10. This is the regression lock against a future
/// "just set the GUC and UPDATE" or "allow any cloudevent_payload change
/// from role X" shortcut.
#[tokio::test]
async fn redaction_shape_update_without_sp_guc_is_rejected() {
    let Some(pool) = setup_pool().await else {
        eprintln!("[retention-redaction-test] skipped (no Docker)");
        return;
    };

    let marker = r#"{"_redacted": true, "redacted_at": "2026-06-18T00:00:00Z"}"#;
    // Exact redaction shape, but a plain UPDATE: no SP, so the
    // spendguard.redaction_audit_id GUC is unset -> trigger must reject.
    let res = sqlx::query(
        r#"
        UPDATE audit_outbox
           SET cloudevent_payload =
                   jsonb_set(
                       jsonb_set(cloudevent_payload, '{data}', $2::JSONB, true),
                       '{_data_sha256_hex}', to_jsonb($3::TEXT), true)
         WHERE audit_outbox_id = $1
        "#,
    )
    .bind(Uuid::parse_str(AUDIT_ID).unwrap())
    .bind(sqlx::types::Json(serde_json::from_str::<serde_json::Value>(marker).unwrap()))
    .bind(VALID_DIGEST)
    .execute(&pool)
    .await;

    let err = res.expect_err("redaction-shaped UPDATE without the SP GUC must be rejected");
    let db_err = err
        .as_database_error()
        .expect("expected a database error from the immutability trigger");
    assert_eq!(
        db_err.code().as_deref(),
        Some("42P10"),
        "expected errcode 42P10 for an un-sanctioned redaction-shaped UPDATE, got: {db_err}"
    );

    let row = sqlx::query(
        "SELECT cloudevent_payload->'data'->>'_redacted' AS r FROM audit_outbox WHERE audit_outbox_id = $1",
    )
    .bind(Uuid::parse_str(AUDIT_ID).unwrap())
    .fetch_one(&pool)
    .await
    .expect("fetch row");
    let redacted: Option<String> = row.get("r");
    assert!(
        redacted.is_none(),
        "row must remain unredacted after a rejected un-sanctioned UPDATE (was: {redacted:?})"
    );
}

/// Invariant (4): the SP validates its own inputs — a malformed digest
/// (not 64-char lowercase hex) is rejected, so a buggy caller cannot store
/// a junk forensic digest. Belt-and-suspenders alongside the trigger.
#[tokio::test]
async fn sp_rejects_malformed_digest() {
    let Some(pool) = setup_pool().await else {
        eprintln!("[retention-redaction-test] skipped (no Docker)");
        return;
    };

    let marker = r#"{"_redacted": true, "redacted_at": "2026-06-18T00:00:00Z"}"#;
    let res = sqlx::query("SELECT redact_audit_outbox_data($1, $2::JSONB, $3::TEXT)")
        .bind(Uuid::parse_str(AUDIT_ID).unwrap())
        .bind(sqlx::types::Json(serde_json::from_str::<serde_json::Value>(marker).unwrap()))
        .bind("deadbeef") // too short / not 64 hex chars
        .execute(&pool)
        .await;

    let err = res.expect_err("SP must reject a malformed digest");
    let db_err = err
        .as_database_error()
        .expect("expected a database error from the SP input validation");
    assert_eq!(
        db_err.code().as_deref(),
        Some("22023"),
        "expected errcode 22023 (invalid_parameter_value) for malformed digest, got: {db_err}"
    );
}
