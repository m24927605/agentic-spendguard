//! Markdown formatter (Phase A scaffold — full implementation in Phase B).
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §4.3.

use crate::formatters::FormatOptions;
use crate::report::Report;

pub fn render(report: &Report, _opts: &FormatOptions) -> String {
    format!(
        "# SpendGuard Calibration Report\n\n\
         **Tenant:** {}\n\n\
         **Window:** {} → {}\n\n\
         **Proof mode:** {}\n",
        report.tenant_id,
        report.window.from.to_rfc3339(),
        report.window.to.to_rfc3339(),
        report.proof_mode
    )
}
