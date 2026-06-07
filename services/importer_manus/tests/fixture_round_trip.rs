//! D15 COV_72 — fixture replay round-trip. Loads the canonical
//! `manus_usage.json` and confirms the audit-row contract +
//! demo-path emission count.

use std::path::PathBuf;

use spendguard_importer_manus::{
    import_record_to_audit_row, FixtureLoader, IngestionMode, PriceTable, SessionStatus, Tier,
};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/manus_usage.json")
}

fn prices() -> PriceTable {
    PriceTable::load_embedded()
}

#[test]
fn fixture_loads_eight_sessions() {
    // Acceptance A1.5: exactly 8 sessions committed.
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    assert_eq!(loader.records().len(), 8);
}

#[test]
fn fixture_loads_at_least_one_record_per_tier() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let mut team = 0usize;
    let mut ent = 0usize;
    let mut byok = 0usize;
    for r in loader.records() {
        match r.tier {
            Tier::TeamPlan => team += 1,
            Tier::Enterprise => ent += 1,
            Tier::EnterpriseByok => byok += 1,
        }
    }
    assert!(team >= 1, "fixture must include team_plan");
    assert!(ent >= 1, "fixture must include enterprise");
    assert!(byok >= 1, "fixture must include enterprise_byok");
}

#[test]
fn fixture_terminal_records_emits_seven() {
    // Headline gate A5.1: 8 loaded, in_progress filtered, 7 emitted.
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let terminal: Vec<_> = loader.terminal_records().collect();
    assert_eq!(terminal.len(), 7);
    for r in terminal {
        assert!(r.status.is_terminal());
        assert!(r.status != SessionStatus::InProgress);
    }
}

#[test]
fn fixture_round_trip_produces_audit_rows() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    for rec in loader.records() {
        let row = import_record_to_audit_row(rec, &prices).unwrap();
        assert_eq!(row.tenant_id, rec.workspace_id);
        assert_eq!(row.reservation_source, "subscription_meter");
        assert_eq!(row.import_source, "manus_team_api");
        assert_eq!(row.ingestion_mode, IngestionMode::Fixture);
    }
}

#[test]
fn fixture_round_trip_is_idempotent() {
    // Acceptance A5.6 / E8 / X3: re-running yields byte-identical rows.
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
fn fixture_team_plan_terminal_amount_total() {
    // Headline gate A5.4: team_plan terminal rows total 1010 credits.
    //   47 + 12 + 0 + 950 + 1 = 1010
    //   1010 × 20_526 micro-USD = 20_731_260
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    let team_total: i64 = loader
        .terminal_records()
        .filter(|r| r.tier == Tier::TeamPlan)
        .map(|r| import_record_to_audit_row(r, &prices).unwrap().amount_micro_usd)
        .sum();
    assert_eq!(team_total, 20_731_260, "team_plan headline total mismatch");
}

#[test]
fn fixture_byok_amount_is_zero() {
    // Acceptance A5.5: BYOK tier always zero amount (load-bearing P3).
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    let byok = loader
        .records()
        .iter()
        .find(|r| r.tier == Tier::EnterpriseByok)
        .expect("fixture missing enterprise_byok record");
    let row = import_record_to_audit_row(byok, &prices).unwrap();
    assert_eq!(row.amount_micro_usd, 0);
}

#[test]
fn fixture_enterprise_default_amount_is_zero() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    let ent = loader
        .records()
        .iter()
        .find(|r| r.tier == Tier::Enterprise)
        .expect("fixture missing enterprise record");
    let row = import_record_to_audit_row(ent, &prices).unwrap();
    // Operator override required; default is 0.
    assert_eq!(row.amount_micro_usd, 0);
}

#[test]
fn fixture_pricing_version_stamped() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    for rec in loader.records() {
        let row = import_record_to_audit_row(rec, &prices).unwrap();
        assert_eq!(row.pricing_version, "manus-credit-v1-2026-06");
    }
}

#[test]
fn fixture_sha256_provenance_stamped_on_every_record() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let sha = loader.sha256_hex().to_string();
    assert_eq!(sha.len(), 64);
    for r in loader.records() {
        assert_eq!(r.fixture_provenance_sha256.as_deref(), Some(sha.as_str()));
    }
}

#[test]
fn fixture_idempotency_key_uniqueness() {
    // Review-standards E6 / X3: exactly one record per
    // (workspace_id, session_id, window_end) triple.
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let mut seen = std::collections::HashSet::new();
    for r in loader.records() {
        let key = (r.workspace_id.clone(), r.session_id.clone(), r.window_end);
        assert!(
            seen.insert(key),
            "duplicate idempotency key in fixture: \
             ({}, {}, {})",
            r.workspace_id,
            r.session_id,
            r.window_end,
        );
    }
}

#[test]
fn fixture_under_one_mib() {
    // Review-standards P1 spirit: fixture file must be small/static.
    let meta = std::fs::metadata(fixture_path()).unwrap();
    assert!(meta.len() < 1024 * 1024, "fixture too large: {} bytes", meta.len());
}

#[test]
fn fixture_synthetic_ids_only() {
    // T8 covered via FixtureLoader rejection at parse time. Belt-and-
    // suspenders here.
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    for r in loader.records() {
        assert!(
            r.workspace_id.starts_with("ws_FAKE_"),
            "non-synthetic workspace id {}",
            r.workspace_id,
        );
        assert!(
            r.session_id.starts_with("ses_FAKE_"),
            "non-synthetic session id {}",
            r.session_id,
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
        "535d5cf688f486cac501436eb24e6357fa6c3677464001a0402e86686a50b0de",
        "fixture SHA-256 drift — regenerate via \
         services/importer_manus/scripts/generate_fixture.py and update PROVENANCE.md",
    );
}

#[test]
fn fixture_dedupe_keys_all_vendor_prefixed() {
    // X3: every dedupe key starts with `manus:` for vendor isolation.
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    for rec in loader.records() {
        let row = import_record_to_audit_row(rec, &prices).unwrap();
        assert!(
            row.dedupe_key.starts_with("manus:"),
            "dedupe key missing manus: prefix: {}",
            row.dedupe_key,
        );
        // Session ID embedded:
        assert!(row.dedupe_key.contains(&rec.session_id));
    }
}
