//! Round-2 #11: Prometheus metrics for outbox forwarder.
//!
//! Daemon-loop service. Counters surface (a) per-iteration outcomes
//! (processed / skipped / error) and (b) per-row forwarding outcomes
//! (ok / err) so an SLO query can compute "what fraction of pending
//! rows are getting through?".

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopOutcome {
    Processed,
    Skipped,
    Error,
}

impl LoopOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Processed => "processed",
            Self::Skipped => "skipped",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    LeaseExpired,
    Standby,
    Unknown,
}

impl SkipReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LeaseExpired => "lease_expired",
            Self::Standby => "standby",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Default)]
pub struct OutboxForwarderMetricsInner {
    /// 3 outcomes: Processed, Skipped, Error.
    loop_outcomes: [AtomicU64; 3],
    /// 3 skip reasons: LeaseExpired, Standby, Unknown.
    skip_reasons: [AtomicU64; 3],
    /// Total rows forwarded successfully + total rows that errored.
    rows_forwarded_ok: AtomicU64,
    rows_forwarded_err: AtomicU64,
}

#[derive(Clone, Default)]
pub struct OutboxForwarderMetrics {
    pub inner: Arc<OutboxForwarderMetricsInner>,
}

impl OutboxForwarderMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_loop(&self, outcome: LoopOutcome) {
        let i = match outcome {
            LoopOutcome::Processed => 0,
            LoopOutcome::Skipped => 1,
            LoopOutcome::Error => 2,
        };
        self.inner.loop_outcomes[i].fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_skip(&self, reason: SkipReason) {
        let i = match reason {
            SkipReason::LeaseExpired => 0,
            SkipReason::Standby => 1,
            SkipReason::Unknown => 2,
        };
        self.inner.skip_reasons[i].fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_rows_forwarded(&self, n: u64, ok: bool) {
        let target = if ok {
            &self.inner.rows_forwarded_ok
        } else {
            &self.inner.rows_forwarded_err
        };
        target.fetch_add(n, Ordering::Relaxed);
    }

    pub fn render(&self) -> String {
        let mut out = String::with_capacity(1024);
        out.push_str("# HELP spendguard_outbox_forwarder_loop_iterations_total Outbox forwarder poll-loop iterations broken out by outcome.\n");
        out.push_str("# TYPE spendguard_outbox_forwarder_loop_iterations_total counter\n");
        for outcome in [LoopOutcome::Processed, LoopOutcome::Skipped, LoopOutcome::Error] {
            let i = match outcome {
                LoopOutcome::Processed => 0,
                LoopOutcome::Skipped => 1,
                LoopOutcome::Error => 2,
            };
            let v = self.inner.loop_outcomes[i].load(Ordering::Relaxed);
            out.push_str(&format!(
                "spendguard_outbox_forwarder_loop_iterations_total{{outcome=\"{}\"}} {}\n",
                outcome.as_str(),
                v,
            ));
        }
        out.push_str("# HELP spendguard_outbox_forwarder_skip_total Outbox forwarder skip reasons (non-leader path).\n");
        out.push_str("# TYPE spendguard_outbox_forwarder_skip_total counter\n");
        for reason in [SkipReason::LeaseExpired, SkipReason::Standby, SkipReason::Unknown] {
            let i = match reason {
                SkipReason::LeaseExpired => 0,
                SkipReason::Standby => 1,
                SkipReason::Unknown => 2,
            };
            let v = self.inner.skip_reasons[i].load(Ordering::Relaxed);
            out.push_str(&format!(
                "spendguard_outbox_forwarder_skip_total{{reason=\"{}\"}} {}\n",
                reason.as_str(),
                v,
            ));
        }
        out.push_str("# HELP spendguard_outbox_forwarder_rows_forwarded_total Outbox audit rows forwarded to canonical_ingest, broken out by outcome.\n");
        out.push_str("# TYPE spendguard_outbox_forwarder_rows_forwarded_total counter\n");
        out.push_str(&format!(
            "spendguard_outbox_forwarder_rows_forwarded_total{{outcome=\"ok\"}} {}\n",
            self.inner.rows_forwarded_ok.load(Ordering::Relaxed),
        ));
        out.push_str(&format!(
            "spendguard_outbox_forwarder_rows_forwarded_total{{outcome=\"err\"}} {}\n",
            self.inner.rows_forwarded_err.load(Ordering::Relaxed),
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_counters_render() {
        let m = OutboxForwarderMetrics::new();
        m.inc_loop(LoopOutcome::Processed);
        m.inc_loop(LoopOutcome::Processed);
        m.inc_loop(LoopOutcome::Skipped);
        m.inc_loop(LoopOutcome::Error);
        let txt = m.render();
        assert!(txt.contains("loop_iterations_total{outcome=\"processed\"} 2"));
        assert!(txt.contains("loop_iterations_total{outcome=\"skipped\"} 1"));
        assert!(txt.contains("loop_iterations_total{outcome=\"error\"} 1"));
    }

    #[test]
    fn skip_reason_counters_render() {
        let m = OutboxForwarderMetrics::new();
        m.inc_skip(SkipReason::Standby);
        m.inc_skip(SkipReason::Standby);
        m.inc_skip(SkipReason::LeaseExpired);
        let txt = m.render();
        assert!(txt.contains("skip_total{reason=\"standby\"} 2"));
        assert!(txt.contains("skip_total{reason=\"lease_expired\"} 1"));
    }

    #[test]
    fn rows_forwarded_counters_render() {
        let m = OutboxForwarderMetrics::new();
        m.add_rows_forwarded(5, true);
        m.add_rows_forwarded(3, true);
        m.add_rows_forwarded(2, false);
        let txt = m.render();
        assert!(txt.contains("rows_forwarded_total{outcome=\"ok\"} 8"));
        assert!(txt.contains("rows_forwarded_total{outcome=\"err\"} 2"));
    }

    #[test]
    fn render_has_help_and_type_lines() {
        let m = OutboxForwarderMetrics::new();
        let txt = m.render();
        assert!(txt.contains("# HELP spendguard_outbox_forwarder_loop_iterations_total"));
        assert!(txt.contains("# TYPE spendguard_outbox_forwarder_skip_total counter"));
        assert!(txt.contains("# HELP spendguard_outbox_forwarder_rows_forwarded_total"));
    }

    #[test]
    fn shared_state_is_thread_safe() {
        let m = OutboxForwarderMetrics::new();
        let m2 = m.clone();
        m.inc_loop(LoopOutcome::Processed);
        m2.inc_loop(LoopOutcome::Processed);
        let txt = m.render();
        assert!(txt.contains("loop_iterations_total{outcome=\"processed\"} 2"));
    }
}
