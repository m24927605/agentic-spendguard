//! Strategy selector per spec output-predictor-service-spec-v1alpha1.md §6.
//!
//! The selector takes the active `prediction_policy` and the three
//! computed strategy values, and returns:
//!
//!   * `reserved_strategy` — what the reservation will actually use
//!     (the safety value committed to the ledger).
//!   * `prediction_strategy_used` — the best strategy available given
//!     the policy. May differ from reserved_strategy under STRICT_CEILING
//!     (where reservation is always A but prediction_used = B/C when
//!     available) — see spec §6.2 for the calibration-report rationale.
//!
//! SLICE_06 ships the full algorithm; SLICE_07 layers Strategy C in.

use std::fmt;

/// Identifier for the chosen strategy. Mirrors the wire string values
/// (A | B | C) directly so `to_string()` is the audit row value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    A,
    B,
    C,
}

impl fmt::Display for Strategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Strategy::A => f.write_str("A"),
            Strategy::B => f.write_str("B"),
            Strategy::C => f.write_str("C"),
        }
    }
}

/// Per spec §6.1 algorithm. Returns (reserved_strategy, prediction_strategy_used).
///
/// `a` is the always-computed Strategy A value (never None). `b` / `c`
/// are the optional Strategy B / C values; None means the strategy was
/// not available for this call (cache miss for B; plugin disabled/failed
/// for C).
///
/// Policy semantics:
///   * `STRICT_CEILING`: reservation always uses A (safest); prediction_used
///     records the best available B/C so calibration can backtest
///     "what if customer switched to EMPIRICAL_RUN_CEILING".
///   * `EMPIRICAL_RUN_CEILING`: same as STRICT_CEILING — reservation
///     uses A, but prediction_used records B/C if available. Per spec
///     §6.1: the reservation actually uses A under both policies; the
///     distinction is enforced at the contract layer for run-scoped
///     adjustments (handled by run_cost_projector in SLICE_09).
///   * `ADAPTIVE_CEILING`: reservation switches to B/C when available;
///     falls back to A only when both are None.
///   * `SHADOW_ONLY`: reservation always A; prediction_used always A
///     too (shadow mode = log but don't change behaviour).
///
/// Unknown policy → STRICT_CEILING (conservative fallback per spec §6.1
/// default branch).
pub fn select_strategy(
    policy: &str,
    _a: i64,
    b: Option<i64>,
    c: Option<i64>,
) -> (Strategy, Strategy) {
    let prefer = prefer_c_then_b_then_a(c, b);
    match policy {
        "STRICT_CEILING" => (Strategy::A, prefer),
        "EMPIRICAL_RUN_CEILING" => (Strategy::A, prefer),
        "ADAPTIVE_CEILING" => (prefer, prefer),
        "SHADOW_ONLY" => (Strategy::A, Strategy::A),
        _ => (Strategy::A, Strategy::A), // conservative default
    }
}

/// "Prefer C over B over A" — per spec §6.1 prefer_c_then_b_then_a.
fn prefer_c_then_b_then_a(c: Option<i64>, b: Option<i64>) -> Strategy {
    if c.is_some() {
        Strategy::C
    } else if b.is_some() {
        Strategy::B
    } else {
        Strategy::A
    }
}

/// Boundary validation — reject unknown policies at the gRPC handler so
/// callers see InvalidArgument instead of a silent fallback. Per spec §6.1
/// the selector still falls back to STRICT_CEILING for unknown values as a
/// defense-in-depth measure, but the handler should not normally rely on it.
pub fn is_known_policy(policy: &str) -> bool {
    matches!(
        policy,
        "STRICT_CEILING" | "EMPIRICAL_RUN_CEILING" | "ADAPTIVE_CEILING" | "SHADOW_ONLY"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_ceiling_reserves_a_even_when_b_some() {
        // Spec §6.2: reserved must be A under STRICT_CEILING regardless
        // of B/C presence. prediction_used records the would-be choice.
        let (reserved, used) = select_strategy("STRICT_CEILING", 100, Some(200), None);
        assert_eq!(reserved, Strategy::A);
        assert_eq!(used, Strategy::B);
    }

    #[test]
    fn strict_ceiling_with_c_records_c_in_used() {
        let (reserved, used) = select_strategy("STRICT_CEILING", 100, Some(200), Some(150));
        assert_eq!(reserved, Strategy::A);
        assert_eq!(used, Strategy::C);
    }

    #[test]
    fn empirical_run_ceiling_reserves_a_records_b_in_used() {
        let (reserved, used) = select_strategy("EMPIRICAL_RUN_CEILING", 100, Some(200), None);
        assert_eq!(reserved, Strategy::A);
        assert_eq!(used, Strategy::B);
    }

    #[test]
    fn adaptive_ceiling_switches_reservation_to_b() {
        let (reserved, used) = select_strategy("ADAPTIVE_CEILING", 100, Some(200), None);
        assert_eq!(reserved, Strategy::B);
        assert_eq!(used, Strategy::B);
    }

    #[test]
    fn adaptive_ceiling_switches_reservation_to_c_when_available() {
        let (reserved, used) = select_strategy("ADAPTIVE_CEILING", 100, Some(200), Some(150));
        assert_eq!(reserved, Strategy::C);
        assert_eq!(used, Strategy::C);
    }

    #[test]
    fn adaptive_ceiling_falls_back_to_a_when_both_none() {
        let (reserved, used) = select_strategy("ADAPTIVE_CEILING", 100, None, None);
        assert_eq!(reserved, Strategy::A);
        assert_eq!(used, Strategy::A);
    }

    #[test]
    fn shadow_only_always_uses_a() {
        // Even with B and C available, SHADOW_ONLY pins both to A.
        let (reserved, used) = select_strategy("SHADOW_ONLY", 100, Some(200), Some(150));
        assert_eq!(reserved, Strategy::A);
        assert_eq!(used, Strategy::A);
    }

    #[test]
    fn unknown_policy_defaults_to_a() {
        let (reserved, used) = select_strategy("UNRECOGNISED_POLICY", 100, Some(200), Some(150));
        assert_eq!(reserved, Strategy::A);
        assert_eq!(used, Strategy::A);
    }

    #[test]
    fn is_known_policy_rejects_garbage() {
        assert!(is_known_policy("STRICT_CEILING"));
        assert!(is_known_policy("EMPIRICAL_RUN_CEILING"));
        assert!(is_known_policy("ADAPTIVE_CEILING"));
        assert!(is_known_policy("SHADOW_ONLY"));
        assert!(!is_known_policy("STRICT-ceiling"));
        assert!(!is_known_policy(""));
        assert!(!is_known_policy("foo"));
    }

    #[test]
    fn display_strategy() {
        assert_eq!(Strategy::A.to_string(), "A");
        assert_eq!(Strategy::B.to_string(), "B");
        assert_eq!(Strategy::C.to_string(), "C");
    }
}
