//! D15 — Wire-shape (`UsageRecord`, from admin REST) and internal shape
//! (`ImportRecord`, after tier + status validation).
//!
//! The split keeps deserialisation tolerant (extra fields ignored) and
//! downstream code total over a validated subset.
//!
//! ## Tier resolution (design §5 #6: unknown = skip + WARN)
//!
//! Three tiers, locked in `assets/price_table.toml`:
//!
//!   * `team_plan`       — public retail rate, $39/mo / 1900 credits
//!   * `enterprise`      — operator-override required at deploy time
//!   * `enterprise_byok` — load-bearing $0; BYOK customers pay LLM
//!                         provider directly
//!
//! Unknown tier strings produce
//! `ImporterError::UnknownTier(<other>)` at fixture-parse / live-poll
//! time and are skipped with a WARN (never fabricated to a default).

use chrono::{DateTime, Utc};
use serde::Deserialize;

/// Single session record from `GET /v1/usage`. Extra fields ignored
/// (forward-compat with vendor schema evolution).
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct UsageRecord {
    /// Vendor session identifier. Opaque.
    pub session_id: String,
    /// Vendor workspace / tenant identifier. Opaque.
    pub workspace_id: String,
    /// Vendor tier slug (`"team_plan"` / `"enterprise"` /
    /// `"enterprise_byok"`). Resolved to [`Tier`] at parse time.
    pub tier: String,
    /// Credits consumed during the window. MUST be >= 0.
    pub credits_consumed: i64,
    /// Status slug (`"completed"` / `"failed"` / `"cancelled"` /
    /// `"in_progress"`). Resolved to [`SessionStatus`].
    pub status: String,
    /// Session start (RFC 3339).
    pub started_at: DateTime<Utc>,
    /// Session end (RFC 3339).
    pub completed_at: DateTime<Utc>,
}

/// Validated, tier-resolved record. All downstream code uses this shape.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportRecord {
    /// Vendor session ID, opaque.
    pub session_id: String,
    /// Vendor workspace ID, opaque.
    pub workspace_id: String,
    /// Resolved tier.
    pub tier: Tier,
    /// Credits consumed; non-negative invariant enforced at validation.
    pub credits_consumed: i64,
    /// Resolved session status.
    pub status: SessionStatus,
    /// Start of the billing window.
    pub window_start: DateTime<Utc>,
    /// End of the billing window — used as `occurred_at` on the audit
    /// row so dashboards line up with the vendor's billing cutoff
    /// (design §3 / review-standards E7).
    pub window_end: DateTime<Utc>,
    /// Fixture vs live ingestion path.
    pub ingestion_mode: IngestionMode,
    /// SHA-256 of the fixture file when `ingestion_mode == Fixture`;
    /// `None` when `Live`. Review-standards E2 / S9 / T9.
    pub fixture_provenance_sha256: Option<String>,
}

/// Manus customer tier (design §1 + §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    /// `team_plan` — public retail rate ($39/mo / 1900 credits).
    TeamPlan,
    /// `enterprise` — operator-override required at deploy time.
    Enterprise,
    /// `enterprise_byok` — load-bearing $0; BYOK customers pay LLM
    /// provider directly.
    EnterpriseByok,
}

impl Tier {
    /// Stable wire string used in CloudEvent `data.tier`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TeamPlan => "team_plan",
            Self::Enterprise => "enterprise",
            Self::EnterpriseByok => "enterprise_byok",
        }
    }

    /// Parse the canonical wire string. Returns `None` for unknown
    /// values — caller emits a WARN and skips the row
    /// (review-standards T6).
    pub fn from_wire(s: &str) -> Option<Self> {
        Some(match s {
            "team_plan" => Self::TeamPlan,
            "enterprise" => Self::Enterprise,
            "enterprise_byok" => Self::EnterpriseByok,
            _ => return None,
        })
    }
}

/// Lifecycle state of a Manus session.
///
/// `InProgress` sessions are LOADED but NOT emitted as audit rows in
/// the demo path (review-standards E3 — loader stays general so live
/// mode can surface in-flight sessions in a future "what's in flight"
/// dashboard variant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionStatus {
    /// Session finished cleanly.
    Completed,
    /// Session aborted with an error.
    Failed,
    /// Session cancelled by user / vendor.
    Cancelled,
    /// Session still in flight; importer skips this row in the demo
    /// path.
    InProgress,
}

impl SessionStatus {
    /// Stable wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::InProgress => "in_progress",
        }
    }

    /// Parse the canonical wire string.
    pub fn from_wire(s: &str) -> Option<Self> {
        Some(match s {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "in_progress" => Self::InProgress,
            _ => return None,
        })
    }

    /// Whether the demo path should emit this row. `InProgress` is
    /// skipped per design §3 / acceptance §5 / review-standards E3.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// Whether the `ImportRecord` came from a sanitized fixture (default
/// merge gate) or from the live Manus admin REST API (gated on `live`
/// Cargo feature + `MANUS_API_TOKEN`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IngestionMode {
    /// Fixture replay — default merge gate. Carries
    /// `fixture_provenance_sha256`.
    Fixture,
    /// Live admin REST pull. `fixture_provenance_sha256` is `None`.
    Live,
}

impl IngestionMode {
    /// Stable wire string used in CloudEvent `data.ingestion_mode`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fixture => "fixture",
            Self::Live => "live",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_as_str_round_trip_team_plan() {
        let t = Tier::TeamPlan;
        assert_eq!(t.as_str(), "team_plan");
        assert_eq!(Tier::from_wire(t.as_str()), Some(t));
    }

    #[test]
    fn tier_as_str_round_trip_enterprise() {
        let t = Tier::Enterprise;
        assert_eq!(t.as_str(), "enterprise");
        assert_eq!(Tier::from_wire(t.as_str()), Some(t));
    }

    #[test]
    fn tier_as_str_round_trip_enterprise_byok() {
        let t = Tier::EnterpriseByok;
        assert_eq!(t.as_str(), "enterprise_byok");
        assert_eq!(Tier::from_wire(t.as_str()), Some(t));
    }

    #[test]
    fn tier_from_wire_unknown_returns_none() {
        // T6: unknown tier never silently maps to a default.
        assert_eq!(Tier::from_wire("solo"), None);
        assert_eq!(Tier::from_wire("free"), None);
        assert_eq!(Tier::from_wire(""), None);
        assert_eq!(Tier::from_wire("TEAM_PLAN"), None);
    }

    #[test]
    fn session_status_round_trip_each_variant() {
        for s in [
            SessionStatus::Completed,
            SessionStatus::Failed,
            SessionStatus::Cancelled,
            SessionStatus::InProgress,
        ] {
            assert_eq!(SessionStatus::from_wire(s.as_str()), Some(s));
        }
    }

    #[test]
    fn session_status_terminal_skips_in_progress() {
        // E3: in-flight rows skipped in demo path.
        assert!(SessionStatus::Completed.is_terminal());
        assert!(SessionStatus::Failed.is_terminal());
        assert!(SessionStatus::Cancelled.is_terminal());
        assert!(!SessionStatus::InProgress.is_terminal());
    }

    #[test]
    fn ingestion_mode_wire_strings_match_spec() {
        assert_eq!(IngestionMode::Fixture.as_str(), "fixture");
        assert_eq!(IngestionMode::Live.as_str(), "live");
    }

    #[test]
    fn usage_record_deserialises_with_extra_fields_ignored() {
        // Forward-compat: extra fields in admin response must not fail.
        let raw = r#"{
            "session_id": "ses_FAKE_x",
            "workspace_id": "ws_FAKE_x",
            "tier": "team_plan",
            "credits_consumed": 47,
            "status": "completed",
            "started_at": "2026-06-05T14:22:08Z",
            "completed_at": "2026-06-05T14:34:51Z",
            "vendor_added_in_future": "ignored"
        }"#;
        let r: UsageRecord = serde_json::from_str(raw).unwrap();
        assert_eq!(r.tier, "team_plan");
        assert_eq!(r.credits_consumed, 47);
    }
}
