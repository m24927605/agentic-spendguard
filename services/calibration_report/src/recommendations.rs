//! Recommendation engine (Phase A scaffold — full 9-rule
//! implementation in Phase B).
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §8.1.

use crate::report::{Recommendation, Report};

pub fn evaluate(_report: &Report) -> Vec<Recommendation> {
    // Phase A: empty; Phase B implements all 9 rules.
    Vec::new()
}
