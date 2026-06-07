//! D14 COV_70 — byte-equal CloudEvent envelope golden test.
//!
//! Acceptance A7.4 + A7.5. Pinned against
//! `tests/golden/cloudevent_v1alpha1.json`. Any envelope change
//! requires editing all three of: `cloudevent_envelope.rs`,
//! `tests/golden/cloudevent_v1alpha1.json`, and
//! `docs/specs/coverage/D14_devin_importer/cloudevent-schema.md`
//! (review-standards S5).

use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};
use spendguard_importer_devin::{
    cloudevent_envelope::build_with, AcuPriceTable, ImportRecord, IngestionMode,
};
use uuid::Uuid;

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

fn prices() -> AcuPriceTable {
    AcuPriceTable::load_from_embedded()
}

fn t(y: i32, mo: u32, d: u32, h: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, mo, d, h, 0, 0).unwrap()
}

fn read_or_write_golden(name: &str, current: &str) -> String {
    let path = golden_path(name);
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, current).unwrap();
        eprintln!("[golden] updated {path:?}");
    }
    std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden at {path:?}; \
         run with UPDATE_GOLDEN=1 to bootstrap"
        )
    })
}

#[test]
fn cloudevent_envelope_v1alpha1_golden() {
    let rec = ImportRecord {
        tenant_id: "demo".into(),
        budget_id: "devin-budget".into(),
        devin_team_id: "TEAM_FIXTURE_001".into(),
        devin_session_id: "SESSION_FIXTURE_001".into(),
        acu_consumed: 12.5,
        plan: "team".into(),
        window_start: t(2026, 6, 1, 0),
        window_end: t(2026, 6, 1, 1),
        ingestion_mode: IngestionMode::Fixture,
        fixture_provenance_sha256: Some(
            "aa4c172164a8a6a5d4e97c6bde4ac455e01f5f37932b8a3561ef213049144807".into(),
        ),
    };
    let id = Uuid::parse_str("018f4a3a-d971-7c14-91fe-d014de71aca0").unwrap();
    let time = t(2026, 6, 8, 12);
    let env = build_with(&rec, &prices(), id, time).unwrap();
    let actual = serde_json::to_string_pretty(&env).unwrap() + "\n";
    let expected = read_or_write_golden("cloudevent_v1alpha1_team_fixture.json", &actual);
    assert_eq!(
        actual, expected,
        "team / fixture CloudEvent envelope drift — run with UPDATE_GOLDEN=1 + update schema doc",
    );
}

#[test]
fn cloudevent_envelope_v1alpha1_enterprise_golden() {
    let rec = ImportRecord {
        tenant_id: "demo".into(),
        budget_id: "devin-budget".into(),
        devin_team_id: "TEAM_FIXTURE_002".into(),
        devin_session_id: "SESSION_FIXTURE_003".into(),
        acu_consumed: 100.0,
        plan: "enterprise".into(),
        window_start: t(2026, 6, 1, 0),
        window_end: t(2026, 6, 1, 1),
        ingestion_mode: IngestionMode::Fixture,
        fixture_provenance_sha256: Some(
            "aa4c172164a8a6a5d4e97c6bde4ac455e01f5f37932b8a3561ef213049144807".into(),
        ),
    };
    let id = Uuid::parse_str("018f4a3a-d971-7c14-91fe-d014de71acab").unwrap();
    let time = t(2026, 6, 8, 12);
    let env = build_with(&rec, &prices(), id, time).unwrap();
    let actual = serde_json::to_string_pretty(&env).unwrap() + "\n";
    let expected = read_or_write_golden("cloudevent_v1alpha1_enterprise_fixture.json", &actual);
    assert_eq!(actual, expected);
}

#[test]
fn cloudevent_envelope_v1alpha1_live_mode_golden() {
    let rec = ImportRecord {
        tenant_id: "demo".into(),
        budget_id: "devin-budget".into(),
        devin_team_id: "TEAM_FIXTURE_001".into(),
        devin_session_id: "SESSION_FIXTURE_001".into(),
        acu_consumed: 12.5,
        plan: "team".into(),
        window_start: t(2026, 6, 1, 0),
        window_end: t(2026, 6, 1, 1),
        ingestion_mode: IngestionMode::Live,
        fixture_provenance_sha256: None,
    };
    let id = Uuid::parse_str("018f4a3a-d971-7c14-91fe-d014de71acac").unwrap();
    let time = t(2026, 6, 8, 12);
    let env = build_with(&rec, &prices(), id, time).unwrap();
    let actual = serde_json::to_string_pretty(&env).unwrap() + "\n";
    let expected = read_or_write_golden("cloudevent_v1alpha1_team_live.json", &actual);
    assert_eq!(actual, expected);
}

#[test]
fn cloudevent_envelope_matches_schema_doc_required_fields() {
    // Cross-check: every documented field is present in the envelope.
    let rec = ImportRecord {
        tenant_id: "demo".into(),
        budget_id: "devin-budget".into(),
        devin_team_id: "TEAM_FIXTURE_001".into(),
        devin_session_id: "SESSION_FIXTURE_001".into(),
        acu_consumed: 12.5,
        plan: "team".into(),
        window_start: t(2026, 6, 1, 0),
        window_end: t(2026, 6, 1, 1),
        ingestion_mode: IngestionMode::Fixture,
        fixture_provenance_sha256: Some("a".repeat(64)),
    };
    let env = build_with(&rec, &prices(), Uuid::nil(), t(2026, 6, 8, 12)).unwrap();
    let v: serde_json::Value = serde_json::to_value(&env).unwrap();
    let obj = v.as_object().unwrap();
    for k in [
        "specversion",
        "type",
        "source",
        "id",
        "time",
        "datacontenttype",
        "subject",
        "data",
    ] {
        assert!(obj.contains_key(k), "envelope missing top-level field {k}");
    }
    let data = obj["data"].as_object().unwrap();
    for k in [
        "schema_version",
        "tenant_id",
        "budget_id",
        "devin_team_id",
        "devin_session_id",
        "acu_consumed",
        "usd_per_acu",
        "amount_micro_usd",
        "pricing_version",
        "window_start",
        "window_end",
        "reservation_source",
        "import_source",
        "ingestion_mode",
        "fixture_provenance_sha256",
    ] {
        assert!(data.contains_key(k), "data missing field {k}");
    }
}
