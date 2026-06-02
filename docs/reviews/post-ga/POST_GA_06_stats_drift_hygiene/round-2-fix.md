# Round 2 Fix

## Reviewer Findings

- Major: `detect_and_emit` wrote the 24h cooldown before signing and durable append. A transient signing or canonical_ingest failure could suppress a real alert for the full cooldown window even though no immutable alert existed.
- Minor: the non-finite z-score SQL test used synthetic `prompt_class` values that could also violate the prompt-class enum, making the test less isolated.

## Fix

- Split `DriftAlertCooldown` into `check` and `record_emitted`.
- `detect_and_emit` now checks cooldown before alert construction, signs and appends the CloudEvent, and records cooldown only after `sink.emit` succeeds.
- Failed append no longer writes cooldown; a follow-up cycle can retry the alert.
- If cooldown recording fails after a successful append, the durable alert is still counted and the daemon logs duplicate-suppression risk.
- The SQL CHECK test now binds legal `prompt_class = 'chat_short'` while varying only `last_z_score`.

## Verification

- `cargo fmt --manifest-path services/stats_aggregator/Cargo.toml`: PASS
- `cargo test --manifest-path services/stats_aggregator/Cargo.toml`: PASS
- `cargo build --manifest-path services/stats_aggregator/Cargo.toml`: PASS
- `helm template charts/spendguard --set chart.profile=demo`: PASS
- `helm template charts/spendguard -f charts/spendguard/values-production.example.yaml --set chart.profile=production`: PASS
- `make demo-down && make demo-up DEMO_MODE=default`: PASS
- `docker compose -f deploy/demo/compose.yaml up -d --build stats-aggregator && curl /healthz && curl /metrics`: PASS
