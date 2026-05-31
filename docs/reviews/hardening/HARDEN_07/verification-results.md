# HARDEN 07 Verification Results

Date: 2026-05-31
Branch: `harden/HARDEN_07_cargo_helm_migration_verification`

## Cargo

Command:

```bash
scripts/verify-cargo-workspace.sh
```

Result: PASS.

- Checked metadata for 30 Cargo manifests.
- Used `--locked` for manifests with committed lockfiles and `--no-deps` for library/service manifests without a committed lockfile.
- Built the predictor-upgrade Rust set:
  - `benchmarks/predictor-upgrade/Cargo.toml`
  - `services/canonical_ingest/Cargo.toml`
  - `services/control_plane/Cargo.toml`
  - `services/egress_proxy/Cargo.toml`
  - `services/ledger/Cargo.toml`
  - `services/output_predictor/Cargo.toml`
  - `services/run_cost_projector/Cargo.toml`
  - `services/sidecar/Cargo.toml`
  - `services/stats_aggregator/Cargo.toml`
  - `services/tokenizer/Cargo.toml`
- No Cargo.lock drift remained after verification.

## Helm

Command:

```bash
scripts/verify-helm-profiles.sh
```

Result: PASS.

Rendered matrix:

- `demo`
- `demo-networkpolicy`
- `production`
- `production-networkpolicy`
- `production-kms`

Additional checks:

- Rendered manifests contain no plaintext `postgres://` URLs.
- Production KMS control-plane render contains no local signing Secret mount or `control-plane.pem` reference.
- Rendered manifests retain baseline security tokens including `runAsUser: 65532`, `readOnlyRootFilesystem: true`, and capability drop `ALL`.

## Migrations

Command:

```bash
scripts/verify-migrations-postgres16.sh
```

Result: PASS.

- Started a fresh `postgres:16-alpine` container.
- Applied all ledger migrations in sorted order: 51 SQL files.
- Applied all canonical_ingest migrations in sorted order: 20 SQL files.
- Applied all control_plane migrations in sorted order: 5 SQL files.
- Smoke checks proved:
  - `audit_outbox` and `tokenizer_t1_samples` exist with prediction columns.
  - `canonical_events` and `canonical_event_replay_dedup` exist with mirror columns.
  - `predictor_plugin_endpoints`, `control_plane_audit_outbox`, and `control_plane_audit_outbox_forwarder_update` policy exist.

## NetworkPolicy

Command:

```bash
tests/k8s/networkpolicy_egress_chaos.sh
```

Result: PASS.

- Created kind cluster `spendguard-netpol` with default CNI disabled.
- Installed Calico `v3.28.2` so NetworkPolicy enforcement is real.
- Applied the chart's `templates/networkpolicy.yaml` with `networkPolicy.enabled=true`.
- Verified an enforced app pod can reach the in-cluster egress proxy on port 9000.
- Verified the same enforced app pod cannot reach `https://1.1.1.1` directly.

## Demo Regression

Command:

```bash
make demo-down
make demo-up DEMO_MODE=default
```

Result: PASS.

- Demo handshake succeeded.
- `release_reservation`, `RequestDecision`, `ConfirmPublishOutcome`, `emit_llm_call_post`, and webhook provider report all completed.
- Phase 2B Step 8 SQL assertions passed with provider-reported commit state.
- Outbox forwarder drained 7/7 audit rows.
- `canonical_events` verification passed with count 5.
