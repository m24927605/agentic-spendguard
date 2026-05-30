//! JSON formatter (Phase A scaffold — full implementation in Phase B).
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §4.2.

use crate::formatters::FormatOptions;
use crate::report::Report;

pub fn render(report: &Report, _opts: &FormatOptions) -> String {
    serde_json::to_string_pretty(report).expect("Report serializes to JSON")
}
