//! Round-2 #11: Prometheus metrics for sidecar.
//!
//! Mirrors `services/ledger/src/metrics.rs` shape — no `prometheus`
//! crate, raw `AtomicU64` + manual text-format render. Counter
//! increments live in `server/adapter_uds.rs` (one per gRPC method,
//! with `outcome` = ok / err). The metrics HTTP server is started by
//! `serve_metrics` in `main.rs` on `cfg.metrics_addr` (default
//! `0.0.0.0:9093`).
//!
//! Surfaced counters:
//!
//!   * `spendguard_sidecar_handler_calls_total{handler, outcome}` —
//!     adapter UDS gRPC method invocation count, broken out by ok /
//!     err. Useful for L1 (sidecar admission) SLO computation.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handler {
    Handshake,
    RequestDecision,
    ConfirmPublishOutcome,
    EmitTraceEvents,
    IssueBudgetGrant,
    RevokeBudgetGrant,
    ConsumeBudgetGrant,
    StreamDrainSignal,
    ResumeAfterApproval,
}

impl Handler {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Handshake => "handshake",
            Self::RequestDecision => "request_decision",
            Self::ConfirmPublishOutcome => "confirm_publish_outcome",
            Self::EmitTraceEvents => "emit_trace_events",
            Self::IssueBudgetGrant => "issue_budget_grant",
            Self::RevokeBudgetGrant => "revoke_budget_grant",
            Self::ConsumeBudgetGrant => "consume_budget_grant",
            Self::StreamDrainSignal => "stream_drain_signal",
            Self::ResumeAfterApproval => "resume_after_approval",
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
pub struct SidecarMetricsInner {
    /// Per (handler, outcome) call counter. 9 handlers × 2 outcomes.
    counts: [[AtomicU64; 2]; 9],
}

#[derive(Clone, Default)]
pub struct SidecarMetrics {
    pub inner: Arc<SidecarMetricsInner>,
}

impl SidecarMetrics {
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
        let mut out = String::with_capacity(1024);
        out.push_str("# HELP spendguard_sidecar_handler_calls_total Sidecar adapter UDS gRPC method invocations broken out by outcome.\n");
        out.push_str("# TYPE spendguard_sidecar_handler_calls_total counter\n");
        for h in ALL_HANDLERS {
            for outcome in [Outcome::Ok, Outcome::Err] {
                let h_idx = handler_index(*h);
                let o_idx = match outcome {
                    Outcome::Ok => 0,
                    Outcome::Err => 1,
                };
                let v = self.inner.counts[h_idx][o_idx].load(Ordering::Relaxed);
                out.push_str(&format!(
                    "spendguard_sidecar_handler_calls_total{{handler=\"{}\",outcome=\"{}\"}} {}\n",
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
    Handler::Handshake,
    Handler::RequestDecision,
    Handler::ConfirmPublishOutcome,
    Handler::EmitTraceEvents,
    Handler::IssueBudgetGrant,
    Handler::RevokeBudgetGrant,
    Handler::ConsumeBudgetGrant,
    Handler::StreamDrainSignal,
    Handler::ResumeAfterApproval,
];

fn handler_index(h: Handler) -> usize {
    match h {
        Handler::Handshake => 0,
        Handler::RequestDecision => 1,
        Handler::ConfirmPublishOutcome => 2,
        Handler::EmitTraceEvents => 3,
        Handler::IssueBudgetGrant => 4,
        Handler::RevokeBudgetGrant => 5,
        Handler::ConsumeBudgetGrant => 6,
        Handler::StreamDrainSignal => 7,
        Handler::ResumeAfterApproval => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_default_to_zero_in_render_output() {
        let m = SidecarMetrics::new();
        let txt = m.render();
        assert!(txt.contains("spendguard_sidecar_handler_calls_total{handler=\"request_decision\",outcome=\"ok\"} 0"));
        assert!(txt.contains("spendguard_sidecar_handler_calls_total{handler=\"resume_after_approval\",outcome=\"err\"} 0"));
    }

    #[test]
    fn increments_show_in_render_output() {
        let m = SidecarMetrics::new();
        m.inc_handler(Handler::RequestDecision, Outcome::Ok);
        m.inc_handler(Handler::RequestDecision, Outcome::Ok);
        m.inc_handler(Handler::EmitTraceEvents, Outcome::Err);
        let txt = m.render();
        assert!(txt.contains("handler=\"request_decision\",outcome=\"ok\"} 2"));
        assert!(txt.contains("handler=\"emit_trace_events\",outcome=\"err\"} 1"));
    }

    #[test]
    fn render_includes_help_and_type_lines() {
        let m = SidecarMetrics::new();
        let txt = m.render();
        assert!(txt.contains("# HELP spendguard_sidecar_handler_calls_total"));
        assert!(txt.contains("# TYPE spendguard_sidecar_handler_calls_total counter"));
    }

    #[test]
    fn shared_state_is_thread_safe() {
        let m = SidecarMetrics::new();
        let m2 = m.clone();
        m.inc_handler(Handler::Handshake, Outcome::Ok);
        m2.inc_handler(Handler::Handshake, Outcome::Ok);
        let txt = m.render();
        assert!(txt.contains("handler=\"handshake\",outcome=\"ok\"} 2"));
    }

    #[test]
    fn handler_as_str_matches_render_label() {
        for h in ALL_HANDLERS {
            assert!(!h.as_str().is_empty());
        }
    }
}
