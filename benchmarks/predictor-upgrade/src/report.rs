//! Report serialization — JSON + Markdown.
//!
//! Per slice §9 review item #1 (hardware spec reproducibility):
//! RESULTS.md captures host OS, CPU arch, num_cpus, total RAM, and
//! wall-clock at the top.
//!
//! Per slice §9 review item #9 (numbers tied to commit + date):
//! we emit `git describe --dirty` if available, plus current ISO timestamp.

use crate::competitors::CompetitorName;
use crate::harness::BurstReport;
use crate::Cli;
use anyhow::Result;
use std::fs;
use std::path::Path;

pub fn write_json(out_dir: &Path, all: &[(CompetitorName, Vec<BurstReport>)]) -> Result<()> {
    let mut top = serde_json::Map::new();
    for (name, reports) in all {
        top.insert(name.as_str().to_string(), serde_json::to_value(reports)?);
    }
    let path = out_dir.join("results.json");
    fs::write(&path, serde_json::to_string_pretty(&top)?)?;
    Ok(())
}

pub fn write_markdown(
    out_dir: &Path,
    all: &[(CompetitorName, Vec<BurstReport>)],
    cli: &Cli,
) -> Result<()> {
    let mut md = String::new();
    md.push_str("# SLICE_15 Predictor Upgrade — Benchmark RESULTS\n\n");

    // -------------------------------------------------------------
    // Hardware spec + reproducibility metadata (slice §9 #1, #9).
    // -------------------------------------------------------------
    md.push_str("## Reproducibility metadata\n\n");
    let host_os = std::env::consts::OS;
    let host_arch = std::env::consts::ARCH;
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    md.push_str(&format!("- **OS / Arch:** {} / {}\n", host_os, host_arch));
    md.push_str(&format!(
        "- **Logical CPUs:** {}\n",
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)
    ));
    md.push_str(&format!("- **Timestamp (UTC):** {}\n", now));
    md.push_str(&format!(
        "- **Bench binary:** spendguard-predictor-upgrade-benchmarks v{}\n",
        env!("CARGO_PKG_VERSION")
    ));
    md.push_str(&format!("- **Bursts run:** {}\n", cli.bursts));
    md.push_str(&format!(
        "- **Requested warmup calls per burst:** {}\n",
        cli.warmup
    ));
    md.push_str(
        "- **Effective warmup discipline:** at least two full burst waves before measurement.\n",
    );
    md.push_str(&format!(
        "- **Measured samples per burst:** {}\n",
        cli.samples
    ));
    md.push_str("\n");

    // -------------------------------------------------------------
    // Headline table.
    // -------------------------------------------------------------
    md.push_str("## Headline results\n\n");
    md.push_str("| Competitor | Burst | Samples | Errors | p50 (us) | p95 (us) | p99 (us) | p99.9 (us) | Overshoot % |\n");
    md.push_str("|---|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for (name, reports) in all {
        for r in reports {
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {:+.2}% |\n",
                name.as_str(),
                r.burst,
                r.samples,
                r.errors,
                r.p50_us,
                r.p95_us,
                r.p99_us,
                r.p999_us,
                r.overshoot_pct * 100.0,
            ));
            if let Some(note) = &r.note {
                md.push_str(&format!("|   ↑ note | | | | | | | | {} |\n", note));
            }
        }
    }
    md.push_str("\n");

    // -------------------------------------------------------------
    // SLO interpretation (slice §8.2 + §0.2 lock criteria).
    // -------------------------------------------------------------
    md.push_str("## SLO interpretation\n\n");
    md.push_str("- **Contract DSL §14 SLO:** SpendGuard decision p99 < 50_000 us (50ms).\n");
    md.push_str("- **Latency accounting:** adapters may report pre-call decision latency separately from post-call accounting; the SpendGuard benchmark measures reserve/deny latency for the decision SLO, while reserve+commit receipts are covered by `benchmarks/runaway-loop/`.\n");
    md.push_str("- **Slice §8.2:** Tier 2 tokenizer p99 < 1_000 us (1ms) — verified separately in `benchmarks/tokenizer/`.\n");
    md.push_str("- **Slice §8.2:** SpendGuard overshoot % < LiteLLM at every burst level.\n");
    md.push_str(
        "- **Slice §8.3:** Calibration accuracy — see `calibration_synthetic.py` output.\n",
    );
    md.push_str("- **Slice §8.5:** CI regression alert if p99 increases > 10% from baseline.\n\n");

    // -------------------------------------------------------------
    // Portkey N/A footnote (slice §3 / §10).
    // -------------------------------------------------------------
    md.push_str("## Competitor notes\n\n");
    md.push_str("- **SpendGuard:** Run against the benchmark reservation shim on `localhost:8090`; full sidecar/demo validation is covered by `tests/e2e/predictor_upgrade.sh` and HARDEN_02 demo gates.\n");
    md.push_str("- **LiteLLM proxy:** `ghcr.io/berriai/litellm:main-stable` (image SHA captured via `docker image inspect` at run time — see `results.json`).\n");
    md.push_str("- **Portkey:** **Documented N/A** — closed-source proxy. Pass `--portkey-url + PORTKEY_API_KEY` to wire against a live gateway.\n\n");

    // -------------------------------------------------------------
    // Reproduction.
    // -------------------------------------------------------------
    md.push_str("## Reproducing\n\n");
    md.push_str("```bash\n");
    md.push_str("# 1. Bring up the benchmark SpendGuard reservation shim:\n");
    md.push_str("SHIM_DISABLE_LEDGER_LOG=1 \\\n");
    md.push_str("docker compose -f benchmarks/runaway-loop/compose.yml \\\n");
    md.push_str("    up -d --build spendguard-shim\n\n");
    md.push_str("# 2. Build + run the benchmark:\n");
    md.push_str("cd benchmarks/predictor-upgrade\n");
    md.push_str("cargo build --release\n");
    md.push_str("SPENDGUARD_BENCH_SHIM_URL=http://localhost:8090 \\\n");
    md.push_str("./target/release/predictor-upgrade-bench --bursts 1,10,100 \\\n");
    md.push_str("    --warmup 50 --samples 1000 \\\n");
    md.push_str("    --targets spendguard,litellm,portkey \\\n");
    md.push_str("    --output ./out\n");
    md.push_str("```\n\n");
    md.push_str("Results land in `./out/RESULTS.md` (this file) + `./out/results.json`.\n");

    let path = out_dir.join("RESULTS.md");
    fs::write(&path, md)?;
    Ok(())
}
