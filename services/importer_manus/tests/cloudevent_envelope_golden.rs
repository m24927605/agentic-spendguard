//! D15 COV_74 — byte-equal CloudEvent envelope golden test.
//!
//! Pinned against `tests/golden/cloudevent_v1alpha1_*.json`. Any
//! envelope change requires editing all of `cloudevent_envelope.rs`,
//! the relevant golden file, and the integration doc.

use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};
use spendguard_importer_manus::{
    cloudevent_envelope::build_with, ImportRecord, IngestionMode, PriceTable, SessionStatus, Tier,
};
use uuid::Uuid;

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

fn prices() -> PriceTable {
    PriceTable::load_embedded()
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
fn cloudevent_envelope_v1alpha1_team_fixture_golden() {
    let rec = ImportRecord {
        session_id: "ses_FAKE_team_completed_001".into(),
        workspace_id: "ws_FAKE_team_001".into(),
        tier: Tier::TeamPlan,
        credits_consumed: 47,
        status: SessionStatus::Completed,
        window_start: t(2026, 6, 5, 14),
        window_end: t(2026, 6, 5, 15),
        ingestion_mode: IngestionMode::Fixture,
        fixture_provenance_sha256: Some(
            "535d5cf688f486cac501436eb24e6357fa6c3677464001a0402e86686a50b0de".into(),
        ),
    };
    let id = Uuid::parse_str("018f4b15-6d61-7c15-91fe-d0156d61aca0").unwrap();
    let time = t(2026, 6, 8, 12);
    let env = build_with(&rec, &prices(), id, time).unwrap();
    let actual = serde_json::to_string_pretty(&env).unwrap() + "\n";
    let expected = read_or_write_golden("cloudevent_v1alpha1_team_fixture.json", &actual);
    assert_eq!(
        actual, expected,
        "team / fixture CloudEvent envelope drift — \
         run with UPDATE_GOLDEN=1 + update integration doc",
    );
}

#[test]
fn cloudevent_envelope_v1alpha1_enterprise_byok_golden() {
    let rec = ImportRecord {
        session_id: "ses_FAKE_byok_006".into(),
        workspace_id: "ws_FAKE_byok_001".into(),
        tier: Tier::EnterpriseByok,
        credits_consumed: 1024,
        status: SessionStatus::Completed,
        window_start: t(2026, 6, 5, 20),
        window_end: t(2026, 6, 5, 22),
        ingestion_mode: IngestionMode::Fixture,
        fixture_provenance_sha256: Some(
            "535d5cf688f486cac501436eb24e6357fa6c3677464001a0402e86686a50b0de".into(),
        ),
    };
    let id = Uuid::parse_str("018f4b15-6d61-7c15-91fe-d0156d61acab").unwrap();
    let time = t(2026, 6, 8, 12);
    let env = build_with(&rec, &prices(), id, time).unwrap();
    let actual = serde_json::to_string_pretty(&env).unwrap() + "\n";
    let expected = read_or_write_golden("cloudevent_v1alpha1_enterprise_byok_fixture.json", &actual);
    assert_eq!(actual, expected);
}

#[test]
fn cloudevent_envelope_v1alpha1_team_live_golden() {
    let rec = ImportRecord {
        session_id: "ses_FAKE_team_completed_001".into(),
        workspace_id: "ws_FAKE_team_001".into(),
        tier: Tier::TeamPlan,
        credits_consumed: 47,
        status: SessionStatus::Completed,
        window_start: t(2026, 6, 5, 14),
        window_end: t(2026, 6, 5, 15),
        ingestion_mode: IngestionMode::Live,
        fixture_provenance_sha256: None,
    };
    let id = Uuid::parse_str("018f4b15-6d61-7c15-91fe-d0156d61acac").unwrap();
    let time = t(2026, 6, 8, 12);
    let env = build_with(&rec, &prices(), id, time).unwrap();
    let actual = serde_json::to_string_pretty(&env).unwrap() + "\n";
    let expected = read_or_write_golden("cloudevent_v1alpha1_team_live.json", &actual);
    assert_eq!(actual, expected);
}

#[test]
fn cloudevent_envelope_matches_schema_required_fields() {
    let rec = ImportRecord {
        session_id: "ses_FAKE_team_completed_001".into(),
        workspace_id: "ws_FAKE_team_001".into(),
        tier: Tier::TeamPlan,
        credits_consumed: 47,
        status: SessionStatus::Completed,
        window_start: t(2026, 6, 5, 14),
        window_end: t(2026, 6, 5, 15),
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
        "workspace_id",
        "session_id",
        "tier",
        "status",
        "credits_consumed",
        "credit_cost_micro_usd",
        "amount_micro_usd",
        "pricing_version",
        "model",
        "input_tokens",
        "output_tokens",
        "window_start",
        "window_end",
        "reservation_source",
        "import_source",
        "ingestion_mode",
        "fixture_provenance_sha256",
        "dedupe_key",
    ] {
        assert!(data.contains_key(k), "data missing field {k}");
    }
}
