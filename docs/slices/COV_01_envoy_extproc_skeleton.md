# COV_01 — D01 Envoy ExtProc: skeleton

> **Deliverable**: D01 Envoy AI Gateway ExtProc sidecar
> **Slice**: 1 of 7 (M)
> **Spec set**: [`docs/specs/coverage/D01_envoy_extproc/`](../specs/coverage/D01_envoy_extproc/)

## Scope

Lay down the new `services/envoy_extproc/` Rust crate, wire the upstream `envoy.service.ext_proc.v3` proto via `tonic-build`, extract the existing routing logic from `services/egress_proxy/src/routing.rs` into a new shared `crates/spendguard-provider-routing/` crate that both `egress_proxy` and `envoy_extproc` consume, and prove the `Handshake` RPC against the existing sidecar UDS using the `decision` demo mode as a sanity check.

Concretely:
- `services/envoy_extproc/Cargo.toml` — bin + lib, deps on tonic, prost, tokio, sidecar adapter proto re-exports, the new shared routing crate, tracing/metrics.
- `services/envoy_extproc/build.rs` — `tonic_build` against the vendored ExtProc proto (place under `services/envoy_extproc/proto/envoy/service/ext_proc/v3/external_processor.proto` + dependencies).
- `crates/spendguard-provider-routing/` — extract `ProviderKind` + the `route()` function from `services/egress_proxy/src/routing.rs` into a public lib. Egress proxy continues to compile against the same surface (re-export shim if needed).
- `services/envoy_extproc/src/main.rs` — bin entry. Binds gRPC server on configured port (default `:9443`), serves `Process` streaming RPC stub (handles only the Handshake phase initially; other phases come in SLICE 2-4).
- `services/envoy_extproc/src/lib.rs` — library surface.
- `services/envoy_extproc/src/handshake.rs` — Handshake phase handler. Echoes config + opens sidecar UDS via existing sidecar adapter client.
- `Cargo.toml` workspace exclude entry for new service + new crate.
- Skeleton demo proof: a `cargo run -p spendguard-envoy-extproc` against `make demo-up DEMO_MODE=decision` should accept an Envoy mock client's Handshake without panic and close cleanly.

## Files touched

| File | Why |
|------|-----|
| `services/envoy_extproc/Cargo.toml` | New service manifest |
| `services/envoy_extproc/build.rs` | tonic_build wiring |
| `services/envoy_extproc/proto/envoy/service/ext_proc/v3/external_processor.proto` | Vendored proto (+ deps) |
| `services/envoy_extproc/src/main.rs` | Binary entry |
| `services/envoy_extproc/src/lib.rs` | Library surface |
| `services/envoy_extproc/src/handshake.rs` | Handshake handler |
| `crates/spendguard-provider-routing/Cargo.toml` | Shared routing crate |
| `crates/spendguard-provider-routing/src/lib.rs` | Public routing API |
| `services/egress_proxy/src/routing.rs` | Switch to consume the shared crate (preserve current behaviour byte-identical) |
| `services/egress_proxy/src/providers/bedrock.rs` | Re-export `dispatch_tokenizer_kind` from the shared crate so in-tree call sites compile byte-identical after the extraction |
| `services/egress_proxy/Cargo.toml` | Add dep on the new shared crate |
| `Cargo.toml` (workspace root) | exclude entries for new service + crate |
| `deploy/demo/runtime/Dockerfile.egress_proxy` | `COPY crates/spendguard-provider-routing` so the egress_proxy container build resolves the new path dep |

## Test/verification plan

1. `cargo build --manifest-path services/envoy_extproc/Cargo.toml` succeeds.
2. `cargo build --manifest-path services/egress_proxy/Cargo.toml` STILL succeeds (routing extraction didn't break it).
3. `cargo test --manifest-path crates/spendguard-provider-routing/Cargo.toml` — at minimum the existing routing tests moved over and pass.
4. `cargo test --manifest-path services/envoy_extproc/Cargo.toml` — a smoke test that boots the gRPC server on a random port, opens a mock ExtProc client, sends a Handshake frame, expects an Ack response, closes.
5. `make demo-up DEMO_MODE=decision` — STILL passes (no regression on existing demo).
6. `cargo fmt --check` on the touched crates.

## Anti-scope

- No token counting — SLICE 2.
- No budget decision translation — SLICE 3.
- No audit emission — SLICE 4.
- No Helm — SLICE 6.
- No new demo mode — SLICE 7.
- No upstream PR to envoyproxy/ai-gateway (per anti-scope §5).

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D01_envoy_extproc/design.md) §4 slice 1 row, §3 locked decisions
- Build plan: [`framework-coverage-build-plan-2026-06.md`](../strategy/framework-coverage-build-plan-2026-06.md) §1.5
- Review standards: [`review-standards.md`](../specs/coverage/D01_envoy_extproc/review-standards.md)
