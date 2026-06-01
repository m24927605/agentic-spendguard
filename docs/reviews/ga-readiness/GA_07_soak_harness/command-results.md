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
| `ait run --adapter codex --review-mode adversarial --base main --branch ga/GA_07_soak_harness --slice-doc docs/slices/GA_07_soak_harness.md --review-budget deep` | FAIL | Local AIT wrapper rejected `--review-mode` with `unrecognized arguments`; fallback direct codex review used per workflow precedent |
| `codex review --base main` | R1 FINDINGS | P1 snapshot probe failures could fail open under Bash `errexit` suppression; P2 generated summary lacked GA §7 evidence metadata |
| `codex review --base main` | R2 FINDINGS | P2 stopped containers could abort before recording inspect details; P3 zero interval could busy-loop |
| `codex review --base main` | R3 FINDINGS | P2 required 30m evidence was stale relative to final script; P2 `docker inspect` failure path still returned before structured evidence |
| `codex review --base main` | R4 FINDINGS | P2 metrics/HTTP probes lacked bounded timeouts; P2 pre-snapshot Rust/Python SVID gates could fail before writing a summary |

Last soak snapshot:

| Field | Value |
|---|---:|
| `elapsed_seconds` | 1814 |
| `canonical_events` | 5 |
| `pending_forward_rows` | 0 |
| `outbox_lag_metric_seconds` | 0 |
| `outbox_leader_count` | 1 |
| `stats_cycles_total` | 31 |
| `stats_errors_total` | 0 |
| `last_cycle_age_seconds` | 23 |
| `svid_probe_status` | 0 |
| `container_count` | 10 |
| `failures` | 0 |
