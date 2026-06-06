//! `spendguard calibration-report` CLI core library.
//!
//! Spec: `docs/calibration-report-spec-v1alpha1.md`.
//! Slice: `docs/slices/SLICE_13_calibration_report_cli.md`.
//!
//! ## Architecture (per spec §1.4 v1alpha1 核心哲學)
//!
//! - **Operator-facing, not marketing**: CLI is callable from on-call
//!   bash; subcommands are simple; exit codes are CI-parseable.
//! - **Cryptographic proof on demand**: `--proof-mode=canonical` plus
//!   `--verify-chain` runs the audit-chain replay verifier (per
//!   `audit-chain-prediction-extension-v1alpha1.md` §7) so the report
//!   is bound to signed evidence.
//! - **Recommendations are heuristic**: each rule pairs a "possible
//!   cause" with a "suggested action" (per spec §8.2). Never
//!   prescriptive.
//! - **No real-time dashboard**: this is a one-shot binary; a streaming
//!   dashboard is an out-of-scope deferral (slice §10).
//!
//! ## Library shape
//!
//! - `cli`: clap-derived CLI surface (§2).
//! - `sql_queries`: typed SQL builders that route through the RLS
//!   tenant-scope `SET LOCAL app.current_tenant_id` discipline (SLICE_06
//!   R2 B1).
//! - `formatters`: text / JSON / Markdown renderers (§4).
//! - `recommendations`: 9-rule heuristic engine (§8.1).
//! - `verify_chain_wrapper`: bridge to the canonical_ingest library's
//!   verifier (Phase C — landed alongside the lib.rs export).
//! - `report`: aggregate Report type returned by the orchestrator.
//!
//! Each module documents its own spec ancestor inline.

pub mod cli;
pub mod formatters;
pub mod recommendations;
pub mod report;
pub mod self_audit;
pub mod sql_queries;
pub mod verify_chain_wrapper;

pub use cli::{Cli, Format, ProofMode, Subcommand};
pub use report::{CalibrationRatio, DriftAlert, Report, ReportExitCode, TierDistribution};
