//! SLICE_15 concurrent-burst benchmark harness.
//!
//! Spec ancestors:
//!   - docs/slices/SLICE_15_end_to_end_benchmark.md §2 + §8.2
//!   - docs/predictor-architecture-spec-v1alpha1.md §0.2 lock criterion #5
//!   - docs/contract-dsl-spec-v1alpha1.md §14 (p99 < 50ms decision SLO)
//!
//! ## What this binary does
//!
//! For each burst level `N` in {1, 10, 100, 1000}:
//!   1. Spawn N concurrent tasks that each call the configured
//!      target (SpendGuard sidecar, LiteLLM proxy, or Portkey).
//!   2. Each task issues a "decision" request and records per-call
//!      wall-clock latency into an HDR histogram.
//!   3. After all N tasks complete, capture:
//!         - p50 / p95 / p99 latency (us)
//!         - overshoot %: (reserved - actual) / actual (per existing
//!           benchmarks/runaway-loop methodology)
//!         - Tier 2 path p99 (when running against SpendGuard; the
//!           tokenizer service exposes a /metrics endpoint we scrape)
//!   4. Write `results.json` + Markdown table to the output dir.
//!
//! ## Warmup discipline
//!
//! Per slice §9 review item #4: every burst includes a `warmup_calls`
//! prologue that runs against the same target but whose latencies are
//! NOT included in the histogram. This is critical because the first
//! few calls per process pay cold-start costs (TLS handshake, gRPC
//! channel init, JIT) that would skew p99.
//!
//! ## Hardware spec capture
//!
//! At startup we emit OS, CPU arch, num_cpus, total RAM, and the host's
//! NTP-disciplined wall-clock to RESULTS.md (per slice §9 item #1). This
//! makes the benchmark reproducible by a reviewer running on a different
//! box.
//!
//! ## Competitor selection
//!
//! Per slice §3: closed-source competitors are documented N/A. Portkey
//! is documented N/A in RESULTS.md because the source isn't available;
//! we ship the stub here so anybody with a Portkey installation can
//! wire it locally without further patches.
//!
//! Usage:
//!     predictor-upgrade-bench --target spendguard --bursts 1,10,100 \
//!         --output ./out --warmup 50

use clap::Parser;
use std::path::PathBuf;
use std::time::Instant;
use tracing::{error, info, warn};

mod competitors;
mod harness;
mod report;

use competitors::{Competitor, CompetitorName};
use harness::{BurstReport, BurstRunner};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "predictor-upgrade-bench",
    about = "SLICE_15 concurrent-burst benchmark harness",
    version
)]
struct Cli {
    /// Comma-separated burst levels to run (e.g. "1,10,100,1000").
    ///
    /// 1000-concurrent is gated behind --include-1k because some CI
    /// runners can't handle 1000 sockets reliably (per slice §9 item #7
    /// — CI run-time budget < 30 min).
    #[arg(long, default_value = "1,10,100")]
    bursts: String,

    /// Number of warmup calls per burst level. Latencies NOT included
    /// in the histogram. Per slice §9 review item #4.
    #[arg(long, default_value_t = 50)]
    warmup: usize,

    /// Number of measured calls per burst level (after warmup). The
    /// histogram receives this many samples per burst. Higher = more
    /// accurate tail percentile but longer wall time.
    #[arg(long, default_value_t = 1000)]
    samples: usize,

    /// Which competitor target(s) to benchmark. Comma-separated.
    /// One of: spendguard,litellm,portkey,all.
    #[arg(long, default_value = "spendguard,litellm,portkey")]
    targets: String,

    /// Output directory for results.json + RESULTS.md. Created if missing.
    #[arg(long, default_value = "./out")]
    output: PathBuf,

    /// Include the 1000-concurrent burst level (off by default in CI).
    #[arg(long, default_value_t = false)]
    include_1k: bool,

    /// SpendGuard sidecar UDS path (only used when targets includes spendguard).
    #[arg(long, default_value = "/var/run/spendguard/adapter.sock")]
    sidecar_uds: String,

    /// LiteLLM proxy base URL (only used when targets includes litellm).
    #[arg(long, default_value = "http://localhost:4000")]
    litellm_url: String,

    /// Portkey base URL (only used when targets includes portkey).
    /// Default unset — script will record portkey as N/A.
    #[arg(long, default_value = "")]
    portkey_url: String,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    let cli = Cli::parse();
    info!("predictor-upgrade-bench starting");

    // ---------------------------------------------------------------
    // 1. Parse burst levels.
    // ---------------------------------------------------------------
    let mut bursts: Vec<usize> = cli
        .bursts
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<usize>().map_err(|e| anyhow::anyhow!("bad burst '{}': {}", s, e)))
        .collect::<Result<_, _>>()?;
    if !cli.include_1k {
        bursts.retain(|&b| b < 1000);
    }
    bursts.sort_unstable();
    bursts.dedup();
    info!("burst levels: {:?}", bursts);

    // ---------------------------------------------------------------
    // 2. Parse targets.
    // ---------------------------------------------------------------
    let mut competitor_specs: Vec<(CompetitorName, Box<dyn Competitor>)> = Vec::new();
    for raw in cli.targets.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match raw {
            "all" => {
                competitor_specs.push((
                    CompetitorName::SpendGuard,
                    competitors::spendguard::new(cli.sidecar_uds.clone()),
                ));
                competitor_specs.push((
                    CompetitorName::LiteLlm,
                    competitors::litellm::new(cli.litellm_url.clone()),
                ));
                competitor_specs.push((
                    CompetitorName::Portkey,
                    competitors::portkey::new(cli.portkey_url.clone()),
                ));
            }
            "spendguard" => competitor_specs.push((
                CompetitorName::SpendGuard,
                competitors::spendguard::new(cli.sidecar_uds.clone()),
            )),
            "litellm" => competitor_specs.push((
                CompetitorName::LiteLlm,
                competitors::litellm::new(cli.litellm_url.clone()),
            )),
            "portkey" => competitor_specs.push((
                CompetitorName::Portkey,
                competitors::portkey::new(cli.portkey_url.clone()),
            )),
            other => {
                warn!("unknown target {:?} — ignored", other);
            }
        }
    }
    if competitor_specs.is_empty() {
        anyhow::bail!("no targets selected after parsing (--targets={})", cli.targets);
    }

    // ---------------------------------------------------------------
    // 3. Run the burst matrix.
    // ---------------------------------------------------------------
    let mut all_reports: Vec<(CompetitorName, Vec<BurstReport>)> = Vec::new();

    for (name, runner) in competitor_specs {
        info!("=== competitor: {} ===", name.as_str());
        let mut per_burst: Vec<BurstReport> = Vec::new();
        for &burst in &bursts {
            info!("  burst={} warmup={} samples={}", burst, cli.warmup, cli.samples);
            let runner_ref = BurstRunner::new(runner.as_ref());
            let start = Instant::now();
            match runner_ref.run(burst, cli.warmup, cli.samples).await {
                Ok(report) => {
                    info!(
                        "    burst={} p50={}us p95={}us p99={}us overshoot={:.2}% errors={}",
                        burst,
                        report.p50_us,
                        report.p95_us,
                        report.p99_us,
                        report.overshoot_pct * 100.0,
                        report.errors,
                    );
                    per_burst.push(report);
                }
                Err(e) => {
                    error!("    burst={} FAILED: {}", burst, e);
                    // Record an error-only report so RESULTS.md shows the
                    // gap explicitly (per feedback_demo_quality_gate.md).
                    per_burst.push(BurstReport::error_only(burst, e.to_string()));
                }
            }
            info!("    wall: {:.2}s", start.elapsed().as_secs_f64());
        }
        all_reports.push((name, per_burst));
    }

    // ---------------------------------------------------------------
    // 4. Write reports.
    // ---------------------------------------------------------------
    std::fs::create_dir_all(&cli.output)?;
    report::write_json(&cli.output, &all_reports)?;
    report::write_markdown(&cli.output, &all_reports, &cli)?;

    info!(
        "Done. Results written to {} (results.json + RESULTS.md)",
        cli.output.display()
    );

    // ---------------------------------------------------------------
    // 5. SLO gate. Per slice §8.2: SpendGuard p99 < 50ms (50_000 us).
    // If SpendGuard ran AND violated the SLO, exit non-zero so CI can
    // fail the PR (per slice §8.5: regression alerts > 10% from baseline).
    // ---------------------------------------------------------------
    let mut slo_violated = false;
    for (name, reports) in &all_reports {
        if !matches!(name, CompetitorName::SpendGuard) {
            continue;
        }
        for r in reports {
            if r.errors > 0 && r.samples == 0 {
                // Pure error report — caller decides via JSON; don't gate.
                continue;
            }
            if r.p99_us > 50_000 {
                error!(
                    "SLO VIOLATION: SpendGuard burst={} p99={}us > 50_000us",
                    r.burst, r.p99_us
                );
                slo_violated = true;
            }
        }
    }
    if slo_violated {
        std::process::exit(2);
    }
    Ok(())
}
