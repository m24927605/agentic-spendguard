# GA_07 Command Results

Date: 2026-06-01

| Gate | Result | Evidence |
|---|---|---|
| `scripts/soak/ga-soak.sh --duration 30m --profile local` | PASS | Rerun after R4 timeout + preflight-summary fix on clean source commit `31631db760531022774c49d38d51ee5a4fb89e2a`. `ga_soak_summary.json`: result `pass`, duration `1800`, snapshots `27`, started `2026-06-01T03:11:42Z`, finished `2026-06-01T03:42:04Z`, `git_dirty=false` |
| `cargo test --manifest-path services/output_predictor/Cargo.toml --test plugin_svid_mtls -- --nocapture` | PASS | Run inside the soak harness before sustained snapshots; 4 tests passed |
| `python3 -m pytest contrib/output_predictor_template/conformance_test.py -q -k 'client_svid'` | PASS | Run inside the soak harness before sustained snapshots; 5 tests passed, 65 deselected |
| `python3 tests/e2e/verify_audit_columns.py --tenant 00000000-0000-4000-8000-000000000001` | PASS | Run on every soak snapshot; last snapshot verify status `0`, verify-chain GREEN |
| `helm template spendguard charts/spendguard --set chart.profile=demo` | PASS | Rendered `/tmp/ga07-helm-demo.yaml`, 1441 lines |
| `helm template spendguard charts/spendguard --set chart.profile=production -f scripts/helm-validate-test-values.yaml` | PASS | Rendered `/tmp/ga07-helm-prod.yaml`, 1534 lines |
| `scripts/soak/ga-soak.sh --duration 30s --interval 30s --profile local --no-reset --evidence-dir /tmp/ga07-review-soak-r1` | PASS | R1 smoke after fail-closed + metadata fix. Summary included branch, commit, command line, environment, machine descriptor, and `git_dirty=false` |
| `scripts/soak/ga-soak.sh --duration 1s --interval 0s --profile local --no-reset --evidence-dir /tmp/ga07-zero-interval-test` | PASS (negative) | R2 fix rejects zero interval before touching docker: exit `2`, `--interval must be > 0` |
| `scripts/soak/ga-soak.sh --duration 90s --interval 30s --profile local --no-reset --evidence-dir /tmp/ga07-stopped-container-test` + `docker stop spendguard-tokenizer` after snapshot 0 | PASS (negative) | Harness exited `1` and wrote failure summary with tokenizer metrics probe failure, `spendguard-tokenizer status is exited`, and `spendguard-tokenizer health is unhealthy` |
| `scripts/soak/ga-soak.sh --duration 30s --interval 30s --profile local --no-reset --evidence-dir /tmp/ga07-review-soak-r2` | PASS | R2 happy-path smoke after probe-failure capture; 2 snapshots, pending `0`, lag `0`, failures `[]` |
| `scripts/soak/ga-soak.sh --duration 30s --interval 30s --profile local --no-reset --evidence-dir /tmp/ga07-inspect-failure-test-r2` + `docker rm -f spendguard-tokenizer` before snapshot 0 | PASS (negative) | R3 inspect-failure fix wrote a fail summary with structured failures: `docker stats failed`, `tokenizer escalation metric failed`, and concise `docker inspect failed ... no such object` |
| `scripts/soak/ga-soak.sh --duration` | PASS (negative) | R4 CLI hardening exits `2` with `--duration requires a value` plus usage instead of Bash `unbound variable` |
| `PATH=/tmp/fake-cargo:$PATH scripts/soak/ga-soak.sh --duration 30s --interval 30s --profile local --no-reset --evidence-dir /tmp/ga07-preflight-failure-test` | PASS (negative) | R4 preflight-summary fix exits `1` and writes `ga_soak_summary.json` with `result=fail`, `snapshot_count=0`, and `Rust SVID/mTLS test failed with status 42` |
| `bash -n scripts/soak/ga-soak.sh && git diff --check` | PASS | Shell syntax and whitespace gates clean after R4 fix |
| `ait run --adapter codex --review-mode adversarial --base main --branch ga/GA_07_soak_harness --slice-doc docs/internal/slices/GA_07_soak_harness.md --review-budget deep` | FAIL | Local AIT wrapper rejected `--review-mode` with `unrecognized arguments`; fallback direct codex review used per workflow precedent |
| `codex review --base main` | R1 FINDINGS | P1 snapshot probe failures could fail open under Bash `errexit` suppression; P2 generated summary lacked GA §7 evidence metadata |
| `codex review --base main` | R2 FINDINGS | P2 stopped containers could abort before recording inspect details; P3 zero interval could busy-loop |
| `codex review --base main` | R3 FINDINGS | P2 required 30m evidence was stale relative to final script; P2 `docker inspect` failure path still returned before structured evidence |
| `codex review --base main` | R4 FINDINGS | P2 metrics/HTTP probes lacked bounded timeouts; P2 pre-snapshot Rust/Python SVID gates could fail before writing a summary |
| `codex review --base main` | R5 FINDINGS | P1 stats cache checks could pass with `output_distribution_cache_rows=0`; P1 soak timer started before Rust/Python preflight and stats warmup, reducing the sustained evidence window |
| Staff+ arbitration panel | FIX IN-SLICE | Software Architect, Backend Architect, Security Engineer, Database Optimizer, and domain expert voted 5/5 to fix R5 findings in GA_07. No R6 review was run after the max 5 rounds; arbitration decision was final |
| `cargo test --manifest-path services/stats_aggregator/Cargo.toml` | PASS | Full stats_aggregator suite after sparse-outcome decision mirror join: 21 lib tests, 1 main test, 4 Postgres integration tests |
| `bash -n scripts/soak/ga-soak.sh && git diff --check` | PASS | Shell syntax and whitespace gates clean after R5 cache/timing fix |
| `cargo fmt --manifest-path services/stats_aggregator/Cargo.toml --check` | PASS | Formatting gate clean after R5 stats_aggregator changes |
| `helm template spendguard charts/spendguard --set chart.profile=demo` | PASS | R5 render clean: `/tmp/ga07-helm-demo-r5.txt`, 1441 lines |
| `helm template spendguard charts/spendguard --set chart.profile=production -f scripts/helm-validate-test-values.yaml` | PASS | R5 render clean: `/tmp/ga07-helm-prod-r5.txt`, 1534 lines |
| `scripts/soak/ga-soak.sh --duration 30s --interval 30s --profile local --no-reset --evidence-dir /tmp/ga07-r5-fix-smoke` | PASS | R5 happy-path smoke: 2 snapshots, snapshot window 38s, output cache rows 1 freshness 6s, run cache rows 2 freshness 7s, failures `[]` |
| `scripts/soak/ga-soak.sh --duration 90s --interval 30s --profile local --no-reset --evidence-dir /tmp/ga07-cache-negative` + truncate `output_distribution_cache` after snapshot 0 | PASS (negative) | Harness exited `1` and wrote failures for missing output cache rows and freshness `999999s` above the 180s gate |
| `scripts/soak/ga-soak.sh --duration 90s --interval 30s --profile local --no-reset --evidence-dir /tmp/ga07-run-cache-negative` + truncate `run_length_distribution_cache` after snapshot 0 | PASS (negative) | Harness exited `1` and wrote failures for missing run cache rows and freshness `999999s` above the 180s gate |
| Slow-preflight timing probe with fake `cargo` sleeping 3s, `scripts/soak/ga-soak.sh --duration 1s --interval 1s --profile local --no-reset --evidence-dir /tmp/ga07-slow-preflight-test-r2` | PASS | Summary recorded `snapshot_window_seconds=9` for requested duration `1`, proving sustained timing starts after preflight and final snapshot covers the requested window |
| `scripts/soak/ga-soak.sh --duration 30m --profile local` | PASS | R5 final evidence on clean source commit `89a233153e68d7863dc2ab28dfea2a6dee466ff7`: result `pass`, duration `1800`, snapshot window `1800`, snapshots `28`, started `2026-06-01T04:22:01Z`, finished `2026-06-01T04:52:08Z`, `git_dirty=false` |

Last soak snapshot:

| Field | Value |
|---|---:|
| `elapsed_seconds` | 1800 |
| `canonical_events` | 5 |
| `pending_forward_rows` | 0 |
| `outbox_lag_metric_seconds` | 0 |
| `outbox_leader_count` | 1 |
| `stats_cycles_total` | 31 |
| `stats_errors_total` | 0 |
| `last_cycle_age_seconds` | 7 |
| `output_distribution_cache_rows` | 1 |
| `output_distribution_cache_freshness_seconds` | 8 |
| `run_length_distribution_cache_rows` | 2 |
| `run_length_distribution_cache_freshness_seconds` | 9 |
| `svid_probe_status` | 0 |
| `container_count` | 10 |
| `failures` | 0 |
