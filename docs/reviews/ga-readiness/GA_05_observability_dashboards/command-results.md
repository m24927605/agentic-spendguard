# GA_05 Command Results

Date: 2026-05-31

| Gate | Result | Evidence |
|---|---|---|
| `scripts/observability/validate-dashboard-metrics.sh` | PASS | 19 dashboard metrics across 19 PromQL expressions validated; endpoint ports are checked against the compose/config port map. |
| `python3 -m json.tool deploy/observability/grafana-dashboard.json` | PASS | Dashboard JSON parsed; normalized output had 460 lines after R2 leader-count panel. |
| `cargo build --manifest-path services/output_predictor/Cargo.toml` | PASS | Dev build completed. |
| `cargo test --manifest-path services/output_predictor/Cargo.toml` | PASS | 150 lib tests, 7 bin tests, integration/doc tests passed. |
| `cargo build --manifest-path services/run_cost_projector/Cargo.toml` | PASS | Dev build completed. |
| `cargo test --manifest-path services/run_cost_projector/Cargo.toml` | PASS | 55 lib tests, 5 bin tests, 3 integration tests passed. |
| `cargo build --manifest-path services/outbox_forwarder/Cargo.toml` | PASS | Dev build completed. |
| `cargo test --manifest-path services/outbox_forwarder/Cargo.toml` | PASS | 9 lib tests and doc tests passed, including the leader gauge. |
| `cargo build --manifest-path services/canonical_ingest/Cargo.toml` | PASS | Dev build completed. |
| `cargo test --manifest-path services/canonical_ingest/Cargo.toml` | PASS | 52 lib tests, 12 verify-chain tests, 1 compile-fence test, 8 Postgres quarantine/replay tests, and doc tests passed. |
| `helm template charts/spendguard --set chart.profile=demo` | PASS | Rendered 1441 manifest lines. |
| `helm template charts/spendguard -f scripts/helm-validate-test-values.yaml` | PASS | Rendered 1534 manifest lines under production profile. |
| `make demo-up DEMO_MODE=default` | PASS | Clean-state rerun after R2 fixes passed Step 8 and outbox closure. An earlier first attempt against an existing dirty demo volume failed with doubled ledger counts, then `make demo-down` reset volumes before clean passes. |
| Live scrape: canonical ingest | PASS | `spendguard_ingest_events_deduped_total`, reject, and quarantine metrics visible at `:9091/metrics`. |
| Live scrape: output predictor | PASS | `spendguard_output_predictor_predict_latency_seconds_bucket`, `spendguard_output_predictor_cache_lookup_total`, and `spendguard_output_predictor_cache_hit_total` visible at `:9100/metrics` after starting `output-predictor` in compose. |
| Live scrape: outbox forwarder | PASS | `spendguard_outbox_pending_oldest_age_seconds`, `spendguard_outbox_forwarder_is_leader`, and forwarder row counters visible at `:9096/metrics`. |
| Live scrape: run cost projector | PASS | project outcome counters, project latency buckets, and terminate counters visible at `:9102/metrics`. |

Notes:

- The placeholder grep only matches the validator's explicit rejection list and the inventory explanation, not emitted service source.
- AIT CLI compatibility: `ait run --adapter codex --review-mode adversarial ...` failed locally with `unrecognized arguments: --review-mode`; codex subagent adversarial review is used as fallback and recorded per round.
- R2 fixed no-leader audit lag masking by refreshing pending oldest-row age on every outbox-forwarder pod and adding an `Outbox Forwarder Leaders` panel.
- R4 fixed metrics-inventory Markdown parsing by replacing raw pipe separators in label enum text and making the validator reject rows that do not have exactly seven cells.
