//! Phase 5 GA hardening S8: Prometheus metrics for canonical ingest.
//!
//! No external `prometheus` crate dependency — we render the canonical
//! Prometheus text format ourselves from a small set of atomic counters.
//! This keeps the dependency graph lean and the rendering deterministic
//! for tests.
//!
//! Surfaced counters:
//!
//!   * `spendguard_ingest_events_accepted_total{route}`
//!   * `spendguard_ingest_events_rejected_invalid_signature_total{route}`
//!   * `spendguard_ingest_events_quarantined_total{reason}`
//!   * `spendguard_ingest_events_pre_s6_admitted_total{route}`
//!   * `spendguard_ingest_events_disabled_admitted_total{route}`
//!
//! Increment paths are wired in `handlers/append_events.rs::process_one`.
//! The metrics HTTP server is started by `serve_metrics` from
//! `services/canonical_ingest/src/main.rs`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Default)]
pub struct IngestMetricsInner {
    pub accepted_enforcement: AtomicU64,
    pub accepted_observability: AtomicU64,
    pub rejected_invalid_sig_enforcement: AtomicU64,
    pub rejected_invalid_sig_observability: AtomicU64,
    pub quarantined_unknown_key: AtomicU64,
    pub quarantined_invalid_signature: AtomicU64,
    pub quarantined_pre_s6: AtomicU64,
    pub quarantined_disabled: AtomicU64,
    pub quarantined_oversized: AtomicU64,
    pub pre_s6_admitted: AtomicU64,
    pub disabled_admitted: AtomicU64,
    pub schema_failure: AtomicU64,
    /// S7: validity-window quarantine reasons.
    pub quarantined_key_expired: AtomicU64,
    pub quarantined_key_not_yet_valid: AtomicU64,
    pub quarantined_key_revoked: AtomicU64,
}

#[derive(Clone, Default)]
pub struct IngestMetrics {
    pub inner: Arc<IngestMetricsInner>,
}

impl IngestMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_accepted(&self, route: Route) {
        match route {
            Route::Enforcement => &self.inner.accepted_enforcement,
            Route::Observability => &self.inner.accepted_observability,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_rejected_invalid_sig(&self, route: Route) {
        match route {
            Route::Enforcement => &self.inner.rejected_invalid_sig_enforcement,
            Route::Observability => &self.inner.rejected_invalid_sig_observability,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_quarantined(&self, reason: QuarantineReason) {
        let counter = match reason {
            QuarantineReason::UnknownKey => &self.inner.quarantined_unknown_key,
            QuarantineReason::InvalidSignature => &self.inner.quarantined_invalid_signature,
            QuarantineReason::PreS6 => &self.inner.quarantined_pre_s6,
            QuarantineReason::Disabled => &self.inner.quarantined_disabled,
            QuarantineReason::Oversized => &self.inner.quarantined_oversized,
            QuarantineReason::SchemaFailure => &self.inner.schema_failure,
            QuarantineReason::KeyExpired => &self.inner.quarantined_key_expired,
            QuarantineReason::KeyNotYetValid => &self.inner.quarantined_key_not_yet_valid,
            QuarantineReason::KeyRevoked => &self.inner.quarantined_key_revoked,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_pre_s6_admitted(&self) {
        self.inner.pre_s6_admitted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_disabled_admitted(&self) {
        self.inner.disabled_admitted.fetch_add(1, Ordering::Relaxed);
    }

    /// Render the Prometheus text format.
    pub fn render(&self) -> String {
        let i = &self.inner;
        let mut out = String::with_capacity(2048);
        out.push_str("# HELP spendguard_ingest_events_accepted_total Events accepted into the canonical log.\n");
        out.push_str("# TYPE spendguard_ingest_events_accepted_total counter\n");
        out.push_str(&format!(
            "spendguard_ingest_events_accepted_total{{route=\"enforcement\"}} {}\n",
            i.accepted_enforcement.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "spendguard_ingest_events_accepted_total{{route=\"observability\"}} {}\n",
            i.accepted_observability.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP spendguard_ingest_events_rejected_invalid_signature_total Events rejected with invalid signature in strict mode.\n");
        out.push_str("# TYPE spendguard_ingest_events_rejected_invalid_signature_total counter\n");
        out.push_str(&format!(
            "spendguard_ingest_events_rejected_invalid_signature_total{{route=\"enforcement\"}} {}\n",
            i.rejected_invalid_sig_enforcement.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "spendguard_ingest_events_rejected_invalid_signature_total{{route=\"observability\"}} {}\n",
            i.rejected_invalid_sig_observability.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP spendguard_ingest_events_quarantined_total Events written to audit_signature_quarantine.\n");
        out.push_str("# TYPE spendguard_ingest_events_quarantined_total counter\n");
        for (label, counter) in [
            ("unknown_key", &i.quarantined_unknown_key),
            ("invalid_signature", &i.quarantined_invalid_signature),
            ("pre_s6", &i.quarantined_pre_s6),
            ("disabled", &i.quarantined_disabled),
            ("oversized_canonical", &i.quarantined_oversized),
            ("schema_failure", &i.schema_failure),
            // S7
            ("key_expired", &i.quarantined_key_expired),
            ("key_not_yet_valid", &i.quarantined_key_not_yet_valid),
            ("key_revoked", &i.quarantined_key_revoked),
        ] {
            out.push_str(&format!(
                "spendguard_ingest_events_quarantined_total{{reason=\"{}\"}} {}\n",
                label,
                counter.load(Ordering::Relaxed)
            ));
        }

        out.push_str("# HELP spendguard_ingest_events_pre_s6_admitted_total pre-S6 rows admitted in non-strict mode.\n");
        out.push_str("# TYPE spendguard_ingest_events_pre_s6_admitted_total counter\n");
        out.push_str(&format!(
            "spendguard_ingest_events_pre_s6_admitted_total {}\n",
            i.pre_s6_admitted.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP spendguard_ingest_events_disabled_admitted_total disabled-signer rows admitted in non-strict mode (demo profile).\n");
        out.push_str("# TYPE spendguard_ingest_events_disabled_admitted_total counter\n");
        out.push_str(&format!(
            "spendguard_ingest_events_disabled_admitted_total {}\n",
            i.disabled_admitted.load(Ordering::Relaxed)
        ));

        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Enforcement,
    Observability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuarantineReason {
    UnknownKey,
    InvalidSignature,
    PreS6,
    Disabled,
    Oversized,
    SchemaFailure,
    /// S7
    KeyExpired,
    KeyNotYetValid,
    KeyRevoked,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_default_to_zero_in_render_output() {
        let m = IngestMetrics::new();
        let txt = m.render();
        assert!(txt.contains("spendguard_ingest_events_accepted_total{route=\"enforcement\"} 0"));
        assert!(txt.contains("spendguard_ingest_events_quarantined_total{reason=\"unknown_key\"} 0"));
    }

    #[test]
    fn increments_show_in_render_output() {
        let m = IngestMetrics::new();
        m.inc_accepted(Route::Enforcement);
        m.inc_accepted(Route::Enforcement);
        m.inc_accepted(Route::Observability);
        m.inc_quarantined(QuarantineReason::UnknownKey);
        m.inc_rejected_invalid_sig(Route::Enforcement);
        m.inc_pre_s6_admitted();
        let txt = m.render();
        assert!(txt.contains("spendguard_ingest_events_accepted_total{route=\"enforcement\"} 2"));
        assert!(txt.contains("spendguard_ingest_events_accepted_total{route=\"observability\"} 1"));
        assert!(txt.contains("spendguard_ingest_events_quarantined_total{reason=\"unknown_key\"} 1"));
        assert!(txt.contains("spendguard_ingest_events_rejected_invalid_signature_total{route=\"enforcement\"} 1"));
        assert!(txt.contains("spendguard_ingest_events_pre_s6_admitted_total 1"));
    }

    #[test]
    fn render_includes_help_and_type_lines() {
        let m = IngestMetrics::new();
        let txt = m.render();
        for prefix in [
            "# HELP spendguard_ingest_events_accepted_total",
            "# TYPE spendguard_ingest_events_accepted_total counter",
            "# HELP spendguard_ingest_events_quarantined_total",
            "# TYPE spendguard_ingest_events_quarantined_total counter",
        ] {
            assert!(txt.contains(prefix), "missing line: {prefix}");
        }
    }

    #[test]
    fn shared_state_is_thread_safe() {
        // Cheap clone (Arc) so workers can hold their own handle.
        let m = IngestMetrics::new();
        let m2 = m.clone();
        m.inc_accepted(Route::Enforcement);
        m2.inc_accepted(Route::Enforcement);
        let txt = m.render();
        assert!(txt.contains("route=\"enforcement\"} 2"));
    }
}
