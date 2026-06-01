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
| pending Round 1 fix | Metrics NetworkPolicy preserves public ingress and tokenizer encode timeout is configurable/sized for the accepted request cap |

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

## Adversarial Review Round 3

Required AIT command was run and recorded in `round-3-ait-command.txt`.
Local AIT again rejected `--review-mode`, so codex CLI fallback review
was run and recorded in `round-3-codex-review.txt`.

Finding:

- P2: `ENCODE_CONCURRENCY_LIMITED_TOTAL` was incremented when the
  semaphore budget rejected a request, but the counter was not exported
  by `/metrics`.

Round 3 fix:

- `render_metrics()` now exports
  `spendguard_tokenizer_encode_concurrency_limited_total` with
  Prometheus HELP/TYPE metadata and the current counter value.
- `render_metrics_contains_shadow_counters` now asserts the metric is
  present.

Round 3 fix verification:

- `cargo fmt --manifest-path services/tokenizer/Cargo.toml`
- `cargo test --manifest-path services/tokenizer/Cargo.toml render_metrics_contains_shadow_counters`
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

## Adversarial Review Round 1

Required AIT command was run and recorded in `round-1-ait-command.txt`.
Local AIT rejected `--review-mode`, so codex CLI fallback review was run
and recorded in `round-1-codex-review.txt`.

Findings:

- P1: metrics ingress NetworkPolicy selected all SpendGuard pods and
  isolated public service ingress unless the ingress source was
  same-release, L2-enforced app pods, or Prometheus.
- P2: tokenizer accepted up to 4 MiB but used a fixed 100ms encode
  timeout, making legitimate long prompts vulnerable to timeout.

Round 1 fixes:

- `charts/spendguard/templates/networkpolicy.yaml` now preserves public
  service ingress on configured public ports while retaining same-release
  pod ingress, egress_proxy listener ingress, and Prometheus metrics
  ingress.
- `networkPolicy.metricsIngress.publicPorts` defaults to webhook
  receiver HTTPS `8443` and control-plane HTTP `8091` in demo and
  production values.
- `SPENDGUARD_TOKENIZER_ENCODE_TIMEOUT_MS` is now a service config field
  wired through Helm as `tokenizer.encodeTimeoutMs`, defaulting to 30s.
- `TokenizerSvc` accepts a per-instance timeout; zero is coerced to the
  safe default.
- `docs/tokenizer-service-spec-v1alpha1.md` now documents that operators
  lowering the encode timeout must also lower upstream request caps or
  prove long-prompt benchmarks stay below the configured value.

Round 1 fix verification:

- `cargo test --manifest-path services/tokenizer/Cargo.toml encode_timeout`
- `cargo test --manifest-path services/tokenizer/Cargo.toml defaults_load_with_minimum_env`
- `helm template charts/spendguard --set chart.profile=demo`
- `helm template charts/spendguard -f charts/spendguard/values-production.example.yaml --set chart.profile=production`
- `rg "public service ingress|port: 8443|SPENDGUARD_TOKENIZER_ENCODE_TIMEOUT_MS|port: 8091" /tmp/spendguard-postga03-r1fix-prod.yaml`
- `cargo build --manifest-path services/tokenizer/Cargo.toml && cargo test --manifest-path services/tokenizer/Cargo.toml`

## Adversarial Review Round 2

Required AIT command was run and recorded in `round-2-ait-command.txt`.
Local AIT again rejected `--review-mode`, so codex CLI fallback review
was run and recorded in `round-2-codex-review.txt`.

Finding:

- P2: `tokio::time::timeout` bounds caller wait time, but dropping a
  `spawn_blocking` join handle does not cancel the encode work. Timed
  out requests could continue consuming Tokio blocking workers.

Round 2 fix:

- `TokenizerSvc` now enforces `DEFAULT_ENCODE_MAX_CONCURRENT` through a
  per-service `Semaphore`.
- The semaphore permit is moved into the blocking encode closure, so it
  is held until actual encode completion even if the RPC returns
  `DeadlineExceeded`.
- Exhausted encode budget returns `ResourceExhausted` and increments
  `ENCODE_CONCURRENCY_LIMITED_TOTAL`.
- `SPENDGUARD_TOKENIZER_ENCODE_MAX_CONCURRENT` is wired through config
  and Helm as `tokenizer.encodeMaxConcurrent`, defaulting to 32.
- The tokenizer service spec now documents timeout as a caller wait
  bound and max-concurrent as the CPU work budget.

Round 2 fix verification:

- `cargo test --manifest-path services/tokenizer/Cargo.toml encode`
- `helm template charts/spendguard --set chart.profile=demo`
- `helm template charts/spendguard -f charts/spendguard/values-production.example.yaml --set chart.profile=production`
- `rg "SPENDGUARD_TOKENIZER_ENCODE_MAX_CONCURRENT|SPENDGUARD_TOKENIZER_ENCODE_TIMEOUT_MS|port: 8443|port: 8091" /tmp/spendguard-postga03-r2fix-prod.yaml`
- `cargo build --manifest-path services/tokenizer/Cargo.toml && cargo test --manifest-path services/tokenizer/Cargo.toml`
