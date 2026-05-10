# Round-2 #11 — Per-service /metrics endpoints

GitHub issue: #11. Original prompt: `../06-issue-11-per-service-metrics.md`.

## Why round 2 + scope adjustment

Original prompt asked for 8 services in a single PR. Round-2 strategy:
**ship one service per PR, ledger first as proof-of-pattern.** Each subsequent
service follows the same shape and the autonomous executor can stop at any
point without leaving main in a broken state.

## Pattern reference

`services/canonical_ingest/src/metrics.rs` is the canonical impl: no
`prometheus` crate, raw `AtomicU64` + manual Prometheus text format render.
`services/canonical_ingest/src/main.rs::serve_metrics` is the hyper-based
HTTP server pattern.

## Per-PR shape (one service per PR)

For service `S` (start with `ledger`):

1. New `services/S/src/metrics.rs` mirroring canonical_ingest's shape
2. `Cargo.toml` adds `hyper = { version = "1", features = ["server", "http1"] }`,
   `hyper-util = { version = "0.1", features = ["tokio"] }`,
   `http-body-util = "0.1"`
3. `src/main.rs` spins up `serve_metrics` task on port 9092 (ledger; per
   issue table — sidecar 9093, webhook_receiver 9094, etc.)
4. Wire counter increments at the gRPC handler entry points (ledger:
   `services/ledger/src/handlers/*.rs` — at least one increment per
   handler function: `post_*_total`)
5. Helm chart's `<service>.yaml`: add metrics port to `containerPort` +
   matching `Service` ports + `metricsPort` knob in values.yaml
6. compose.yaml: add port mapping for the new metrics endpoint
7. Cargo test: smoke test that `metrics.render()` returns non-empty text
   with expected counter labels

## Acceptance per PR

- [ ] `cargo test --lib` passes for the affected service
- [ ] `helm template t charts/spendguard | grep -E "containerPort: 909[1-9]"` shows the new port
- [ ] DEMO_MODE=decision regression PASS (curl to the new metrics port returns text)
- [ ] PR body documents which counters are incremented + at which handler

## Ship order

ledger → sidecar → control_plane → dashboard → outbox_forwarder →
ttl_sweeper → webhook_receiver → usage_poller. (Highest leverage first;
canonical_ingest already has its endpoint.)

## Round-2 starting commit

Latest main as of round-2 kickoff: see `git log origin/main -1`.
