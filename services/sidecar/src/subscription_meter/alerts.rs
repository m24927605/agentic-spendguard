//! D13 COV_63 — Subscription alert emission + cooldown.
//!
//! When a soft / hard cap fires the sidecar emits a canonical event
//! (`com.spendguard.subscription.alert.v1`).  To prevent alert storms
//! we honour the cooldown stored on the `subscription_alerts` row:
//! consecutive alerts for the same `(tenant_id, period_start,
//! severity)` triple within `cooldown_seconds` are suppressed.
//!
//! The cooldown semantics mirror the stats_aggregator drift_alert
//! pattern (D11 SLICE_05) so dashboard operators see the same backoff
//! shape across all alert classes.
//!
//! Spec: docs/specs/coverage/D13_subscription_meter/design.md §4.4

use chrono::{DateTime, Duration, Utc};

/// Severity tags written to subscription_alerts.severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertSeverity {
    SoftCap,
    HardCap,
}

impl AlertSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SoftCap => "soft_cap",
            Self::HardCap => "hard_cap",
        }
    }
}

/// Cooldown decision — emit the canonical event or suppress it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertDecision {
    /// Emit the canonical event AND update last_fired_at on the
    /// subscription_alerts row.  The returned cooldown_seconds is
    /// the duration the caller should record (= the row's cooldown
    /// or `default_cooldown_seconds` if no row exists yet).
    Emit { cooldown_seconds: i64 },
    /// Skip emission; the previous alert is still inside the cooldown
    /// window.  `remaining_seconds` says how long until the next
    /// emission is allowed.
    Suppress { remaining_seconds: i64 },
}

/// Default cooldown when no `subscription_alerts` row exists for the
/// (tenant, period_start, severity) triple.  Matches the schema
/// default (`cooldown_seconds INT NOT NULL DEFAULT 3600`).
pub const DEFAULT_COOLDOWN_SECONDS: i64 = 3_600;

/// Decide whether to emit an alert.
///
/// `last_fired_at` is the timestamp from the existing
/// `subscription_alerts` row (None when no row exists yet — the very
/// first alert always emits).  `row_cooldown_seconds` lets operators
/// override the default per-tenant.
pub fn should_emit_alert(
    last_fired_at: Option<DateTime<Utc>>,
    row_cooldown_seconds: Option<i64>,
    now: DateTime<Utc>,
) -> AlertDecision {
    let cooldown = row_cooldown_seconds
        .filter(|n| *n >= 0)
        .unwrap_or(DEFAULT_COOLDOWN_SECONDS);

    let Some(last) = last_fired_at else {
        return AlertDecision::Emit {
            cooldown_seconds: cooldown,
        };
    };

    let elapsed = now.signed_duration_since(last).num_seconds();
    if elapsed >= cooldown {
        AlertDecision::Emit {
            cooldown_seconds: cooldown,
        }
    } else {
        AlertDecision::Suppress {
            remaining_seconds: cooldown - elapsed,
        }
    }
}

/// Compute the cooldown deadline (= last_fired_at + cooldown) for the
/// audit chain.  Pure helper — callers use this when emitting the
/// canonical event so dashboards can show "next alert in 47m".
pub fn cooldown_deadline(last_fired_at: DateTime<Utc>, cooldown_seconds: i64) -> DateTime<Utc> {
    last_fired_at + Duration::seconds(cooldown_seconds.max(0))
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    #[test]
    fn first_alert_always_emits() {
        let r = should_emit_alert(None, None, t(2026, 6, 7, 12, 0));
        assert_eq!(
            r,
            AlertDecision::Emit {
                cooldown_seconds: DEFAULT_COOLDOWN_SECONDS
            }
        );
    }

    #[test]
    fn alert_inside_cooldown_is_suppressed() {
        let last = t(2026, 6, 7, 12, 0);
        let now = t(2026, 6, 7, 12, 30); // 30 min later — still inside 1h cooldown.
        let r = should_emit_alert(Some(last), None, now);
        assert!(
            matches!(r, AlertDecision::Suppress { remaining_seconds } if remaining_seconds == 1800)
        );
    }

    #[test]
    fn alert_at_exact_cooldown_boundary_emits() {
        let last = t(2026, 6, 7, 12, 0);
        let now = t(2026, 6, 7, 13, 0); // exactly 1h later.
        let r = should_emit_alert(Some(last), None, now);
        assert!(matches!(r, AlertDecision::Emit { .. }));
    }

    #[test]
    fn alert_past_cooldown_emits() {
        let last = t(2026, 6, 7, 12, 0);
        let now = t(2026, 6, 7, 14, 0);
        let r = should_emit_alert(Some(last), None, now);
        assert_eq!(
            r,
            AlertDecision::Emit {
                cooldown_seconds: DEFAULT_COOLDOWN_SECONDS
            }
        );
    }

    #[test]
    fn custom_cooldown_is_honoured() {
        let last = t(2026, 6, 7, 12, 0);
        let now = t(2026, 6, 7, 12, 10);
        // Operator override = 5 min cooldown.
        let r = should_emit_alert(Some(last), Some(300), now);
        assert!(matches!(
            r,
            AlertDecision::Emit {
                cooldown_seconds: 300
            }
        ));
    }

    #[test]
    fn negative_cooldown_falls_back_to_default() {
        let last = t(2026, 6, 7, 12, 0);
        let now = t(2026, 6, 7, 12, 30);
        let r = should_emit_alert(Some(last), Some(-1), now);
        // -1 ignored → DEFAULT (3600s) → still inside cooldown → suppress.
        assert!(matches!(r, AlertDecision::Suppress { .. }));
    }

    #[test]
    fn zero_cooldown_means_always_emit() {
        let last = t(2026, 6, 7, 12, 0);
        let now = t(2026, 6, 7, 12, 0);
        let r = should_emit_alert(Some(last), Some(0), now);
        assert!(matches!(
            r,
            AlertDecision::Emit {
                cooldown_seconds: 0
            }
        ));
    }

    #[test]
    fn severity_string_repr() {
        assert_eq!(AlertSeverity::SoftCap.as_str(), "soft_cap");
        assert_eq!(AlertSeverity::HardCap.as_str(), "hard_cap");
    }

    #[test]
    fn cooldown_deadline_adds_seconds() {
        let last = t(2026, 6, 7, 12, 0);
        let dl = cooldown_deadline(last, 3600);
        assert_eq!(dl, t(2026, 6, 7, 13, 0));
    }

    #[test]
    fn cooldown_deadline_clamps_negative_seconds() {
        let last = t(2026, 6, 7, 12, 0);
        let dl = cooldown_deadline(last, -1);
        assert_eq!(dl, last);
    }
}
