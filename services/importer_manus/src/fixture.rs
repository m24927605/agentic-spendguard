//! D15 COV_72 — Fixture-mode loader. Reads a canonical sanitized
//! `manus_usage.json` snapshot and yields [`ImportRecord`]s with
//! `ingestion_mode = Fixture` plus the file's SHA-256 hash stamped on
//! every record (review-standards E1 / T8 / T9).
//!
//! The fixture path is the **default merge gate** (design §5 #2). Live
//! mode is feature-gated and CI-optional.
//!
//! ## Synthetic-ID invariant (review-standards T8)
//!
//! Workspace IDs MUST match `^ws_FAKE_…$` and session IDs MUST match
//! `^ses_FAKE_…$`. Real customer IDs are hard-rejected at parse time
//! so a future PR that drops in real data fails CI before merge.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::ImporterError;
use crate::record::{ImportRecord, IngestionMode, SessionStatus, Tier, UsageRecord};

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
    /// invariant (review-standards T8).
    #[error("fixture {path} contains non-synthetic ID at index {index}: {id}")]
    NonSyntheticId {
        /// Path that failed.
        path: PathBuf,
        /// Which record in the array.
        index: usize,
        /// The offending ID.
        id: String,
    },
    /// A record carried a tier this importer does not know (T6).
    #[error("fixture {path} contains unknown tier at index {index}: {tier}")]
    UnknownTier {
        /// Path that failed.
        path: PathBuf,
        /// Which record in the array.
        index: usize,
        /// The offending tier slug.
        tier: String,
    },
    /// A record carried a session status this importer does not know.
    #[error("fixture {path} contains unknown status at index {index}: {status}")]
    UnknownStatus {
        /// Path that failed.
        path: PathBuf,
        /// Which record in the array.
        index: usize,
        /// The offending status slug.
        status: String,
    },
    /// A record had `credits_consumed < 0`.
    #[error("fixture {path} contains negative credits at index {index}: {credits}")]
    NegativeCredits {
        /// Path that failed.
        path: PathBuf,
        /// Which record in the array.
        index: usize,
        /// The offending value.
        credits: i64,
    },
}

/// Top-level fixture file shape.
#[derive(Debug, Deserialize)]
struct FixtureFile {
    sessions: Vec<UsageRecord>,
    #[serde(default)]
    #[allow(dead_code)]
    next_cursor: Option<String>,
}

/// Fixture loader. Holds the parsed records + the file's SHA-256 hash.
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

        let mut records = Vec::with_capacity(file.sessions.len());
        for (index, row) in file.sessions.into_iter().enumerate() {
            if !is_synthetic_workspace_id(&row.workspace_id) {
                return Err(FixtureLoadError::NonSyntheticId {
                    path: path.to_path_buf(),
                    index,
                    id: row.workspace_id,
                });
            }
            if !is_synthetic_session_id(&row.session_id) {
                return Err(FixtureLoadError::NonSyntheticId {
                    path: path.to_path_buf(),
                    index,
                    id: row.session_id,
                });
            }
            if row.credits_consumed < 0 {
                return Err(FixtureLoadError::NegativeCredits {
                    path: path.to_path_buf(),
                    index,
                    credits: row.credits_consumed,
                });
            }
            let tier = Tier::from_wire(&row.tier).ok_or_else(|| FixtureLoadError::UnknownTier {
                path: path.to_path_buf(),
                index,
                tier: row.tier.clone(),
            })?;
            let status = SessionStatus::from_wire(&row.status).ok_or_else(|| {
                FixtureLoadError::UnknownStatus {
                    path: path.to_path_buf(),
                    index,
                    status: row.status.clone(),
                }
            })?;
            records.push(ImportRecord {
                session_id: row.session_id,
                workspace_id: row.workspace_id,
                tier,
                credits_consumed: row.credits_consumed,
                status,
                window_start: row.started_at,
                window_end: row.completed_at,
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

    /// Returns the SHA-256 of the fixture file (hex, lowercase).
    pub fn sha256_hex(&self) -> &str {
        &self.sha256_hex
    }

    /// Path the fixture was loaded from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Parsed records. Every record carries `ingestion_mode = Fixture`
    /// and the fixture's SHA-256.
    pub fn records(&self) -> &[ImportRecord] {
        &self.records
    }

    /// Move records out for callers that need owned data.
    pub fn into_records(self) -> Vec<ImportRecord> {
        self.records
    }

    /// Records the demo / production path should EMIT — i.e. drop
    /// in-flight rows (review-standards E3).
    pub fn terminal_records(&self) -> impl Iterator<Item = &ImportRecord> {
        self.records.iter().filter(|r| r.status.is_terminal())
    }
}

/// Validate a `UsageRecord` from a live HTTP response, returning either
/// a fully-resolved `ImportRecord` (tagged `IngestionMode::Live`) or an
/// `ImporterError` that the caller should WARN + skip.
///
/// Live mode never carries a `fixture_provenance_sha256` — that field
/// is `None` for every live row (review-standards mirror to D14
/// fixture-vs-live invariant).
pub fn validate_record_public(rec: UsageRecord) -> Result<ImportRecord, ImporterError> {
    if rec.credits_consumed < 0 {
        return Err(ImporterError::NegativeCredits);
    }
    let tier = Tier::from_wire(&rec.tier).ok_or(ImporterError::UnknownTier(rec.tier.clone()))?;
    let status = SessionStatus::from_wire(&rec.status)
        .ok_or(ImporterError::UnknownStatus(rec.status.clone()))?;
    Ok(ImportRecord {
        session_id: rec.session_id,
        workspace_id: rec.workspace_id,
        tier,
        credits_consumed: rec.credits_consumed,
        status,
        window_start: rec.started_at,
        window_end: rec.completed_at,
        ingestion_mode: IngestionMode::Live,
        fixture_provenance_sha256: None,
    })
}

fn sha256_hex_of(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

fn is_synthetic_workspace_id(s: &str) -> bool {
    // Sentinel `ws_FAKE_…` — review-standards T8 / A10.4.
    s.starts_with("ws_FAKE_") && s.len() > "ws_FAKE_".len()
}

fn is_synthetic_session_id(s: &str) -> bool {
    // Sentinel `ses_FAKE_…` — review-standards T8 / A10.5.
    s.starts_with("ses_FAKE_") && s.len() > "ses_FAKE_".len()
}

#[allow(unused_imports)]
pub(crate) use validate_record_public as validate_record_for_live;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::io::Write;

    fn write_temp_fixture(json: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(json.as_bytes()).unwrap();
        f
    }

    const VALID_FIXTURE: &str = r#"
    {
        "sessions": [
            {
                "session_id": "ses_FAKE_unit_001",
                "workspace_id": "ws_FAKE_unit_001",
                "tier": "team_plan",
                "credits_consumed": 47,
                "status": "completed",
                "started_at": "2026-06-05T14:22:08Z",
                "completed_at": "2026-06-05T14:34:51Z"
            },
            {
                "session_id": "ses_FAKE_unit_002",
                "workspace_id": "ws_FAKE_unit_001",
                "tier": "enterprise",
                "credits_consumed": 350,
                "status": "completed",
                "started_at": "2026-06-05T09:11:00Z",
                "completed_at": "2026-06-05T11:48:00Z"
            }
        ],
        "next_cursor": null
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
            "sessions": [{
                "session_id": "ses_FAKE_unit_001",
                "workspace_id": "real-ws-acme",
                "tier": "team_plan", "credits_consumed": 1,
                "status": "completed",
                "started_at": "2026-06-05T00:00:00Z",
                "completed_at": "2026-06-05T01:00:00Z"
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
            "sessions": [{
                "session_id": "real-sess-1",
                "workspace_id": "ws_FAKE_unit_001",
                "tier": "team_plan", "credits_consumed": 1,
                "status": "completed",
                "started_at": "2026-06-05T00:00:00Z",
                "completed_at": "2026-06-05T01:00:00Z"
            }]
        }"#;
        let f = write_temp_fixture(bad);
        let err = FixtureLoader::new(f.path()).unwrap_err();
        assert!(matches!(err, FixtureLoadError::NonSyntheticId { .. }));
    }

    #[test]
    fn rejects_unknown_tier() {
        let bad = r#"
        {
            "sessions": [{
                "session_id": "ses_FAKE_unit_001",
                "workspace_id": "ws_FAKE_unit_001",
                "tier": "solo", "credits_consumed": 1,
                "status": "completed",
                "started_at": "2026-06-05T00:00:00Z",
                "completed_at": "2026-06-05T01:00:00Z"
            }]
        }"#;
        let f = write_temp_fixture(bad);
        let err = FixtureLoader::new(f.path()).unwrap_err();
        assert!(matches!(err, FixtureLoadError::UnknownTier { .. }));
    }

    #[test]
    fn rejects_unknown_status() {
        let bad = r#"
        {
            "sessions": [{
                "session_id": "ses_FAKE_unit_001",
                "workspace_id": "ws_FAKE_unit_001",
                "tier": "team_plan", "credits_consumed": 1,
                "status": "paused_for_review",
                "started_at": "2026-06-05T00:00:00Z",
                "completed_at": "2026-06-05T01:00:00Z"
            }]
        }"#;
        let f = write_temp_fixture(bad);
        let err = FixtureLoader::new(f.path()).unwrap_err();
        assert!(matches!(err, FixtureLoadError::UnknownStatus { .. }));
    }

    #[test]
    fn rejects_negative_credits() {
        let bad = r#"
        {
            "sessions": [{
                "session_id": "ses_FAKE_unit_001",
                "workspace_id": "ws_FAKE_unit_001",
                "tier": "team_plan", "credits_consumed": -5,
                "status": "completed",
                "started_at": "2026-06-05T00:00:00Z",
                "completed_at": "2026-06-05T01:00:00Z"
            }]
        }"#;
        let f = write_temp_fixture(bad);
        let err = FixtureLoader::new(f.path()).unwrap_err();
        assert!(matches!(err, FixtureLoadError::NegativeCredits { .. }));
    }

    #[test]
    fn rejects_malformed_json() {
        let f = write_temp_fixture("{ this is not json }");
        let err = FixtureLoader::new(f.path()).unwrap_err();
        assert!(matches!(err, FixtureLoadError::Parse { .. }));
    }

    #[test]
    fn rejects_missing_file() {
        let path = Path::new("/tmp/spendguard-importer-manus-no-such-file");
        let err = FixtureLoader::new(path).unwrap_err();
        assert!(matches!(err, FixtureLoadError::Io { .. }));
    }

    #[test]
    fn synthetic_id_helpers_are_exact() {
        assert!(is_synthetic_workspace_id("ws_FAKE_team_001"));
        assert!(is_synthetic_workspace_id("ws_FAKE_byok_001"));
        assert!(!is_synthetic_workspace_id("ws_FAKE_"));
        assert!(!is_synthetic_workspace_id("WS_FAKE_001"));
        assert!(!is_synthetic_workspace_id("real-ws"));

        assert!(is_synthetic_session_id("ses_FAKE_unit_001"));
        assert!(!is_synthetic_session_id("ses_FAKE_"));
        assert!(!is_synthetic_session_id("SES_FAKE_001"));
    }

    #[test]
    fn empty_sessions_array_is_ok() {
        let f = write_temp_fixture(r#"{ "sessions": [] }"#);
        let loader = FixtureLoader::new(f.path()).unwrap();
        assert_eq!(loader.records().len(), 0);
    }

    #[test]
    fn terminal_records_skips_in_progress() {
        let with_inprogress = r#"
        {
            "sessions": [
                {
                    "session_id": "ses_FAKE_done_001",
                    "workspace_id": "ws_FAKE_team_001",
                    "tier": "team_plan", "credits_consumed": 47,
                    "status": "completed",
                    "started_at": "2026-06-05T14:22:08Z",
                    "completed_at": "2026-06-05T14:34:51Z"
                },
                {
                    "session_id": "ses_FAKE_flight_002",
                    "workspace_id": "ws_FAKE_team_001",
                    "tier": "team_plan", "credits_consumed": 8,
                    "status": "in_progress",
                    "started_at": "2026-06-05T17:00:00Z",
                    "completed_at": "2026-06-05T17:00:00Z"
                }
            ]
        }"#;
        let f = write_temp_fixture(with_inprogress);
        let loader = FixtureLoader::new(f.path()).unwrap();
        // Loader keeps all rows (general-purpose).
        assert_eq!(loader.records().len(), 2);
        // But the demo / commit path filters to terminal only.
        let terminal: Vec<_> = loader.terminal_records().collect();
        assert_eq!(terminal.len(), 1);
        assert_eq!(terminal[0].status, SessionStatus::Completed);
    }

    #[test]
    fn validate_record_public_for_live_tags_live_mode() {
        let raw = UsageRecord {
            session_id: "ses_FAKE_unit_001".into(),
            workspace_id: "ws_FAKE_unit_001".into(),
            tier: "team_plan".into(),
            credits_consumed: 47,
            status: "completed".into(),
            started_at: Utc.with_ymd_and_hms(2026, 6, 5, 14, 22, 8).unwrap(),
            completed_at: Utc.with_ymd_and_hms(2026, 6, 5, 14, 34, 51).unwrap(),
        };
        let r = validate_record_public(raw).unwrap();
        assert_eq!(r.ingestion_mode, IngestionMode::Live);
        assert_eq!(r.fixture_provenance_sha256, None);
        assert_eq!(r.tier, Tier::TeamPlan);
    }

    #[test]
    fn validate_record_public_returns_unknown_tier_for_warn_skip() {
        // T6: WARN + skip via Err in live path.
        let raw = UsageRecord {
            session_id: "ses_FAKE_unit_001".into(),
            workspace_id: "ws_FAKE_unit_001".into(),
            tier: "solo".into(),
            credits_consumed: 1,
            status: "completed".into(),
            started_at: Utc.with_ymd_and_hms(2026, 6, 5, 0, 0, 0).unwrap(),
            completed_at: Utc.with_ymd_and_hms(2026, 6, 5, 1, 0, 0).unwrap(),
        };
        let err = validate_record_public(raw).unwrap_err();
        assert!(matches!(err, ImporterError::UnknownTier(_)));
    }
}
