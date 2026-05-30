//! Output formatters for the calibration report.
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §4.
//!
//! Three formats:
//!   * `text` — default; monospace human-readable (§4.1).
//!   * `json` — structured, SIEM-stable (§4.2).
//!   * `markdown` — Slack / Confluence / GitHub paste (§4.3).
//!
//! Each formatter takes a `Report` + a `format_options` struct and
//! returns a `String`. Caller writes that string to stdout or a file
//! per `--output`.

pub mod json;
pub mod markdown;
pub mod text;

use crate::report::Report;

/// Options the orchestrator passes to all formatters.
#[derive(Debug, Clone)]
pub struct FormatOptions {
    /// Whether to include the §8 recommendation section. Default
    /// follows `Cli::effective_include_recommendations()`.
    pub include_recommendations: bool,
    /// Whether the `--verify-chain` flag ran. Affects the trailing
    /// integrity-attestation block in text/markdown.
    pub verify_chain_run: bool,
}

pub fn render(report: &Report, format: crate::cli::Format, opts: &FormatOptions) -> String {
    match format {
        crate::cli::Format::Text => text::render(report, opts),
        crate::cli::Format::Json => json::render(report, opts),
        crate::cli::Format::Markdown => markdown::render(report, opts),
    }
}
