//! D16 COV_86 — Fixture-mode loader. Reads a canonical sanitized
//! `genspark_usage.json` snapshot and yields `ImportRecord`s with
//! `ingestion_mode = Fixture` plus the file's SHA-256 hash stamped on
//! every record (review-standards P3, T9).
//!
//! The fixture path is the **default merge gate** (design §4). Live
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
    /// invariant (review-standards T9).
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
/// * `usage[]` — one row per `(workspace, task, window)` triple.
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
    workspace_id: String,
    task_id: String,
    credits_consumed: f64,
    plan: String,
    #[serde(default)]
    task_category: Option<String>,
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

        // Synthetic-ID enforcement (review-standards T9). The fixture
        // is committed; non-synthetic IDs would mean a real customer's
        // data leaked. Hard fail.
        let mut records = Vec::with_capacity(file.usage.len());
        for (index, row) in file.usage.into_iter().enumerate() {
            if !is_synthetic_workspace_id(&row.workspace_id) {
                return Err(FixtureLoadError::NonSyntheticId {
                    path: path.to_path_buf(),
                    index,
                    id: row.workspace_id,
                });
            }
            if !is_synthetic_task_id(&row.task_id) {
                return Err(FixtureLoadError::NonSyntheticId {
                    path: path.to_path_buf(),
                    index,
                    id: row.task_id,
                });
            }
            records.push(ImportRecord {
                tenant_id: row.tenant_id,
                budget_id: row.budget_id,
                workspace_id: row.workspace_id,
                task_id: row.task_id,
                credits_consumed: row.credits_consumed,
                plan: row.plan,
                task_category: row.task_category,
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

fn is_synthetic_workspace_id(s: &str) -> bool {
    // Review-standards T9: `^FAKE_ws_\d{3}$`.
    s.starts_with("FAKE_ws_")
        && s.len() == "FAKE_ws_".len() + 3
        && s["FAKE_ws_".len()..].chars().all(|c| c.is_ascii_digit())
}

fn is_synthetic_task_id(s: &str) -> bool {
    s.starts_with("FAKE_task_")
        && s.len() == "FAKE_task_".len() + 3
        && s["FAKE_task_".len()..]
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
                "budget_id": "genspark-budget",
                "workspace_id": "FAKE_ws_001",
                "task_id": "FAKE_task_001",
                "credits_consumed": 3200.0,
                "plan": "plus",
                "task_category": "research",
                "window_start": "2026-06-01T00:00:00Z",
                "window_end": "2026-06-01T01:00:00Z"
            },
            {
                "tenant_id": "demo",
                "budget_id": "genspark-budget",
                "workspace_id": "FAKE_ws_002",
                "task_id": "FAKE_task_002",
                "credits_consumed": 50000.0,
                "plan": "premium",
                "task_category": "code_generation",
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
            assert_eq!(r.fixture_provenance_sha256.as_deref().unwrap().len(), 64);
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
    fn rejects_non_synthetic_workspace_id() {
        let bad = r#"
        {
            "usage": [{
                "tenant_id": "demo", "budget_id": "b",
                "workspace_id": "real-workspace-acme",
                "task_id": "FAKE_task_001",
                "credits_consumed": 1.0, "plan": "plus",
                "window_start": "2026-06-01T00:00:00Z",
                "window_end": "2026-06-01T01:00:00Z"
            }]
        }"#;
        let f = write_temp_fixture(bad);
        let err = FixtureLoader::new(f.path()).unwrap_err();
        assert!(matches!(err, FixtureLoadError::NonSyntheticId { .. }));
    }

    #[test]
    fn rejects_non_synthetic_task_id() {
        let bad = r#"
        {
            "usage": [{
                "tenant_id": "demo", "budget_id": "b",
                "workspace_id": "FAKE_ws_001",
                "task_id": "task-real-1234",
                "credits_consumed": 1.0, "plan": "plus",
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
        let path = Path::new("/tmp/spendguard-importer-genspark-no-such-file");
        let err = FixtureLoader::new(path).unwrap_err();
        assert!(matches!(err, FixtureLoadError::Io { .. }));
    }

    #[test]
    fn synthetic_id_regex_helpers_are_exact() {
        assert!(is_synthetic_workspace_id("FAKE_ws_000"));
        assert!(is_synthetic_workspace_id("FAKE_ws_999"));
        assert!(!is_synthetic_workspace_id("FAKE_ws_99"));
        assert!(!is_synthetic_workspace_id("FAKE_ws_9999"));
        assert!(!is_synthetic_workspace_id("fake_ws_001"));
        assert!(!is_synthetic_workspace_id("REAL_WS_001"));

        assert!(is_synthetic_task_id("FAKE_task_000"));
        assert!(!is_synthetic_task_id("FAKE_task_AAA"));
        assert!(!is_synthetic_task_id("real_task_001"));
    }

    #[test]
    fn empty_usage_array_is_ok() {
        let f = write_temp_fixture(r#"{ "usage": [] }"#);
        let loader = FixtureLoader::new(f.path()).unwrap();
        assert_eq!(loader.records().len(), 0);
    }

    #[test]
    fn task_category_optional() {
        // task_category MAY be absent on a fixture row.
        let no_cat = r#"
        {
            "usage": [{
                "tenant_id": "demo", "budget_id": "b",
                "workspace_id": "FAKE_ws_001",
                "task_id": "FAKE_task_001",
                "credits_consumed": 1.0, "plan": "plus",
                "window_start": "2026-06-01T00:00:00Z",
                "window_end": "2026-06-01T01:00:00Z"
            }]
        }"#;
        let f = write_temp_fixture(no_cat);
        let loader = FixtureLoader::new(f.path()).unwrap();
        assert_eq!(loader.records().len(), 1);
        assert_eq!(loader.records()[0].task_category, None);
    }

    #[test]
    fn fixture_tolerates_unknown_fields() {
        // T8: parser does NOT use deny_unknown_fields — extra fields
        // from the vendor must not break CI.
        let extra = r#"
        {
            "_meta": { "extra_field": "tolerated" },
            "usage": [{
                "tenant_id": "demo", "budget_id": "b",
                "workspace_id": "FAKE_ws_001",
                "task_id": "FAKE_task_001",
                "credits_consumed": 1.0, "plan": "plus",
                "task_category": "research",
                "window_start": "2026-06-01T00:00:00Z",
                "window_end": "2026-06-01T01:00:00Z",
                "future_field": "vendor_added_this"
            }]
        }"#;
        let f = write_temp_fixture(extra);
        let loader = FixtureLoader::new(f.path()).unwrap();
        assert_eq!(loader.records().len(), 1);
    }
}
