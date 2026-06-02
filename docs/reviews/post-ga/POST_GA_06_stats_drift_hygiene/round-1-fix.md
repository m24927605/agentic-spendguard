# Round 1 Fix

## Reviewer Finding

- P2: `prediction_drift_alert_cooldowns.agent_id` used `octet_length(agent_id) <= 256`, while canonical aggregator mirror columns allow `char_length(agent_id) <= 128`. A valid multibyte 128-character agent ID could be rejected by the cooldown store, causing fail-safe alert suppression.

## Fix

- Changed migration 0022 key constraints to mirror canonical aggregator columns:
  - `model`: `char_length(model) BETWEEN 1 AND 64`
  - `agent_id`: `char_length(agent_id) BETWEEN 1 AND 128`
  - `prompt_class`: canonical 7-class enum
- Updated tests to use canonical `rag` instead of non-canonical `rag_short`.
- Added `drift_alert_cooldown_postgres_accepts_canonical_multibyte_agent_id` with a 128-character CJK agent ID.

## Verification

- `cargo fmt --manifest-path services/stats_aggregator/Cargo.toml`: PASS
- `cargo test --manifest-path services/stats_aggregator/Cargo.toml`: PASS
- `cargo build --manifest-path services/stats_aggregator/Cargo.toml`: PASS
- `helm template charts/spendguard --set chart.profile=demo`: PASS
- `helm template charts/spendguard -f charts/spendguard/values-production.example.yaml --set chart.profile=production`: PASS
- `make demo-down && make demo-up DEMO_MODE=default`: PASS
- `docker compose -f deploy/demo/compose.yaml exec -T postgres psql ... pg_get_constraintdef`: PASS
- `docker compose -f deploy/demo/compose.yaml up -d --build stats-aggregator && curl /healthz && curl /metrics`: PASS
