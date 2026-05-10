//! Round-2 #11: Prometheus metrics for usage poller daemon.
//!
//! Counters surface (a) per-cycle outcomes (ok/err), and (b)
//! per-cycle record counts (fetched/inserted/deduped) so an SLO
//! query can tell whether the poller is keeping up with the
//! provider's usage stream.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleOutcome {
    Ok,
    Err,
}

impl CycleOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Err => "err",
        }
    }
}

#[derive(Default)]
pub struct UsagePollerMetricsInner {
    cycles_ok: AtomicU64,
    cycles_err: AtomicU64,
    records_fetched: AtomicU64,
    records_inserted: AtomicU64,
    records_deduped: AtomicU64,
}

#[derive(Clone, Default)]
pub struct UsagePollerMetrics {
    pub inner: Arc<UsagePollerMetricsInner>,
}

impl UsagePollerMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_cycle(&self, outcome: CycleOutcome) {
        let target = match outcome {
            CycleOutcome::Ok => &self.inner.cycles_ok,
            CycleOutcome::Err => &self.inner.cycles_err,
        };
        target.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_records(&self, fetched: u64, inserted: u64, deduped: u64) {
        self.inner.records_fetched.fetch_add(fetched, Ordering::Relaxed);
        self.inner.records_inserted.fetch_add(inserted, Ordering::Relaxed);
        self.inner.records_deduped.fetch_add(deduped, Ordering::Relaxed);
    }

    pub fn render(&self) -> String {
        let mut out = String::with_capacity(1024);
        out.push_str("# HELP spendguard_usage_poller_cycles_total Usage poller cycles broken out by outcome.\n");
        out.push_str("# TYPE spendguard_usage_poller_cycles_total counter\n");
        out.push_str(&format!(
            "spendguard_usage_poller_cycles_total{{outcome=\"ok\"}} {}\n",
            self.inner.cycles_ok.load(Ordering::Relaxed),
        ));
        out.push_str(&format!(
            "spendguard_usage_poller_cycles_total{{outcome=\"err\"}} {}\n",
            self.inner.cycles_err.load(Ordering::Relaxed),
        ));
        out.push_str("# HELP spendguard_usage_poller_records_total Records counted by lifecycle stage.\n");
        out.push_str("# TYPE spendguard_usage_poller_records_total counter\n");
        out.push_str(&format!(
            "spendguard_usage_poller_records_total{{stage=\"fetched\"}} {}\n",
            self.inner.records_fetched.load(Ordering::Relaxed),
        ));
        out.push_str(&format!(
            "spendguard_usage_poller_records_total{{stage=\"inserted\"}} {}\n",
            self.inner.records_inserted.load(Ordering::Relaxed),
        ));
        out.push_str(&format!(
            "spendguard_usage_poller_records_total{{stage=\"deduped\"}} {}\n",
            self.inner.records_deduped.load(Ordering::Relaxed),
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_counters_render() {
        let m = UsagePollerMetrics::new();
        m.inc_cycle(CycleOutcome::Ok);
        m.inc_cycle(CycleOutcome::Ok);
        m.inc_cycle(CycleOutcome::Err);
        let txt = m.render();
        assert!(txt.contains("cycles_total{outcome=\"ok\"} 2"));
        assert!(txt.contains("cycles_total{outcome=\"err\"} 1"));
    }

    #[test]
    fn record_counters_render() {
        let m = UsagePollerMetrics::new();
        m.add_records(10, 7, 3);
        m.add_records(5, 5, 0);
        let txt = m.render();
        assert!(txt.contains("records_total{stage=\"fetched\"} 15"));
        assert!(txt.contains("records_total{stage=\"inserted\"} 12"));
        assert!(txt.contains("records_total{stage=\"deduped\"} 3"));
    }

    #[test]
    fn render_help_and_type() {
        let m = UsagePollerMetrics::new();
        let txt = m.render();
        assert!(txt.contains("# HELP spendguard_usage_poller_cycles_total"));
        assert!(txt.contains("# TYPE spendguard_usage_poller_records_total counter"));
    }

    #[test]
    fn shared_state_thread_safe() {
        let m = UsagePollerMetrics::new();
        let m2 = m.clone();
        m.inc_cycle(CycleOutcome::Ok);
        m2.inc_cycle(CycleOutcome::Ok);
        let txt = m.render();
        assert!(txt.contains("cycles_total{outcome=\"ok\"} 2"));
    }
}
