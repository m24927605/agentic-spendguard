# COV_05 — D01 Envoy ExtProc: conformance fixtures (Envoy AI Gateway v0.6)

> **Deliverable**: D01 Envoy AI Gateway ExtProc sidecar
> **Slice**: 5 of 7 (M)
> **Spec set**: [`docs/specs/coverage/D01_envoy_extproc/`](../../specs/coverage/D01_envoy_extproc/)

## Scope

Lock the wire-level conformance to Envoy AI Gateway v0.6 reference manifest examples (`token_counting.yaml`, `budget.yaml`) via golden-file tests. SLICES 1-4 wired the ExtProc protocol; SLICE 5 proves we conform to the public v0.6 reference for `chat/completions` and `messages` paths.

Concretely:
- `services/envoy_extproc/tests/fixtures/v0_6/` — NEW vendored reference fixtures:
  - `token_counting.yaml` — copy from Envoy AI Gateway v0.6 release tag (cite source URL + tag in fixture README)
  - `budget.yaml` — same
  - Request-Headers / Request-Body / Response-Body sample frames (binary protobuf or JSON)
- `services/envoy_extproc/tests/conformance.rs` — NEW:
  - Loads each v0.6 fixture frame
  - Feeds through the SLICE 1-4 ExtProc pipeline
  - Asserts the resulting ProcessingResponse matches the v0.6 expected shape byte-for-byte (or modulo decision IDs which are non-deterministic)
  - Golden-file diff on mismatch
- ≥10 conformance tests:
  - Each fixture's happy-path (ALLOW)
  - Each fixture's DENY case
  - Each fixture's DEGRADE case (where applicable)
  - Streaming SSE shape (verify v1 commit-at-end pattern matches v0.6 expectation)
  - Header-only path (no body) shape
  - Body-only path shape
  - Trailers-NOT-handled assertion (v1 anti-scope per design §3.5)
- Optional: `services/envoy_extproc/scripts/refresh_fixtures.sh` — fetch latest Envoy AI Gateway tag, refresh fixtures, diff old vs new

## Files touched

| File | Why |
|------|-----|
| `services/envoy_extproc/tests/fixtures/v0_6/` | NEW — vendored v0.6 reference manifests + frames |
| `services/envoy_extproc/tests/conformance.rs` | NEW — golden-file conformance tests |
| `services/envoy_extproc/tests/fixtures/README.md` | NEW — provenance + refresh instructions |

## Test/verification plan

1. `cargo build --manifest-path services/envoy_extproc/Cargo.toml` clean
2. `cargo test --manifest-path services/envoy_extproc/Cargo.toml --test conformance` — ≥10 passing
3. SLICE 1-4 regression: 83 lib + 7 integration = 90 tests unchanged
4. `cargo fmt --check` + `cargo clippy -D warnings` clean
5. Conformance tests fail loudly with golden-diff output on shape mismatch

## Anti-scope

- No Helm — SLICE 6
- No new demo mode — SLICE 7
- No streaming SSE chunk-by-chunk gating (v1 commit-at-end per design §3.5)
- No TRAILERS phase handling (Envoy v0.6 reference doesn't require)

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D01_envoy_extproc/design.md) §4 slice 5 row, §3.5 v1 wire-format scope
- SLICE 4: [`COV_04_envoy_extproc_audit_emit.md`](COV_04_envoy_extproc_audit_emit.md)
