//! D16 COV_86 — fixture replay round-trip. Loads the canonical
//! `genspark_usage.json` and confirms the audit-row contract.

use std::path::PathBuf;

use spendguard_importer_genspark::{
    import_record_to_audit_row, CreditPriceTable, FixtureLoader, IngestionMode,
};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/genspark_usage.json")
}

fn prices() -> CreditPriceTable {
    CreditPriceTable::load_from_embedded()
}

#[test]
fn fixture_loads_at_least_one_record_per_plan_path() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let mut plus = 0usize;
    let mut premium = 0usize;
    let mut unknown = 0usize;
    for r in loader.records() {
        match r.plan.as_str() {
            "plus" => plus += 1,
            "premium" => premium += 1,
            "enterprise" => unknown += 1,
            _ => {}
        }
    }
    // Review-standards P5 / coverage: at least one record per plan variant
    // PLUS at least one unknown-plan slug.
    assert!(plus >= 1, "fixture must have at least one plus record");
    assert!(
        premium >= 1,
        "fixture must have at least one premium record"
    );
    assert!(
        unknown >= 1,
        "fixture must have at least one unknown-plan record to exercise fallback"
    );
}

#[test]
fn fixture_round_trip_produces_audit_rows() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    for rec in loader.records() {
        let row = import_record_to_audit_row(rec, &prices).unwrap();
        assert_eq!(row.tenant_id, rec.tenant_id);
        assert_eq!(row.reservation_source, "subscription_meter");
        assert_eq!(row.import_source, "genspark_team_api");
        assert_eq!(row.ingestion_mode, IngestionMode::Fixture);
    }
}

#[test]
fn fixture_round_trip_is_idempotent() {
    // Acceptance: re-running the same window must produce the same
    // event_id deterministically.
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
fn fixture_unknown_plan_record_lands_with_zero_amount_and_reason() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    let unknown = loader
        .records()
        .iter()
        .find(|r| r.plan == "enterprise")
        .expect("fixture missing enterprise record");
    let row = import_record_to_audit_row(unknown, &prices).unwrap();
    // T7: BOTH fields must be set.
    assert_eq!(row.amount_micro_usd, 0);
    assert_eq!(row.reason_code, Some("genspark_plan_unknown"));
}

#[test]
fn fixture_pricing_version_stamped() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    for rec in loader.records() {
        let row = import_record_to_audit_row(rec, &prices).unwrap();
        // Pricing version stamped on EVERY row, even unknown-plan.
        assert_eq!(
            row.pricing_version.as_deref(),
            Some("genspark-credit-v1-2026-06"),
        );
    }
}

#[test]
fn fixture_plus_record_conversion_correctness() {
    // 3200 credits × $0.001999/credit = $6.3968 = 6_396_800 micro-USD.
    // Hand-computed expected per review-standards P7.
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    let plus_rows: Vec<_> = loader
        .records()
        .iter()
        .filter(|r| r.plan == "plus")
        .map(|r| import_record_to_audit_row(r, &prices).unwrap())
        .collect();
    assert!(!plus_rows.is_empty());
    for row in &plus_rows {
        assert!(row.amount_micro_usd > 0, "plus-plan amount must be positive");
        assert_eq!(row.reason_code, None);
    }
    // The 3200-credit row should produce 6_396_800 micro-USD.
    let three_k_two_hundred = plus_rows
        .iter()
        .find(|r| (r.credits_consumed - 3200.0).abs() < 1e-9)
        .expect("fixture should contain the 3200-credit plus row");
    assert_eq!(three_k_two_hundred.amount_micro_usd, 6_396_800);
}

#[test]
fn fixture_premium_record_uses_premium_rate() {
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let prices = prices();
    let premium_row = loader
        .records()
        .iter()
        .find(|r| r.plan == "premium")
        .expect("fixture missing premium record");
    let row = import_record_to_audit_row(premium_row, &prices).unwrap();
    assert!(row.amount_micro_usd > 0);
    assert_eq!(row.reason_code, None);
    // 50_000 credits × $0.00199992/credit ≈ $99.996 ≈ 99_996_000 micro-USD.
    assert!(row.amount_micro_usd > 99_000_000);
    assert!(row.amount_micro_usd < 101_000_000);
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
    // Exactly one record per (workspace_id, task_id, window_end) triple.
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let mut seen = std::collections::HashSet::new();
    for r in loader.records() {
        let key = (r.workspace_id.clone(), r.task_id.clone(), r.window_end);
        assert!(
            seen.insert(key),
            "duplicate idempotency key in fixture: ({}, {}, {})",
            r.workspace_id,
            r.task_id,
            r.window_end,
        );
    }
}

#[test]
fn fixture_under_one_mib() {
    // Review-standards: fixture file must be < 1 MiB.
    let meta = std::fs::metadata(fixture_path()).unwrap();
    assert!(
        meta.len() < 1024 * 1024,
        "fixture too large: {} bytes",
        meta.len(),
    );
}

#[test]
fn fixture_synthetic_ids_only() {
    // T9 covered via FixtureLoader rejection at parse time. Belt-and-
    // suspenders here.
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    for r in loader.records() {
        assert!(
            r.workspace_id.starts_with("FAKE_ws_"),
            "non-synthetic workspace id {}",
            r.workspace_id,
        );
        assert!(
            r.task_id.starts_with("FAKE_task_"),
            "non-synthetic task id {}",
            r.task_id,
        );
    }
}

#[test]
fn fixture_no_prompt_content() {
    // T10: fixture MUST NOT contain prompt content. The admin API does
    // not return prompts; any "content" field would indicate fixture
    // contamination.
    let raw = std::fs::read_to_string(fixture_path()).unwrap();
    assert!(
        !raw.contains("\"content\":"),
        "fixture contains a 'content' field — possible prompt leak",
    );
    assert!(
        !raw.contains("Bearer "),
        "fixture contains 'Bearer ' — possible token leak",
    );
    assert!(
        !raw.contains("Authorization"),
        "fixture contains 'Authorization' — possible header leak",
    );
}

#[test]
fn fixture_provenance_pin_holds() {
    // PROVENANCE.md pins the SHA-256 of the fixture body. If the
    // fixture is edited by hand without re-running the generator,
    // this test fails and the reviewer is forced to look.
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    assert_eq!(
        loader.sha256_hex(),
        "fd2c0bb772bfbf2605ce09204aed0025cd754c3edce7296ee281637e5a52baf6",
        "fixture SHA-256 drift — regenerate via \
         services/importer_genspark/scripts/generate_fixture.py and update PROVENANCE.md",
    );
}

#[test]
fn fixture_at_least_two_distinct_workspaces() {
    // Coverage: the demo verifier groups by workspace, so the fixture
    // must exercise multi-workspace dispatch.
    let loader = FixtureLoader::new(&fixture_path()).unwrap();
    let distinct: std::collections::HashSet<_> =
        loader.records().iter().map(|r| &r.workspace_id).collect();
    assert!(
        distinct.len() >= 2,
        "fixture must exercise >= 2 distinct workspaces; got {}",
        distinct.len(),
    );
}
