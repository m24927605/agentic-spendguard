//! Phase B placeholder — full Strategy A arrives in Phase C with the
//! TOML loader integration.
//!
//! Spec ref output-predictor-service-spec-v1alpha1.md §3 (algorithm) +
//! §3.4 (invariant: A is always callable + always > 0 + ≤ context_window).

/// `predicted_a_tokens = min(
///     max_tokens_requested if > 0 else INFINITY,
///     model_context_window - input_tokens
/// )`
///
/// Invariant per spec §3.4: A is **always** > 0 and **always** callable.
/// Edge cases:
///   * `max_tokens_requested = 0` (proto3 default / "unset") → INFINITY
///     side of min; the formula collapses to (context_window - input).
///   * `input_tokens >= context_window` → cap at 1 (defensive floor —
///     spec demands A > 0; reservation safety net).
///   * negative inputs (shouldn't happen but defensive) → cap at 1.
pub fn compute_a(max_tokens_requested: i64, model_context_window: i64, input_tokens: i64) -> i64 {
    let headroom = model_context_window.saturating_sub(input_tokens);
    let candidate = if max_tokens_requested > 0 {
        std::cmp::min(max_tokens_requested, headroom)
    } else {
        // max_tokens_requested unset (proto3 default 0) → INFINITY in
        // the formula collapses to the headroom side of min.
        headroom
    };
    // Spec §3.4 invariant: A > 0. Defensive floor against pathological
    // inputs (input_tokens > context_window, or negative context_window).
    if candidate <= 0 {
        1
    } else {
        candidate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typical_case_max_tokens_set() {
        // max_tokens=500 < (128k - 1k) = 127k → result is 500.
        assert_eq!(compute_a(500, 128_000, 1000), 500);
    }

    #[test]
    fn typical_case_max_tokens_unset() {
        // max_tokens=0 → INFINITY → headroom = 128k - 1k = 127k.
        assert_eq!(compute_a(0, 128_000, 1000), 127_000);
    }

    #[test]
    fn ceiling_clamps_to_headroom() {
        // max_tokens=999999 > headroom=127000 → result = headroom.
        assert_eq!(compute_a(999_999, 128_000, 1000), 127_000);
    }

    #[test]
    fn defensive_floor_when_input_exceeds_window() {
        // input_tokens > context_window → headroom <= 0 → A = 1 (invariant).
        assert_eq!(compute_a(500, 128_000, 200_000), 1);
        assert_eq!(compute_a(0, 128_000, 200_000), 1);
    }

    #[test]
    fn defensive_floor_when_context_window_zero() {
        // context_window = 0 (unknown model + no fallback) → headroom <= 0 → A = 1.
        assert_eq!(compute_a(0, 0, 0), 1);
    }

    #[test]
    fn small_window_small_input() {
        // 8000 (unknown default) - 100 input → headroom 7900; max_tokens=2000
        // → result is min(2000, 7900) = 2000.
        assert_eq!(compute_a(2000, 8000, 100), 2000);
    }
}
