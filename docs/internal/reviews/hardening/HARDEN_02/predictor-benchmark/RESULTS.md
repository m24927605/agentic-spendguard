# SLICE_15 Predictor Upgrade — Benchmark RESULTS

## Reproducibility metadata

- **OS / Arch:** macos / aarch64
- **Logical CPUs:** 8
- **Timestamp (UTC):** 2026-05-30T23:10:52Z
- **Bench binary:** spendguard-predictor-upgrade-benchmarks v0.1.0-alpha
- **Bursts run:** 1,10,100
- **Requested warmup calls per burst:** 50
- **Effective warmup discipline:** at least two full burst waves before measurement.
- **Measured samples per burst:** 1000

## Headline results

| Competitor | Burst | Samples | Errors | p50 (us) | p95 (us) | p99 (us) | p99.9 (us) | Overshoot % |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| spendguard | 1 | 1000 | 0 | 391 | 447 | 504 | 648 | +42.86% |
| spendguard | 10 | 1000 | 0 | 1679 | 3083 | 3645 | 3933 | +31.82% |
| spendguard | 100 | 1000 | 0 | 13247 | 14719 | 15407 | 15599 | +18.27% |

## SLO interpretation

- **Contract DSL §14 SLO:** SpendGuard decision p99 < 50_000 us (50ms).
- **Latency accounting:** adapters may report pre-call decision latency separately from post-call accounting; the SpendGuard benchmark measures reserve/deny latency for the decision SLO, while reserve+commit receipts are covered by `benchmarks/runaway-loop/`.
- **Slice §8.2:** Tier 2 tokenizer p99 < 1_000 us (1ms) — verified separately in `benchmarks/tokenizer/`.
- **Slice §8.2:** SpendGuard overshoot % < LiteLLM at every burst level.
- **Slice §8.3:** Calibration accuracy — see `calibration_synthetic.py` output.
- **Slice §8.5:** CI regression alert if p99 increases > 10% from baseline.

## Competitor notes

- **SpendGuard:** Run against the benchmark reservation shim on `localhost:8090`; full sidecar/demo validation is covered by `tests/e2e/predictor_upgrade.sh` and HARDEN_02 demo gates.
- **LiteLLM proxy:** `ghcr.io/berriai/litellm:main-stable` (image SHA captured via `docker image inspect` at run time — see `results.json`).
- **Portkey:** **Documented N/A** — closed-source proxy. Pass `--portkey-url + PORTKEY_API_KEY` to wire against a live gateway.

## Reproducing

```bash
# 1. Bring up the benchmark SpendGuard reservation shim:
SHIM_DISABLE_LEDGER_LOG=1 \
docker compose -f benchmarks/runaway-loop/compose.yml \
    up -d --build spendguard-shim

# 2. Build + run the benchmark:
cd benchmarks/predictor-upgrade
cargo build --release
SPENDGUARD_BENCH_SHIM_URL=http://localhost:8090 \
./target/release/predictor-upgrade-bench --bursts 1,10,100 \
    --warmup 50 --samples 1000 \
    --targets spendguard,litellm,portkey \
    --output ./out
```

Results land in `./out/RESULTS.md` (this file) + `./out/results.json`.
