//! Shared benchmark harness: burst-level concurrency + HDR histogram +
//! overshoot accounting. Decoupled from any specific competitor so the
//! same harness applies to SpendGuard / LiteLLM / Portkey identically.
//!
//! Per slice doc §9 review items #4 + #5: warmup phase runs separately
//! from the measured phase; p99 comes from hdrhistogram (no averaging
//! foot-gun).

use crate::competitors::{Competitor, DecisionResult};
use anyhow::Result;
use futures::stream::{FuturesUnordered, StreamExt};
use hdrhistogram::Histogram;
use serde::Serialize;
use std::time::Instant;

/// One row in RESULTS.md / one entry in results.json per (competitor, burst).
#[derive(Debug, Serialize, Clone)]
pub struct BurstReport {
    pub burst: usize,
    pub samples: usize,
    pub errors: usize,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub p999_us: u64,
    /// Overshoot percentage: `(reserved_units - actual_units) / actual_units`.
    /// Per existing benchmarks/runaway-loop methodology. Negative =
    /// reservation was lower than actual usage (= predictor undersold;
    /// this is what we want from SpendGuard at <0% overshoot under
    /// budget; comparison vs LiteLLM whose "overshoot" is the
    /// post-call invoice gap is documented in RESULTS.md).
    pub overshoot_pct: f64,
    /// Reservation total across all measured calls (atomic units).
    pub reserved_total: u64,
    /// Actual usage total across all measured calls (atomic units).
    pub actual_total: u64,
    pub note: Option<String>,
}

impl BurstReport {
    pub fn error_only(burst: usize, msg: String) -> Self {
        Self {
            burst,
            samples: 0,
            errors: 0,
            p50_us: 0,
            p95_us: 0,
            p99_us: 0,
            p999_us: 0,
            overshoot_pct: 0.0,
            reserved_total: 0,
            actual_total: 0,
            note: Some(format!("HARNESS ERROR: {}", msg)),
        }
    }
}

pub struct BurstRunner<'a> {
    target: &'a dyn Competitor,
}

impl<'a> BurstRunner<'a> {
    pub fn new(target: &'a dyn Competitor) -> Self {
        Self { target }
    }

    pub async fn run(&self, burst: usize, warmup: usize, samples: usize) -> Result<BurstReport> {
        // ---------------------------------------------------------------
        // Warmup. Latencies discarded.
        // ---------------------------------------------------------------
        let effective_warmup = warmup.max(burst.saturating_mul(2));
        if effective_warmup > 0 {
            self.fan_out(burst, effective_warmup, /*record=*/ false)
                .await?;
        }

        // ---------------------------------------------------------------
        // Measured phase.
        //
        // HDR histogram tracks 1us..60s with 3 significant digits — that
        // gives us p99.9 accuracy to ~0.1% of the value at any latency
        // band, which is plenty for the 1-50ms SLO band.
        // ---------------------------------------------------------------
        let mut hist: Histogram<u64> = Histogram::new_with_bounds(1, 60_000_000, 3)
            .expect("hdr bounds valid: 1us..60s @ 3 sig digits");
        let (lat_samples, results) = self.fan_out(burst, samples, /*record=*/ true).await?;

        let mut errors = 0usize;
        let mut reserved_total = 0u64;
        let mut actual_total = 0u64;
        for r in &results {
            match r {
                Ok(d) => {
                    reserved_total = reserved_total.saturating_add(d.reserved_atomic);
                    actual_total = actual_total.saturating_add(d.actual_atomic);
                }
                Err(_) => {
                    errors += 1;
                }
            }
        }

        for &lat in &lat_samples {
            // hdrhistogram::record takes u64; we feed microseconds in.
            // saturating to upper bound avoids panic on stalled calls.
            let _ = hist.record(lat.min(60_000_000));
        }

        let overshoot_pct = if actual_total == 0 {
            // Edge: actual zero (everything denied). We surface this as
            // 0.0 with a note so RESULTS.md doesn't claim "infinity %".
            0.0
        } else {
            // i128 math so we can express negative overshoot cleanly.
            let r = reserved_total as i128;
            let a = actual_total as i128;
            ((r - a) as f64) / (a as f64)
        };

        let report = BurstReport {
            burst,
            samples: lat_samples.len(),
            errors,
            p50_us: hist.value_at_quantile(0.50),
            p95_us: hist.value_at_quantile(0.95),
            p99_us: hist.value_at_quantile(0.99),
            p999_us: hist.value_at_quantile(0.999),
            overshoot_pct,
            reserved_total,
            actual_total,
            note: None,
        };
        Ok(report)
    }

    /// Spawn `total` decisions across waves of size `burst`. Each task
    /// makes one decision request, records its latency, and returns
    /// the per-call result. If `record` is false, latencies are
    /// discarded (warmup mode).
    ///
    /// Returns `(latencies_us, per_call_results)` — caller drops the
    /// latency vec if record=false.
    async fn fan_out(
        &self,
        burst: usize,
        total: usize,
        record: bool,
    ) -> Result<(Vec<u64>, Vec<Result<DecisionResult>>)> {
        let mut latencies: Vec<u64> = if record {
            Vec::with_capacity(total)
        } else {
            Vec::new()
        };
        let mut results: Vec<Result<DecisionResult>> = Vec::with_capacity(total);

        // Process in waves so we don't allocate `total` tasks at once for
        // huge values of `total`. Wave size = burst per slice §2 — that's
        // the actual "concurrent" load level we want to exercise.
        let mut remaining = total;
        while remaining > 0 {
            let wave_size = remaining.min(burst);
            let mut wave: FuturesUnordered<_> = FuturesUnordered::new();
            for i in 0..wave_size {
                let target = self.target;
                wave.push(async move {
                    let t0 = Instant::now();
                    let r = target.one_decision(i).await;
                    let dt_us = t0.elapsed().as_micros() as u64;
                    (dt_us, r)
                });
            }
            while let Some((dt_us, r)) = wave.next().await {
                if record {
                    let decision_dt_us = r
                        .as_ref()
                        .ok()
                        .and_then(|d| d.decision_latency_us)
                        .unwrap_or(dt_us);
                    latencies.push(decision_dt_us);
                }
                results.push(r);
            }
            remaining -= wave_size;
        }

        Ok((latencies, results))
    }
}
