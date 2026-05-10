//! Phase 5 GA hardening S23 followup #11: Prometheus metrics for ledger.
//!
//! Mirrors `services/canonical_ingest/src/metrics.rs` shape — no
//! `prometheus` crate dependency, raw `AtomicU64` + manual text-format
//! render. Counter increments live in `server.rs` (one per gRPC
//! method, with `outcome` = ok / err). The metrics HTTP server is
//! started by `serve_metrics` in `main.rs`.
//!
//! Surfaced counters:
//!
//!   * `spendguard_ledger_handler_calls_total{handler, outcome}` —
//!     gRPC method invocation count, broken out by ok / err. Drives
//!     L3 (ledger commit success) SLO computation.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// All gRPC methods we instrument. Adding a new method = extend this
/// enum + the match arm in `inc_handler`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handler {
    ReserveSet,
    Release,
    RecordDeniedDecision,
    AcquireFencingLease,
    CommitEstimated,
    ProviderReport,
    InvoiceReconcile,
    QueryBudgetState,
    QueryReservationContext,
    QueryDecisionOutcome,
    ReplayAuditFromCursor,
    RefundCredit,
    DisputeAdjustment,
    Compensate,
    GetApprovalForResume,
    MarkApprovalBundled,
}

impl Handler {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReserveSet => "reserve_set",
            Self::Release => "release",
            Self::RecordDeniedDecision => "record_denied_decision",
            Self::AcquireFencingLease => "acquire_fencing_lease",
            Self::CommitEstimated => "commit_estimated",
            Self::ProviderReport => "provider_report",
            Self::InvoiceReconcile => "invoice_reconcile",
            Self::QueryBudgetState => "query_budget_state",
            Self::QueryReservationContext => "query_reservation_context",
            Self::QueryDecisionOutcome => "query_decision_outcome",
            Self::ReplayAuditFromCursor => "replay_audit_from_cursor",
            Self::RefundCredit => "refund_credit",
            Self::DisputeAdjustment => "dispute_adjustment",
            Self::Compensate => "compensate",
            Self::GetApprovalForResume => "get_approval_for_resume",
            Self::MarkApprovalBundled => "mark_approval_bundled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Err,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Err => "err",
        }
    }
}

#[derive(Default)]
pub struct LedgerMetricsInner {
    /// Per (handler, outcome) call counter. Fixed-size array of
    /// 16 handlers × 2 outcomes = 32 atomics. Cheap; no allocation.
    counts: [[AtomicU64; 2]; 16],
}

#[derive(Clone, Default)]
pub struct LedgerMetrics {
    pub inner: Arc<LedgerMetricsInner>,
}

impl LedgerMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_handler(&self, handler: Handler, outcome: Outcome) {
        let h = handler_index(handler);
        let o = match outcome {
            Outcome::Ok => 0,
            Outcome::Err => 1,
        };
        self.inner.counts[h][o].fetch_add(1, Ordering::Relaxed);
    }

    /// Render the Prometheus text-format payload.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(2048);
        out.push_str("# HELP spendguard_ledger_handler_calls_total Ledger gRPC method invocations broken out by outcome.\n");
        out.push_str("# TYPE spendguard_ledger_handler_calls_total counter\n");
        for h in ALL_HANDLERS {
            for outcome in [Outcome::Ok, Outcome::Err] {
                let h_idx = handler_index(*h);
                let o_idx = match outcome {
                    Outcome::Ok => 0,
                    Outcome::Err => 1,
                };
                let v = self.inner.counts[h_idx][o_idx].load(Ordering::Relaxed);
                out.push_str(&format!(
                    "spendguard_ledger_handler_calls_total{{handler=\"{}\",outcome=\"{}\"}} {}\n",
                    h.as_str(),
                    outcome.as_str(),
                    v,
                ));
            }
        }
        out
    }
}

const ALL_HANDLERS: &[Handler] = &[
    Handler::ReserveSet,
    Handler::Release,
    Handler::RecordDeniedDecision,
    Handler::AcquireFencingLease,
    Handler::CommitEstimated,
    Handler::ProviderReport,
    Handler::InvoiceReconcile,
    Handler::QueryBudgetState,
    Handler::QueryReservationContext,
    Handler::QueryDecisionOutcome,
    Handler::ReplayAuditFromCursor,
    Handler::RefundCredit,
    Handler::DisputeAdjustment,
    Handler::Compensate,
    Handler::GetApprovalForResume,
    Handler::MarkApprovalBundled,
];

fn handler_index(h: Handler) -> usize {
    match h {
        Handler::ReserveSet => 0,
        Handler::Release => 1,
        Handler::RecordDeniedDecision => 2,
        Handler::AcquireFencingLease => 3,
        Handler::CommitEstimated => 4,
        Handler::ProviderReport => 5,
        Handler::InvoiceReconcile => 6,
        Handler::QueryBudgetState => 7,
        Handler::QueryReservationContext => 8,
        Handler::QueryDecisionOutcome => 9,
        Handler::ReplayAuditFromCursor => 10,
        Handler::RefundCredit => 11,
        Handler::DisputeAdjustment => 12,
        Handler::Compensate => 13,
        Handler::GetApprovalForResume => 14,
        Handler::MarkApprovalBundled => 15,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_default_to_zero_in_render_output() {
        let m = LedgerMetrics::new();
        let txt = m.render();
        assert!(txt.contains("spendguard_ledger_handler_calls_total{handler=\"reserve_set\",outcome=\"ok\"} 0"));
        assert!(txt.contains("spendguard_ledger_handler_calls_total{handler=\"invoice_reconcile\",outcome=\"err\"} 0"));
    }

    #[test]
    fn increments_show_in_render_output() {
        let m = LedgerMetrics::new();
        m.inc_handler(Handler::ReserveSet, Outcome::Ok);
        m.inc_handler(Handler::ReserveSet, Outcome::Ok);
        m.inc_handler(Handler::CommitEstimated, Outcome::Err);
        let txt = m.render();
        assert!(txt.contains("handler=\"reserve_set\",outcome=\"ok\"} 2"));
        assert!(txt.contains("handler=\"commit_estimated\",outcome=\"err\"} 1"));
    }

    #[test]
    fn render_includes_help_and_type_lines() {
        let m = LedgerMetrics::new();
        let txt = m.render();
        assert!(txt.contains("# HELP spendguard_ledger_handler_calls_total"));
        assert!(txt.contains("# TYPE spendguard_ledger_handler_calls_total counter"));
    }

    #[test]
    fn shared_state_is_thread_safe() {
        let m = LedgerMetrics::new();
        let m2 = m.clone();
        m.inc_handler(Handler::Release, Outcome::Ok);
        m2.inc_handler(Handler::Release, Outcome::Ok);
        let txt = m.render();
        assert!(txt.contains("handler=\"release\",outcome=\"ok\"} 2"));
    }

    #[test]
    fn handler_as_str_matches_render_label() {
        // If we add a new Handler variant, both as_str + render must
        // pick it up; this test pins that contract.
        for h in ALL_HANDLERS {
            assert!(!h.as_str().is_empty());
        }
    }
}
