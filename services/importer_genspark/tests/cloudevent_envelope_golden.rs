//! D16 COV_86 — byte-equal CloudEvent envelope golden test.
//!
//! Pinned against `tests/golden/cloudevent_v1alpha1_*.json`. Any
//! envelope change requires editing both `cloudevent_envelope.rs` and
//! the matching golden file. Run with `UPDATE_GOLDEN=1 cargo test` to
//! bootstrap or refresh after a deliberate envelope change.

use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};
use spendguard_importer_genspark::{
    cloudevent_envelope::build_with, CreditPriceTable, ImportRecord, IngestionMode,
};
use uuid::Uuid;

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

fn prices() -> CreditPriceTable {
    CreditPriceTable::load_from_embedded()
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
fn cloudevent_envelope_v1alpha1_plus_fixture_golden() {
    let rec = ImportRecord {
        tenant_id: "demo".into(),
        budget_id: "genspark-budget".into(),
        workspace_id: "FAKE_ws_001".into(),
        task_id: "FAKE_task_001".into(),
        credits_consumed: 3200.0,
        plan: "plus".into(),
        task_category: Some("research".into()),
        window_start: t(2026, 6, 1, 0),
        window_end: t(2026, 6, 1, 1),
        ingestion_mode: IngestionMode::Fixture,
        fixture_provenance_sha256: Some(
            "fd2c0bb772bfbf2605ce09204aed0025cd754c3edce7296ee281637e5a52baf6".into(),
        ),
    };
    let id = Uuid::parse_str("018f4a3a-d971-7c16-91fe-d0169e715ba0").unwrap();
    let time = t(2026, 6, 8, 12);
    let env = build_with(&rec, &prices(), id, time).unwrap();
    let actual = serde_json::to_string_pretty(&env).unwrap() + "\n";
    let expected = read_or_write_golden("cloudevent_v1alpha1_plus_fixture.json", &actual);
    assert_eq!(
        actual, expected,
        "plus / fixture CloudEvent envelope drift — run with UPDATE_GOLDEN=1",
    );
}

#[test]
fn cloudevent_envelope_v1alpha1_unknown_plan_fixture_golden() {
    let rec = ImportRecord {
        tenant_id: "demo".into(),
        budget_id: "genspark-budget".into(),
        workspace_id: "FAKE_ws_003".into(),
        task_id: "FAKE_task_004".into(),
        credits_consumed: 1000.0,
        plan: "enterprise".into(),
        task_category: None,
        window_start: t(2026, 6, 1, 0),
        window_end: t(2026, 6, 1, 1),
        ingestion_mode: IngestionMode::Fixture,
        fixture_provenance_sha256: Some(
            "fd2c0bb772bfbf2605ce09204aed0025cd754c3edce7296ee281637e5a52baf6".into(),
        ),
    };
    let id = Uuid::parse_str("018f4a3a-d971-7c16-91fe-d0169e715bab").unwrap();
    let time = t(2026, 6, 8, 12);
    let env = build_with(&rec, &prices(), id, time).unwrap();
    let actual = serde_json::to_string_pretty(&env).unwrap() + "\n";
    let expected = read_or_write_golden("cloudevent_v1alpha1_unknown_plan_fixture.json", &actual);
    assert_eq!(actual, expected);
}

#[test]
fn cloudevent_envelope_v1alpha1_premium_live_golden() {
    let rec = ImportRecord {
        tenant_id: "demo".into(),
        budget_id: "genspark-budget".into(),
        workspace_id: "FAKE_ws_002".into(),
        task_id: "FAKE_task_003".into(),
        credits_consumed: 50_000.0,
        plan: "premium".into(),
        task_category: Some("code_generation".into()),
        window_start: t(2026, 6, 1, 0),
        window_end: t(2026, 6, 1, 1),
        ingestion_mode: IngestionMode::Live,
        fixture_provenance_sha256: None,
    };
    let id = Uuid::parse_str("018f4a3a-d971-7c16-91fe-d0169e715bac").unwrap();
    let time = t(2026, 6, 8, 12);
    let env = build_with(&rec, &prices(), id, time).unwrap();
    let actual = serde_json::to_string_pretty(&env).unwrap() + "\n";
    let expected = read_or_write_golden("cloudevent_v1alpha1_premium_live.json", &actual);
    assert_eq!(actual, expected);
}

#[test]
fn cloudevent_envelope_matches_schema_doc_required_fields() {
    // Cross-check: every documented field is present in the envelope.
    let rec = ImportRecord {
        tenant_id: "demo".into(),
        budget_id: "genspark-budget".into(),
        workspace_id: "FAKE_ws_001".into(),
        task_id: "FAKE_task_001".into(),
        credits_consumed: 3200.0,
        plan: "plus".into(),
        task_category: Some("research".into()),
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
        "workspace_id",
        "task_id",
        "plan",
        "credits_consumed",
        "usd_per_credit",
        "amount_micro_usd",
        "reason_code",
        "pricing_version",
        "window_start",
        "window_end",
        "reservation_source",
        "import_source",
        "ingestion_mode",
        "fixture_provenance_sha256",
        "task_category",
    ] {
        assert!(data.contains_key(k), "data missing field {k}");
    }
}
