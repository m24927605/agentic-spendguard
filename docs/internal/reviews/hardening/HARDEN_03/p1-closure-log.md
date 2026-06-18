# HARDEN_03 P1 Closure Log

Date: 2026-05-31
Branch: `harden/HARDEN_03_production_blocker_gh_triage`

| Issue | Fix commit(s) | Verification |
|---:|---|---|
| #90 | `307eed4` | `make -C sdk/python proto`; `make -C sdk/python test` passed (`852 passed, 4 skipped`). |
| #137 | `071ae54`, `9d444ab` | `cargo test --manifest-path services/control_plane/Cargo.toml` passed (`45` tests). |
| #143 | `307eed4` | `cargo test --manifest-path services/canonical_ingest/Cargo.toml` passed; verify-chain unit tests cover drift alerts not requiring prediction mirrors. |
| #145 | `ad85d9b` | `helm lint charts/spendguard`; demo/prod `helm template`; rendered-manifest grep for `postgres://`, `postgresql://`, and `CHANGE_ME` returned empty. |
| #150 | `307eed4` | `cargo test --manifest-path services/canonical_ingest/Cargo.toml` passed; migration smoke check now filters `schemaname='public'`. |
| #160 | `88461a5` | `cargo test --manifest-path services/stats_aggregator/Cargo.toml --features integration --test cycle_e2e_postgres -- --nocapture` passed (`3` real Postgres tests). |
| #168 | `3925551` | `cargo test --manifest-path services/tokenizer/Cargo.toml append_request_carries_required_observability_envelope`; demo Helm render passed. |
| #169 | `307eed4` | Canonical ingest append handler tests cover model mirror extraction and `model_family` fallback into `model`; sidecar/egress payload emission paths already carry model, prompt_class, and prompt_class_fingerprint. |
| #171 | none in HARDEN_03 | Left open by design. Ownership is HARDEN_08 per `docs/internal/slices/HARDEN_08_per_tenant_svid_cert.md`; HARDEN_03 posts a tracking comment only. |

GitHub disposition after this branch:
- Close: #90, #137, #143, #145, #150, #160, #168, #169.
- Comment and keep open: #171.
