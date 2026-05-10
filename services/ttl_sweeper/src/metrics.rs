//! Round-2 #11: Prometheus metrics for TTL sweeper daemon.
//!
//! Mirrors `services/outbox_forwarder/src/metrics.rs` shape. Loop
//! iteration outcomes + skip reasons + per-reservation sweep
//! outcomes drive an SLO query like "what fraction of expired
//! reservations get released within K poll intervals?".

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
pub struct TtlSweeperMetricsInner {
    loop_outcomes: [AtomicU64; 3],
    skip_reasons: [AtomicU64; 3],
    swept_ok: AtomicU64,
    swept_err: AtomicU64,
}

#[derive(Clone, Default)]
pub struct TtlSweeperMetrics {
    pub inner: Arc<TtlSweeperMetricsInner>,
}

impl TtlSweeperMetrics {
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

    pub fn add_swept(&self, n: u64, ok: bool) {
        let target = if ok {
            &self.inner.swept_ok
        } else {
            &self.inner.swept_err
        };
        target.fetch_add(n, Ordering::Relaxed);
    }

    pub fn render(&self) -> String {
        let mut out = String::with_capacity(1024);
        out.push_str("# HELP spendguard_ttl_sweeper_loop_iterations_total TTL sweeper poll-loop iterations broken out by outcome.\n");
        out.push_str("# TYPE spendguard_ttl_sweeper_loop_iterations_total counter\n");
        for outcome in [LoopOutcome::Processed, LoopOutcome::Skipped, LoopOutcome::Error] {
            let i = match outcome {
                LoopOutcome::Processed => 0,
                LoopOutcome::Skipped => 1,
                LoopOutcome::Error => 2,
            };
            let v = self.inner.loop_outcomes[i].load(Ordering::Relaxed);
            out.push_str(&format!(
                "spendguard_ttl_sweeper_loop_iterations_total{{outcome=\"{}\"}} {}\n",
                outcome.as_str(),
                v,
            ));
        }
        out.push_str("# HELP spendguard_ttl_sweeper_skip_total TTL sweeper skip reasons (non-leader path).\n");
        out.push_str("# TYPE spendguard_ttl_sweeper_skip_total counter\n");
        for reason in [SkipReason::LeaseExpired, SkipReason::Standby, SkipReason::Unknown] {
            let i = match reason {
                SkipReason::LeaseExpired => 0,
                SkipReason::Standby => 1,
                SkipReason::Unknown => 2,
            };
            let v = self.inner.skip_reasons[i].load(Ordering::Relaxed);
            out.push_str(&format!(
                "spendguard_ttl_sweeper_skip_total{{reason=\"{}\"}} {}\n",
                reason.as_str(),
                v,
            ));
        }
        out.push_str("# HELP spendguard_ttl_sweeper_reservations_swept_total Reservations swept (TTL release) broken out by outcome.\n");
        out.push_str("# TYPE spendguard_ttl_sweeper_reservations_swept_total counter\n");
        out.push_str(&format!(
            "spendguard_ttl_sweeper_reservations_swept_total{{outcome=\"ok\"}} {}\n",
            self.inner.swept_ok.load(Ordering::Relaxed),
        ));
        out.push_str(&format!(
            "spendguard_ttl_sweeper_reservations_swept_total{{outcome=\"err\"}} {}\n",
            self.inner.swept_err.load(Ordering::Relaxed),
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_counters_render() {
        let m = TtlSweeperMetrics::new();
        m.inc_loop(LoopOutcome::Processed);
        m.inc_loop(LoopOutcome::Skipped);
        m.inc_loop(LoopOutcome::Error);
        let txt = m.render();
        assert!(txt.contains("loop_iterations_total{outcome=\"processed\"} 1"));
        assert!(txt.contains("loop_iterations_total{outcome=\"error\"} 1"));
    }

    #[test]
    fn swept_counters_render() {
        let m = TtlSweeperMetrics::new();
        m.add_swept(7, true);
        m.add_swept(2, false);
        let txt = m.render();
        assert!(txt.contains("reservations_swept_total{outcome=\"ok\"} 7"));
        assert!(txt.contains("reservations_swept_total{outcome=\"err\"} 2"));
    }

    #[test]
    fn render_has_help_and_type_lines() {
        let m = TtlSweeperMetrics::new();
        let txt = m.render();
        assert!(txt.contains("# HELP spendguard_ttl_sweeper_loop_iterations_total"));
        assert!(txt.contains("# TYPE spendguard_ttl_sweeper_skip_total counter"));
    }

    #[test]
    fn shared_state_is_thread_safe() {
        let m = TtlSweeperMetrics::new();
        let m2 = m.clone();
        m.inc_loop(LoopOutcome::Processed);
        m2.inc_loop(LoopOutcome::Processed);
        let txt = m.render();
        assert!(txt.contains("loop_iterations_total{outcome=\"processed\"} 2"));
    }

    #[test]
    fn skip_reason_counters_render() {
        let m = TtlSweeperMetrics::new();
        m.inc_skip(SkipReason::Standby);
        m.inc_skip(SkipReason::Standby);
        let txt = m.render();
        assert!(txt.contains("skip_total{reason=\"standby\"} 2"));
    }
}
