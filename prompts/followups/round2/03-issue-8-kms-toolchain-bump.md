# Round-2 #8 — Live KMS via project-wide rust 1.91 bump

GitHub issue: #8. Original prompt: `../05-issue-8-live-kms-signing.md`.

## Why round 2

Round-1 attempt failed on MSRV: `aws-sdk-kms 1.x` transitive deps
(`aws-credential-types`, `aws-runtime`, `aws-smithy-*`) require **rustc
1.91.1**; project Dockerfiles all use `rust:1.88-bookworm`. Pinning at
the kms-crate level alone doesn't constrain transitives.

## Round-2 strategy (split into 2 PRs)

### PR 8a — Toolchain bump

1. Find every Dockerfile under `deploy/demo/runtime/Dockerfile.*` and
   bump `FROM rust:1.88-bookworm` → `FROM rust:1.91-bookworm` (or the
   latest stable bookworm tag at execution time).
2. cargo check every Rust service crate in isolation against rust:1.91:
   - signing, leases, policy, auth, doctor, usage_poller
   - canonical_ingest, ledger, sidecar, control_plane, dashboard,
     outbox_forwarder, ttl_sweeper, webhook_receiver, retention_sweeper
3. If any crate breaks on the new toolchain, pin the offending dep or
   adjust the code; do NOT roll back the toolchain bump.
4. `make demo-up DEMO_MODE=decision` regression after the toolchain bump.

Acceptance:
- All 15+ services cargo-check clean on rust:1.91
- DEMO_MODE=decision passes
- No new clippy warnings introduced (or, if introduced, fixed)

### PR 8b — KmsSigner real impl

After PR 8a merges:

1. `services/signing/Cargo.toml`: add `aws-config = "1"` +
   `aws-sdk-kms = "1"` (no version pin needed once toolchain supports them).
2. Implement `KmsSigner` per the original prompt: ECDSA_SHA_256 default,
   `with_client(...)` test hook, ECDSA verifier in
   canonical_ingest. (See round-1 attempt notes on the issue's comment
   thread for the implementation that was already drafted.)
3. `signer_from_env(...)` becomes async (it was already async-friendly
   internally; just add `.await` on `KmsSigner::new`).
4. 3 caller mains add `.await` (sidecar, ttl_sweeper, webhook_receiver).
5. Cargo test signing crate (no LocalStack required for unit tests —
   verifier with metadata only).

Acceptance:
- `cargo test --lib spendguard-signing` passes
- DEMO_MODE=decision regression
- LocalStack KMS round-trip deferred to operator integration

## Risk

PR 8a is wide. Breaking any service at the toolchain level reverts the
whole change. Be prepared to pin transitive deps if rust 1.91 surfaces
new lints / breaks (e.g. `proc-macro2`, `serde_derive`).
