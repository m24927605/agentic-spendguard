# SpendGuard Changelog

All notable product-level changes are recorded here. SDK-only releases continue to use `sdk/python/CHANGELOG.md`.

Version tags follow `vYYYY.MM.DD-ga.N` for GA release candidates and GA releases.

## Unreleased

- GA readiness phase started after HARDEN_08.
- Release bundle tooling added in GA_01.

## v2026.05.31-ga.0 - 2026-05-31

### Summary

- Predictor upgrade SLICE_01 through SLICE_15 completed.
- HARDEN_01 through HARDEN_08 completed and merged to main.
- Legacy egress heuristic is replaced by predictor-backed budget projection and audit mirror columns.
- Python SDK 0.5.0 is the predictor-upgrade SDK line.

### Operator Highlights

- Production blockers #90, #137, #143, #145, #150, #160, #168, #169, and #171 are closed.
- Demo modes verified during hardening include `default`, `m1_benchmark_runaway_loop`, `multi_provider_usd`, `agent_real_anthropic`, and `plugin_c_synthetic`.
- Per-tenant SVID plugin identity is enforced for Strategy C production readiness.

### Migration Notes

- Ledger, canonical ingest, and control-plane migrations must be applied in documented order.
- Immutable audit data must be treated as forward-fix only; do not plan destructive rollback for canonical audit history.

### Security Notes

- Database URLs are expected to come from Kubernetes Secret references in production Helm.
- Container security baseline remains required: non-root user, read-only root filesystem, no privilege escalation, and dropped capabilities.
- Supply-chain signing, SBOM, and vulnerability scan gates are owned by GA_09.
