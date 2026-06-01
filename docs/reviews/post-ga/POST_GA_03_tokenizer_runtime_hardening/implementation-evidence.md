# POST_GA_03 Tokenizer Runtime Hardening - Implementation Evidence

## Scope

Branch: `post-ga/POST_GA_03_tokenizer_runtime_hardening`

Base: `main` at `77bdc8f` (`Merge POST_GA_02 contract spec cleanup`)

Mapped issues: #92, #94, #96, #98, #100, #103, #105, #110, #111, #112, #114, #115, #117, #118, #119, #126, #127, #129, #133, #135, #148, #149, #151, #152, #156.

## Commits

| Commit | Purpose |
|---|---|
| `c6a702b` | Runtime gates: request cap, readyz bound-to-listener state, request_id validation/minting, encode timeout, rate-limit metric |
| `582f5cc` | Decision audit envelopes: schema bundle on CONTINUE/DENY and resume policy helper |
| `bd8f880` | Tokenizer crate dispatch: non-OpenAI dispatch hygiene, null sentinel rename, vendor fixtures, Python parity script, 10K benches |
| `2f5a268` | Shadow sink semantics: clone tonic client per emit, sample time before persistence latency, drift payload canonical model, migration lint |
| `32fe3ac` | Cluster wiring docs: metrics NetworkPolicy, production values, UDS hostPath runbook |
| `b0de004` | Real shadow worker boot coverage with Postgres testcontainer and canonical-ingest tonic mock |
| `09e08f2` | Benchmark feature declaration for opt-in Cohere benches |
| `de3a668` | Demo gate unblock: webhook receiver maps typed `IDEMPOTENCY_CONFLICT` to HTTP 409 |

## Issue Closure Map

| Issue(s) | Evidence |
|---|---|
| #96 | `/readyz` is backed by an `Arc<AtomicBool>` set only after TCP/UDS listener bind path is created. |
| #111, #115 | `request_id` accepts minted UUIDv7 for empty values, accepts UUIDv7, accepts UUIDv4 with metric, and rejects invalid/other UUID versions. |
| #100, #114, #127 | Shared 4 MiB request cap, gRPC decode cap, spec SLO amendment, and `spawn_blocking` + timeout around encode path. |
| #110 | Count-token quota path increments `spendguard_tokenizer_rate_limited_total{reason="count_tokens_quota"}`. Tenant/model detail remains in structured logs to avoid high-cardinality metrics. |
| #92, #94, #152 | Sidecar decision audit CloudEvents now populate `schema_bundle_id`; resume helper test covers the policy field. |
| #98 | Heuristic public sentinel renamed to `TIER3_NULL_SENTINEL_VERSION_ID`. |
| #112, #135, #148, #149 | Shadow sample timestamps are captured before persistence latency; drift alert payload includes `canonical_model`; migration lint rejects `drift_alert_decided` defaults. |
| #117, #118, #119, #126, #129, #133, #151 | Vendor fixture diversity, Python parity script, tampered asset tests, cache dispatch smoke tests, 10K bench coverage, and real worker boot test. |
| #103, #105 | Helm metrics ingress NetworkPolicy, production example values, NOTES/runbook for tokenizer UDS hostPath pre-provisioning. |
| #156 | Canonical ingest shadow sink no longer serializes all emits behind one `Mutex`; concurrency test uses a tonic mock server. |

## Verification

Passed before evidence capture:

- `cargo build --manifest-path services/tokenizer/Cargo.toml && cargo test --manifest-path services/tokenizer/Cargo.toml`
- `cargo build --manifest-path services/sidecar/Cargo.toml && cargo test --manifest-path services/sidecar/Cargo.toml`
- `cargo build --manifest-path services/webhook_receiver/Cargo.toml && cargo test --manifest-path services/webhook_receiver/Cargo.toml`
- `cargo test --manifest-path crates/spendguard-tokenizer/Cargo.toml`
- `cargo check --manifest-path benchmarks/tokenizer/Cargo.toml --benches`
- `scripts/ga/verify-tokenizer-python-parity.py`
- `scripts/ga/lint-tokenizer-t1-migrations.sh`
- `helm template charts/spendguard --set chart.profile=demo`
- `helm template charts/spendguard -f charts/spendguard/values-production.example.yaml --set chart.profile=production`
- `cargo test --manifest-path services/tokenizer/Cargo.toml boot_shadow_worker_real_wiring_connects_postgres_and_canonical_ingest`
- `make demo-down`
- `make demo-up DEMO_MODE=default`

Targeted tests passed during implementation:

- `cargo test --manifest-path services/tokenizer/Cargo.toml request_id`
- `cargo test --manifest-path services/sidecar/Cargo.toml resume_policy`
- `cargo test --manifest-path services/sidecar/Cargo.toml resume_emit_helper_populates_cloud_event_policy_field`
- `cargo test --manifest-path services/sidecar/Cargo.toml allow_path_audit_payload_includes_reason_codes`
- `cargo run --manifest-path crates/spendguard-tokenizer/Cargo.toml --example discover_fixture_tokens --release`
- `cargo test --manifest-path services/tokenizer/Cargo.toml canonical_ingest_sink_clones_client_for_concurrent_emit`
- `cargo test --manifest-path services/tokenizer/Cargo.toml sampled_at_is_captured_before_persistence_latency`
- `cargo test --manifest-path services/tokenizer/Cargo.toml drift_above_threshold_emits_signed_cloudevent`
- `cargo test --manifest-path services/webhook_receiver/Cargo.toml idempotency_conflict_maps_to_http_409`

## Demo Gate Notes

`make demo-up DEMO_MODE=default` was run, not simulated.

First attempt exposed a real compile blocker in `services/webhook_receiver/src/domain/error.rs`: the typed `common.v1.Error.Code::IDEMPOTENCY_CONFLICT` enum variant was not handled. Commit `de3a668` maps it to HTTP 409 and adds a regression test.

Second attempt reached the demo assertions but failed because the previous partial run had left durable Postgres volume state, doubling the expected ledger counts. `make demo-down` was run to remove containers and named volumes. The clean rerun passed:

- demo handshake ok
- release smoke ok
- decision CONTINUE ok
- provider_report webhook ok
- Step 8 SQL assertions PASS
- audit_outbox forwarder closure PASS
- canonical_events count observed as 5

`helm template --notes` was not used because the installed Helm rejects `--notes`; the NOTES template was inspected directly and normal demo/production templates rendered cleanly.
