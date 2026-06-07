//! D14 COV_69 — Fixture-mode loader. Reads a canonical sanitized
//! `devin_usage.json` snapshot and yields `ImportRecord`s with
//! `ingestion_mode = Fixture` plus the file's SHA-256 hash stamped on
//! every record (review-standards S9, P5).
//!
//! The fixture path is the **default merge gate** (design §3). Live
//! mode is feature-gated and CI-optional.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::import_record::{ImportRecord, IngestionMode};

/// Failures the fixture loader can surface.
#[derive(Debug, thiserror::Error)]
pub enum FixtureLoadError {
    /// File could not be opened / read.
    #[error("fixture I/O at {path}: {source}")]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// File is not valid JSON or doesn't match the fixture schema.
    #[error("fixture parse at {path}: {source}")]
    Parse {
        /// Path that failed.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: serde_json::Error,
    },
    /// Fixture is well-formed but doesn't satisfy the synthetic-ID
    /// invariant (review-standards T5).
    #[error("fixture {path} contains non-synthetic ID at index {index}: {id}")]
    NonSyntheticId {
        /// Path that failed.
        path: PathBuf,
        /// Which record in the array.
        index: usize,
        /// The offending ID.
        id: String,
    },
}

/// Top-level fixture file shape. Two arrays:
///
/// * `usage[]` — one row per `(team, session, window)` triple.
/// * `meta` — bookkeeping (generated_at, vendor_snapshot_url) — opaque.
#[derive(Debug, Deserialize)]
struct FixtureFile {
    #[serde(default)]
    _meta: Option<serde_json::Value>,
    usage: Vec<FixtureUsageRow>,
}

#[derive(Debug, Deserialize)]
struct FixtureUsageRow {
    tenant_id: String,
    budget_id: String,
    devin_team_id: String,
    devin_session_id: String,
    acu_consumed: f64,
    plan: String,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
}

/// Fixture loader. Holds the parsed records + the file's SHA-256
/// hash. Construct via `FixtureLoader::new(path)`.
#[derive(Debug)]
pub struct FixtureLoader {
    path: PathBuf,
    sha256_hex: String,
    records: Vec<ImportRecord>,
}

impl FixtureLoader {
    /// Read + hash + parse a fixture file.
    pub fn new(path: &Path) -> Result<Self, FixtureLoadError> {
        let raw = std::fs::read(path).map_err(|source| FixtureLoadError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let sha256_hex = sha256_hex_of(&raw);
        let file: FixtureFile =
            serde_json::from_slice(&raw).map_err(|source| FixtureLoadError::Parse {
                path: path.to_path_buf(),
                source,
            })?;

        // Synthetic-ID enforcement (review-standards T5). The fixture
        // is committed; non-synthetic IDs would mean a real customer's
        // data leaked. Hard fail.
        let mut records = Vec::with_capacity(file.usage.len());
        for (index, row) in file.usage.into_iter().enumerate() {
            if !is_synthetic_team_id(&row.devin_team_id) {
                return Err(FixtureLoadError::NonSyntheticId {
                    path: path.to_path_buf(),
                    index,
                    id: row.devin_team_id,
                });
            }
            if !is_synthetic_session_id(&row.devin_session_id) {
                return Err(FixtureLoadError::NonSyntheticId {
                    path: path.to_path_buf(),
                    index,
                    id: row.devin_session_id,
                });
            }
            records.push(ImportRecord {
                tenant_id: row.tenant_id,
                budget_id: row.budget_id,
                devin_team_id: row.devin_team_id,
                devin_session_id: row.devin_session_id,
                acu_consumed: row.acu_consumed,
                plan: row.plan,
                window_start: row.window_start,
                window_end: row.window_end,
                ingestion_mode: IngestionMode::Fixture,
                fixture_provenance_sha256: Some(sha256_hex.clone()),
            });
        }

        Ok(Self {
            path: path.to_path_buf(),
            sha256_hex,
            records,
        })
    }

    /// Returns the SHA-256 of the fixture file (hex, lowercase). Same
    /// hash is stamped onto every emitted `ImportRecord`.
    pub fn sha256_hex(&self) -> &str {
        &self.sha256_hex
    }

    /// Path the fixture was loaded from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Parsed import records. Every record carries
    /// `ingestion_mode = Fixture` and the fixture's SHA-256.
    pub fn records(&self) -> &[ImportRecord] {
        &self.records
    }

    /// Move records out for callers that need owned data.
    pub fn into_records(self) -> Vec<ImportRecord> {
        self.records
    }
}

fn sha256_hex_of(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

fn is_synthetic_team_id(s: &str) -> bool {
    // Review-standards T5: `^TEAM_FIXTURE_\d{3}$`.
    s.starts_with("TEAM_FIXTURE_")
        && s.len() == "TEAM_FIXTURE_".len() + 3
        && s["TEAM_FIXTURE_".len()..]
            .chars()
            .all(|c| c.is_ascii_digit())
}

fn is_synthetic_session_id(s: &str) -> bool {
    s.starts_with("SESSION_FIXTURE_")
        && s.len() == "SESSION_FIXTURE_".len() + 3
        && s["SESSION_FIXTURE_".len()..]
            .chars()
            .all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_fixture(json: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(json.as_bytes()).unwrap();
        f
    }

    const VALID_FIXTURE: &str = r#"
    {
        "_meta": { "generated_at": "2026-06-01T00:00:00Z" },
        "usage": [
            {
                "tenant_id": "demo",
                "budget_id": "devin-budget",
                "devin_team_id": "TEAM_FIXTURE_001",
                "devin_session_id": "SESSION_FIXTURE_001",
                "acu_consumed": 12.5,
                "plan": "team",
                "window_start": "2026-06-01T00:00:00Z",
                "window_end": "2026-06-01T01:00:00Z"
            },
            {
                "tenant_id": "demo",
                "budget_id": "devin-budget",
                "devin_team_id": "TEAM_FIXTURE_002",
                "devin_session_id": "SESSION_FIXTURE_002",
                "acu_consumed": 100.0,
                "plan": "enterprise",
                "window_start": "2026-06-01T00:00:00Z",
                "window_end": "2026-06-01T01:00:00Z"
            }
        ]
    }
    "#;

    #[test]
    fn loads_valid_fixture_and_tags_fixture_mode() {
        let f = write_temp_fixture(VALID_FIXTURE);
        let loader = FixtureLoader::new(f.path()).unwrap();
        assert_eq!(loader.records().len(), 2);
        for r in loader.records() {
            assert_eq!(r.ingestion_mode, IngestionMode::Fixture);
            assert!(r.fixture_provenance_sha256.is_some());
            assert_eq!(r.fixture_provenance_sha256.as_deref().unwrap().len(), 64,);
        }
    }

    #[test]
    fn sha256_hex_is_stable_and_64_hex_chars() {
        let f = write_temp_fixture(VALID_FIXTURE);
        let loader = FixtureLoader::new(f.path()).unwrap();
        assert_eq!(loader.sha256_hex().len(), 64);
        assert!(loader.sha256_hex().chars().all(|c| c.is_ascii_hexdigit()));

        let loader2 = FixtureLoader::new(f.path()).unwrap();
        assert_eq!(loader.sha256_hex(), loader2.sha256_hex());
    }

    #[test]
    fn rejects_non_synthetic_team_id() {
        let bad = r#"
        {
            "usage": [{
                "tenant_id": "demo", "budget_id": "b",
                "devin_team_id": "real-team-acme",
                "devin_session_id": "SESSION_FIXTURE_001",
                "acu_consumed": 1.0, "plan": "team",
                "window_start": "2026-06-01T00:00:00Z",
                "window_end": "2026-06-01T01:00:00Z"
            }]
        }"#;
        let f = write_temp_fixture(bad);
        let err = FixtureLoader::new(f.path()).unwrap_err();
        assert!(matches!(err, FixtureLoadError::NonSyntheticId { .. }));
    }

    #[test]
    fn rejects_non_synthetic_session_id() {
        let bad = r#"
        {
            "usage": [{
                "tenant_id": "demo", "budget_id": "b",
                "devin_team_id": "TEAM_FIXTURE_001",
                "devin_session_id": "sess-real-1234",
                "acu_consumed": 1.0, "plan": "team",
                "window_start": "2026-06-01T00:00:00Z",
                "window_end": "2026-06-01T01:00:00Z"
            }]
        }"#;
        let f = write_temp_fixture(bad);
        let err = FixtureLoader::new(f.path()).unwrap_err();
        assert!(matches!(err, FixtureLoadError::NonSyntheticId { .. }));
    }

    #[test]
    fn rejects_malformed_json() {
        let f = write_temp_fixture("{ this is not json }");
        let err = FixtureLoader::new(f.path()).unwrap_err();
        assert!(matches!(err, FixtureLoadError::Parse { .. }));
    }

    #[test]
    fn rejects_missing_file() {
        let path = Path::new("/tmp/spendguard-importer-devin-no-such-file");
        let err = FixtureLoader::new(path).unwrap_err();
        assert!(matches!(err, FixtureLoadError::Io { .. }));
    }

    #[test]
    fn synthetic_id_regex_helpers_are_exact() {
        assert!(is_synthetic_team_id("TEAM_FIXTURE_000"));
        assert!(is_synthetic_team_id("TEAM_FIXTURE_999"));
        assert!(!is_synthetic_team_id("TEAM_FIXTURE_99"));
        assert!(!is_synthetic_team_id("TEAM_FIXTURE_9999"));
        assert!(!is_synthetic_team_id("team_fixture_001"));
        assert!(!is_synthetic_team_id("REAL_TEAM_001"));

        assert!(is_synthetic_session_id("SESSION_FIXTURE_000"));
        assert!(!is_synthetic_session_id("SESSION_FIXTURE_AAA"));
    }

    #[test]
    fn empty_usage_array_is_ok() {
        let f = write_temp_fixture(r#"{ "usage": [] }"#);
        let loader = FixtureLoader::new(f.path()).unwrap();
        assert_eq!(loader.records().len(), 0);
    }
}
