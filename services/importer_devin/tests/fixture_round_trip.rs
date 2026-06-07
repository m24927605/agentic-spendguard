//! D14 COV_69 — fixture replay round-trip. Loads the canonical
//! `devin_usage.json` and confirms the audit-row contract.

use std::path::PathBuf;

use spendguard_importer_devin::{
    import_record_to_audit_row, AcuPriceTable, FixtureLoader, IngestionMode,
};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/devin_usage.json")
}

fn prices() -> AcuPriceTable {
    AcuPriceTable::load_from_embedded()
}

#[test]
fn fixture_loads_at_least_one_record_per_plan() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let mut team = 0usize;
    let mut ent = 0usize;
    for r in loader.records() {
        match r.plan.as_str() {
            "team" => team += 1,
            "enterprise" => ent += 1,
            _ => {}
        }
    }
    // Review-standards P3: at least one record per plan variant.
    assert!(team >= 1, "fixture must have at least one team record");
    assert!(ent >= 1, "fixture must have at least one enterprise record");
}

#[test]
fn fixture_round_trip_produces_audit_rows() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    for rec in loader.records() {
        let row = import_record_to_audit_row(rec, &prices).unwrap();
        assert_eq!(row.tenant_id, rec.tenant_id);
        assert_eq!(row.reservation_source, "subscription_meter");
        assert_eq!(row.import_source, "devin_team_api");
        assert_eq!(row.ingestion_mode, IngestionMode::Fixture);
    }
}

#[test]
fn fixture_round_trip_is_idempotent() {
    // T12 / acceptance A4.2 — re-running the same window must produce
    // the same event_id deterministically.
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    let first: Vec<_> = loader
        .records()
        .iter()
        .map(|r| import_record_to_audit_row(r, &prices).unwrap())
        .collect();
    let second: Vec<_> = loader
        .records()
        .iter()
        .map(|r| import_record_to_audit_row(r, &prices).unwrap())
        .collect();
    assert_eq!(first, second, "import is not idempotent");
}

#[test]
fn fixture_enterprise_record_lands_with_null_amount() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    let ent = loader
        .records()
        .iter()
        .find(|r| r.plan == "enterprise")
        .expect("fixture missing enterprise record");
    let row = import_record_to_audit_row(ent, &prices).unwrap();
    assert_eq!(row.amount_micro_usd, None);
    assert_eq!(row.reason_code, Some("devin_enterprise_negotiated_rate"));
}

#[test]
fn fixture_pricing_version_stamped() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    for rec in loader.records() {
        let row = import_record_to_audit_row(rec, &prices).unwrap();
        assert_eq!(row.pricing_version.as_deref(), Some("devin-acu-v1-2026-06"),);
    }
}

#[test]
fn fixture_team_record_conversion_correctness() {
    // The 12.5 ACU × $2.25 = 28_125_000 micro-USD headline conversion
    // is verified at the unit level — here we just confirm at least
    // one fixture team record carries a positive amount.
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    let team_rows: Vec<_> = loader
        .records()
        .iter()
        .filter(|r| r.plan == "team")
        .map(|r| import_record_to_audit_row(r, &prices).unwrap())
        .collect();
    assert!(!team_rows.is_empty());
    for row in &team_rows {
        let amt = row.amount_micro_usd.expect("team plan must produce amount");
        assert!(amt > 0, "team-plan amount must be positive");
    }
}

#[test]
fn fixture_sha256_provenance_stamped_on_every_record() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let sha = loader.sha256_hex().to_string();
    assert_eq!(sha.len(), 64);
    for r in loader.records() {
        assert_eq!(r.fixture_provenance_sha256.as_deref(), Some(sha.as_str()),);
    }
}

#[test]
fn fixture_idempotency_key_uniqueness() {
    // Review-standards P4: exactly one record per
    // (devin_team_id, devin_session_id, window_end) triple. Sanity:
    // a duplicate would mean the live importer would emit two events
    // with the same UUIDv5 (canonical_ingest dedups via
    // event_replay_dedup, but the fixture should still be clean).
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let mut seen = std::collections::HashSet::new();
    for r in loader.records() {
        let key = (
            r.devin_team_id.clone(),
            r.devin_session_id.clone(),
            r.window_end,
        );
        assert!(
            seen.insert(key),
            "duplicate idempotency key in fixture: \
             ({}, {}, {})",
            r.devin_team_id,
            r.devin_session_id,
            r.window_end,
        );
    }
}

#[test]
fn fixture_under_one_mib() {
    // Review-standards P1: fixture file must be < 1 MiB.
    let meta = std::fs::metadata(fixture_path()).unwrap();
    assert!(
        meta.len() < 1024 * 1024,
        "fixture too large: {} bytes",
        meta.len(),
    );
}

#[test]
fn fixture_synthetic_ids_only() {
    // T5 covered via FixtureLoader rejection at parse time. Belt-and-
    // suspenders here.
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    for r in loader.records() {
        assert!(
            r.devin_team_id.starts_with("TEAM_FIXTURE_"),
            "non-synthetic team id {}",
            r.devin_team_id,
        );
        assert!(
            r.devin_session_id.starts_with("SESSION_FIXTURE_"),
            "non-synthetic session id {}",
            r.devin_session_id,
        );
    }
}

#[test]
fn fixture_provenance_pin_holds() {
    // PROVENANCE.md pins the SHA-256 of the fixture body. If the
    // fixture is edited by hand without re-running the generator,
    // this test fails and the reviewer is forced to look.
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    assert_eq!(
        loader.sha256_hex(),
        "aa4c172164a8a6a5d4e97c6bde4ac455e01f5f37932b8a3561ef213049144807",
        "fixture SHA-256 drift — regenerate via \
         services/importer_devin/scripts/generate_fixture.py and update PROVENANCE.md",
    );
}
