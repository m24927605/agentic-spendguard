# SLICE_15 Predictor Upgrade — Benchmark RESULTS (template)

> **Status:** template. Real numbers populated by running the harness.
> Per slice doc §8.5: CI runs this on every PR and updates the table.
> Local repro: see [Reproducing](#reproducing) below.

This file is the **template** structure committed to the repo. The
actual results table is overwritten by the bench binary's report writer
(`benchmarks/predictor-upgrade/src/report.rs::write_markdown`) on every
run. Treat this file as documentation of the SHAPE of the results, not
the results themselves. After a run, copy the latest from
`./out/RESULTS.md` over this file (and commit if it's the canonical
release benchmark).

## Reproducibility metadata (template)

- **OS / Arch:** _populated at run time_ (e.g. `macos / aarch64`, `linux / x86_64`)
- **Logical CPUs:** _populated at run time_
- **Timestamp (UTC):** _populated at run time_
- **SpendGuard version:** SLICE_15 merge commit hash (`git describe --dirty`)
- **LiteLLM version:** `ghcr.io/berriai/litellm:main-stable` (SHA captured via `docker image inspect`)
- **Portkey version:** N/A — closed source

## Headline results (template structure)

| Competitor | Burst | Samples | Errors | p50 (us) | p95 (us) | p99 (us) | p99.9 (us) | Overshoot % |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| spendguard | 1   | 1000 | 0 | _ | _ | _ | _ | _ |
| spendguard | 10  | 1000 | 0 | _ | _ | _ | _ | _ |
| spendguard | 100 | 1000 | 0 | _ | _ | _ | _ | _ |
| litellm    | 1   | 1000 | 0 | _ | _ | _ | _ | _ |
| litellm    | 10  | 1000 | 0 | _ | _ | _ | _ | _ |
| litellm    | 100 | 1000 | 0 | _ | _ | _ | _ | _ |
| portkey    | 1   | 0    | 1000 | — | — | — | — | — |
| portkey    | 10  | 0    | 1000 | — | — | — | — | — |
| portkey    | 100 | 0    | 1000 | — | — | — | — | — |

Portkey rows show `errors == burst` because the harness records
"documented N/A — closed source" as the per-call error message. RESULTS.md
treats this as a structured non-result, not a benchmark failure.

## SLO interpretation

- **Contract DSL §14 SLO:** SpendGuard decision p99 < 50_000 us (50 ms).
- **Slice §8.2:** Tier 2 tokenizer p99 < 1_000 us (1 ms) — verified separately in `benchmarks/tokenizer/`.
- **Slice §8.2:** SpendGuard overshoot % < LiteLLM at every burst level.
- **Slice §8.3:** Calibration accuracy — see `calibration.json` / `calibration.md` from `calibration_synthetic.py`.
- **Slice §8.5:** CI regression alert if p99 increases > 10% from baseline (`.github/workflows/predictor-benchmark.yml`).

## Competitor notes

- **SpendGuard:** Full predictor-upgrade stack from `deploy/demo/compose.yaml` (tokenizer + output_predictor + run_cost_projector + stats_aggregator + sidecar). Reservation path exercised end-to-end via the local shim.
- **LiteLLM proxy:** `ghcr.io/berriai/litellm:main-stable`. Post-call enforcement — captures the "one call past budget" pattern visible in `benchmarks/runaway-loop/RESULTS.md`.
- **Portkey:** Documented N/A. Closed-source proxy. The benchmark adapter in `src/competitors/portkey.rs` will activate against any reachable Portkey gateway given `--portkey-url + PORTKEY_API_KEY`.

## Reproducing

```bash
# 1. Bring up the full predictor-upgrade demo stack:
bash tests/e2e/predictor_upgrade.sh

# 2. (Optional) Bring up LiteLLM proxy via the existing example:
cd examples/litellm-proxy-composite && docker compose up -d
cd ../..

# 3. Build + run the benchmark:
cd benchmarks/predictor-upgrade
cargo build --release
./target/release/predictor-upgrade-bench \
    --bursts 1,10,100 \
    --warmup 50 \
    --samples 1000 \
    --targets spendguard,litellm,portkey \
    --output ./out

# 4. (Optional) Run calibration synthetic:
python3 calibration_synthetic.py --targets spendguard --output ./out

# Results land in ./out/RESULTS.md (overwrites this template) + ./out/results.json
# and ./out/calibration.json + ./out/calibration.md.
```

## Notes on the calibration path

`calibration_synthetic.py` runs 1000 controlled prompts spanning 7
prompt classes. Per slice §8.3 the spec asserts SpendGuard P95
|predicted - actual| / actual ≤ 0.05 (5%). The script exits non-zero
if any class violates this — used as the CI gate alongside the burst
benchmark.

The mock-LLM output distribution is deterministic in `(prompt_class,
sample_idx)` so the benchmark is reproducible bit-for-bit: identical
hardware + identical SpendGuard version → identical numbers. Drift
across versions is what the CI workflow flags via
[`.github/workflows/predictor-benchmark.yml`](../../.github/workflows/predictor-benchmark.yml).
