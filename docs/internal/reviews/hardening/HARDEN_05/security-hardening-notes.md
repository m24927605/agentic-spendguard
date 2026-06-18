# HARDEN 05 Security Hardening Notes

## Scope

- CloudEvent replay protection now claims `(producer_id, event_id)` and globally reserves `event_id` in `canonical_event_replay_dedup` before immutable append or quarantine.
- Tokenizer Tier 1 shadow provider calls are default-denied until `tokenizer_shadow_security_settings.pii_shadow_enabled=true` for the tenant.
- `count_tokens` calls are capped per `(tenant_id, provider)` per minute through a shared control-plane DB ledger so tokenizer replicas cannot multiply the tenant cap.
- Rust binaries that use rustls through SQL/TLS clients install `rustls::crypto::aws_lc_rs::default_provider()` at boot.
- Unused tonic `gzip` feature flags were removed from service manifests.

## Locked Decisions

- The replay horizon is operationally bounded by `expires_at` and indexed for cleanup, but canonical `event_id` is globally reserved during quarantine and remains globally unique through `canonical_events_global_keys` after append.
- A replay with identical `(producer_id, event_id, payload_hash)` is idempotently `DEDUPED`; a hash mismatch or cross-producer `event_id` collision returns a duplicate/tamper error and does not append.
- Migration 0020 backfills non-released quarantine rows as `reservation_only=true` with non-expiring `event_id` reservations because the original CloudEvent hash cannot be reconstructed from legacy quarantine rows.
- Missing tenant shadow security settings mean `pii_shadow_enabled=false` and `count_tokens_quota_per_minute=0`.
- Provider keys alone are insufficient to send raw prompt text; control-plane tenant opt-in is required.
- The tokenizer shadow worker uses the dedicated `tokenizer_shadow_runtime_role` DB URL: SELECT on sampling/security settings plus DML only on quota usage, not control-plane setting mutation privileges.
- Quota exhaustion or quota-DB failure skips only the async shadow path and does not touch the Tier 2 tokenizer hot path.

## Verification Greps

```bash
rg -n "tonic = \\{[^\\n]*gzip|CompressionEncoding|send_gzip|accept_gzip|accept_compressed|send_compressed" services crates -g 'Cargo.toml' -g '*.rs'
rg -n "canonical_event_replay_dedup|payload_hash|pii_shadow_enabled|count_tokens_quota_per_minute" services/canonical_ingest services/tokenizer services/control_plane
rg -n "install_default\\(\\)" services/*/src/main.rs services/*/src/bin/*.rs
```

## Tests

- `cargo test --manifest-path services/canonical_ingest/Cargo.toml replay_dedup -- --nocapture`
- `cargo test --manifest-path services/tokenizer/Cargo.toml count_tokens_quota -- --nocapture`
- `cargo test --manifest-path services/tokenizer/Cargo.toml worker::tests -- --nocapture`
- `cargo test --manifest-path services/control_plane/Cargo.toml tokenizer_sampling_auth_tests -- --nocapture`
- `cargo build --manifest-path services/{bundle_registry,cost_advisor,tokenizer,canonical_ingest}/Cargo.toml`
