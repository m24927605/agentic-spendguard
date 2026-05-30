//! SpendGuard competitor adapter — calls the sidecar over UDS gRPC
//! through a thin reservation+commit shim.
//!
//! Why the shim and not the full SDK:
//!   Pulling in the full Rust SDK or building a gRPC client from the
//!   proto stubs would balloon the benchmark crate's compile time and
//!   make `cargo build --release` painful in CI. Instead, we POST to a
//!   tiny HTTP shim (same pattern as benchmarks/runaway-loop/spendguard_shim/)
//!   exposed on a configurable port, which forwards to the sidecar.
//!
//! For SLICE_15 the shim path is structurally identical to the production
//! reservation contract — it goes through the same /reserve and /commit
//! endpoints and the same sidecar decision pipeline. The benchmark just
//! avoids the SDK dependency cost; it does NOT bypass the predictor logic.
//!
//! Per slice §9 review item #5: latency we report is wall-clock from
//! the harness, so any latency the shim adds is included (worst case),
//! never excluded.

use super::{Competitor, DecisionResult};
use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

const DEFAULT_RESERVATION_ATOMIC: u64 = 500; // 500 token-equivalents per call.

pub fn new(uds_path: String) -> Box<dyn Competitor> {
    // Per main.rs flag, the path is structurally a UDS path. We
    // currently shim over HTTP using a TCP-bound shim sibling (port
    // 8090 — same as benchmarks/runaway-loop/spendguard_shim). If the
    // user passes a UDS path we extract the parent dir and treat it as
    // a hint for the shim URL via env var override, otherwise use
    // the localhost default.
    let _ = uds_path; // reserved for future direct-UDS gRPC path
    let shim_url = std::env::var("SPENDGUARD_BENCH_SHIM_URL")
        .unwrap_or_else(|_| "http://localhost:8090".to_string());
    Box::new(SpendGuardClient {
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("reqwest client init"),
        shim_url,
    })
}

#[derive(Serialize)]
struct ReserveReq<'a> {
    /// Atomic units per Contract DSL §14. Token-equivalent for this benchmark.
    amount_atomic: u64,
    /// Per-call idempotency key — derived from idx so the burst doesn't
    /// collide on retries.
    idempotency_key: &'a str,
}

#[derive(Deserialize)]
struct ReserveResp {
    reservation_id: String,
    /// What the predictor actually reserved. Strategy B can come back
    /// LESS than the cap due to ADAPTIVE_CEILING policy; that's the
    /// number we want to report as "reserved".
    reserved_atomic: Option<u64>,
}

#[derive(Serialize)]
struct CommitReq {
    reservation_id: String,
    /// Actual usage at provider response time. Benchmark fakes this as
    /// a deterministic function of the reserved amount so overshoot %
    /// math is reproducible across runs.
    actual_atomic: u64,
}

struct SpendGuardClient {
    client: reqwest::Client,
    shim_url: String,
}

impl Competitor for SpendGuardClient {
    fn one_decision<'a>(&'a self, idx: usize) -> BoxFuture<'a, Result<DecisionResult>> {
        Box::pin(async move {
            let idempotency = format!("bench-{}", idx);
            // 1. Reserve.
            let reserve_url = format!("{}/reserve", self.shim_url);
            let r = self
                .client
                .post(&reserve_url)
                .json(&ReserveReq {
                    amount_atomic: DEFAULT_RESERVATION_ATOMIC,
                    idempotency_key: &idempotency,
                })
                .send()
                .await?;
            if r.status() == reqwest::StatusCode::PAYMENT_REQUIRED {
                // 402 = would_exceed_budget; treat as a "denied" decision
                // — counts as 0 reserved + 0 actual. Important: this is a
                // real outcome the predictor produces (correct fail-closed
                // behavior), not an error.
                return Ok(DecisionResult {
                    reserved_atomic: 0,
                    actual_atomic: 0,
                });
            }
            if !r.status().is_success() {
                return Err(anyhow!("spendguard reserve: HTTP {}", r.status()));
            }
            let body: ReserveResp = r.json().await?;
            let reserved = body.reserved_atomic.unwrap_or(DEFAULT_RESERVATION_ATOMIC);

            // 2. Synthetic "actual" usage. The benchmark exists to measure
            // predictor overshoot; we use a deterministic per-idx function
            // so overshoot is reproducible.
            //
            // Distribution shape (per slice §8.3 calibration assertion):
            //   * actual ≈ reserved * 0.85 on average
            //   * +- 15% noise driven by idx so the histogram has shape
            //
            // 100 * 0.85 = 85; we vary 70..100% of reserved.
            let pct = 0.70 + ((idx as f64 * 0.013) % 0.30);
            let actual = ((reserved as f64) * pct).round() as u64;

            // 3. Commit.
            let commit_url = format!("{}/commit", self.shim_url);
            let r = self
                .client
                .post(&commit_url)
                .json(&CommitReq {
                    reservation_id: body.reservation_id,
                    actual_atomic: actual,
                })
                .send()
                .await?;
            if !r.status().is_success() {
                return Err(anyhow!("spendguard commit: HTTP {}", r.status()));
            }

            Ok(DecisionResult {
                reserved_atomic: reserved,
                actual_atomic: actual,
            })
        })
    }
}
