//! D13 COV_64 — Hard-cap synthetic 429 short circuit.
//!
//! Evaluates the cap thresholds on a (projected) `consumed_atomic`
//! value and returns one of:
//!
//!   * `CapDecision::Pass`         — under all thresholds, CONTINUE
//!   * `CapDecision::SoftCapAlert` — at/over `alert_at_atomic`, EMIT
//!                                   ALERT but CONTINUE
//!   * `CapDecision::HardCapBlock` — at/over `hard_cap_at_atomic`,
//!                                   DENY with synthetic 429 + payload
//!
//! The synthetic 429 body shape is vendor-matched so CLIs (claude-cli,
//! codex_cli_rs) treat it identically to a vendor rate-limit response
//! and exit cleanly.  A distinct `code = "spendguard_subscription_cap"`
//! distinguishes SpendGuard-injected from vendor-injected 429s.
//!
//! Spec: docs/specs/coverage/D13_subscription_meter/design.md §4.5

use chrono::{DateTime, Utc};

/// Upper bound on the synthetic Retry-After header (design §8 decision
/// 10 — prevents misconfigured windows asking CLIs to wait > 24h).
pub const HARD_CAP_RETRY_AFTER_MAX_SECONDS: i64 = 86_400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapDecision {
    Pass,
    SoftCapAlert {
        threshold_atomic: i64,
        projected_atomic: i64,
    },
    HardCapBlock {
        threshold_atomic: i64,
        projected_atomic: i64,
        retry_after_seconds: i64,
    },
}

impl CapDecision {
    pub fn is_block(&self) -> bool {
        matches!(self, CapDecision::HardCapBlock { .. })
    }

    pub fn is_alert(&self) -> bool {
        matches!(self, CapDecision::SoftCapAlert { .. })
    }
}

/// Aggregate evaluation outcome — the decision plus the inputs that
/// produced it.  Useful for the audit_outbox row.
#[derive(Debug, Clone)]
pub struct CapEvaluation {
    pub decision: CapDecision,
    pub current_consumed_atomic: i64,
    pub projected_consumed_atomic: i64,
    pub alert_at_atomic: i64,
    pub hard_cap_at_atomic: Option<i64>,
}

/// Evaluate the cap thresholds.
///
/// `current_consumed_atomic` is the already-committed meter total for
/// the current window. `delta_atomic` is the amount we are about to
/// add for THIS request.  The decision is taken on the projected
/// (current + delta) value so callers can short-circuit BEFORE
/// persisting the increment.
///
/// Hard-cap takes precedence over soft-cap when both fire.
pub fn evaluate_cap(
    current_consumed_atomic: i64,
    delta_atomic: i64,
    alert_at_atomic: i64,
    hard_cap_at_atomic: Option<i64>,
    period_end: DateTime<Utc>,
    now: DateTime<Utc>,
) -> CapEvaluation {
    let current = current_consumed_atomic.max(0);
    let delta = delta_atomic.max(0);
    let projected = current.saturating_add(delta);

    // Hard cap first — DENY beats ALERT.
    if let Some(hard) = hard_cap_at_atomic {
        if hard > 0 && projected >= hard {
            let secs_until_reset = period_end.signed_duration_since(now).num_seconds();
            let retry_after = secs_until_reset.clamp(1, HARD_CAP_RETRY_AFTER_MAX_SECONDS);
            return CapEvaluation {
                decision: CapDecision::HardCapBlock {
                    threshold_atomic: hard,
                    projected_atomic: projected,
                    retry_after_seconds: retry_after,
                },
                current_consumed_atomic: current,
                projected_consumed_atomic: projected,
                alert_at_atomic,
                hard_cap_at_atomic,
            };
        }
    }

    if alert_at_atomic > 0 && projected >= alert_at_atomic {
        return CapEvaluation {
            decision: CapDecision::SoftCapAlert {
                threshold_atomic: alert_at_atomic,
                projected_atomic: projected,
            },
            current_consumed_atomic: current,
            projected_consumed_atomic: projected,
            alert_at_atomic,
            hard_cap_at_atomic,
        };
    }

    CapEvaluation {
        decision: CapDecision::Pass,
        current_consumed_atomic: current,
        projected_consumed_atomic: projected,
        alert_at_atomic,
        hard_cap_at_atomic,
    }
}

/// Build the vendor-shaped synthetic 429 body the egress proxy
/// returns when a hard-cap fires.  Anthropic uses `error.type`,
/// OpenAI uses `error.code` — we populate both so either CLI treats
/// it as a normal rate-limit.
pub fn synthetic_429_body() -> &'static str {
    r#"{"error":{"type":"rate_limit_exceeded","message":"spendguard subscription cap reached","code":"spendguard_subscription_cap"}}"#
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn under_all_thresholds_is_pass() {
        let ev = evaluate_cap(
            100,        // current
            50,         // delta
            1000,       // alert_at
            Some(2000), // hard_cap
            t(2026, 7, 1, 0),
            t(2026, 6, 7, 12),
        );
        assert_eq!(ev.decision, CapDecision::Pass);
        assert_eq!(ev.projected_consumed_atomic, 150);
    }

    #[test]
    fn at_or_above_alert_is_soft_alert() {
        let ev = evaluate_cap(
            950,
            50,
            1000,
            Some(2000),
            t(2026, 7, 1, 0),
            t(2026, 6, 7, 12),
        );
        assert!(matches!(
            ev.decision,
            CapDecision::SoftCapAlert {
                threshold_atomic: 1000,
                projected_atomic: 1000
            }
        ));
    }

    #[test]
    fn at_or_above_hard_cap_is_block() {
        let ev = evaluate_cap(
            1900,
            200,
            1000,
            Some(2000),
            t(2026, 7, 1, 0),
            t(2026, 6, 7, 12),
        );
        assert!(ev.decision.is_block());
        if let CapDecision::HardCapBlock {
            threshold_atomic,
            projected_atomic,
            retry_after_seconds,
        } = ev.decision
        {
            assert_eq!(threshold_atomic, 2000);
            assert_eq!(projected_atomic, 2100);
            assert!(retry_after_seconds > 0);
            assert!(retry_after_seconds <= HARD_CAP_RETRY_AFTER_MAX_SECONDS);
        }
    }

    #[test]
    fn hard_cap_beats_soft_alert() {
        // Projected crosses BOTH thresholds — block wins.
        let ev = evaluate_cap(
            0,
            5000,
            1000,
            Some(2000),
            t(2026, 7, 1, 0),
            t(2026, 6, 7, 12),
        );
        assert!(ev.decision.is_block());
        assert!(!ev.decision.is_alert());
    }

    #[test]
    fn no_hard_cap_configured_only_soft_alerts() {
        let ev = evaluate_cap(900, 200, 1000, None, t(2026, 7, 1, 0), t(2026, 6, 7, 12));
        assert!(ev.decision.is_alert());
    }

    #[test]
    fn no_thresholds_configured_is_always_pass() {
        let ev = evaluate_cap(
            10_000_000,
            100,
            0,
            None,
            t(2026, 7, 1, 0),
            t(2026, 6, 7, 12),
        );
        assert_eq!(ev.decision, CapDecision::Pass);
    }

    #[test]
    fn retry_after_is_clamped_to_24h() {
        // period_end 2 years out, retry_after should clamp at 86400.
        let ev = evaluate_cap(
            2100,
            0,
            1000,
            Some(2000),
            t(2028, 6, 7, 12),
            t(2026, 6, 7, 12),
        );
        if let CapDecision::HardCapBlock {
            retry_after_seconds,
            ..
        } = ev.decision
        {
            assert_eq!(retry_after_seconds, HARD_CAP_RETRY_AFTER_MAX_SECONDS);
        } else {
            panic!("expected hard cap block");
        }
    }

    #[test]
    fn retry_after_is_at_least_one_second_when_already_past_window() {
        // period_end in the past — clamp returns >= 1 so CLIs don't
        // see Retry-After: 0 and hammer immediately.
        let ev = evaluate_cap(
            2100,
            0,
            1000,
            Some(2000),
            t(2026, 6, 6, 12),
            t(2026, 6, 7, 12),
        );
        if let CapDecision::HardCapBlock {
            retry_after_seconds,
            ..
        } = ev.decision
        {
            assert!(retry_after_seconds >= 1);
        } else {
            panic!("expected hard cap block");
        }
    }

    #[test]
    fn synthetic_429_payload_carries_distinguishing_code() {
        let body = synthetic_429_body();
        assert!(body.contains("rate_limit_exceeded"));
        assert!(body.contains("spendguard_subscription_cap"));
        // Must be valid JSON.
        let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(parsed["error"]["code"], "spendguard_subscription_cap");
        assert_eq!(parsed["error"]["type"], "rate_limit_exceeded");
    }

    #[test]
    fn current_consumed_clamps_negative_to_zero() {
        let ev = evaluate_cap(
            -50,
            100,
            1000,
            Some(2000),
            t(2026, 7, 1, 0),
            t(2026, 6, 7, 12),
        );
        assert_eq!(ev.current_consumed_atomic, 0);
        assert_eq!(ev.projected_consumed_atomic, 100);
    }
}
