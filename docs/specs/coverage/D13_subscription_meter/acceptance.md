# D13 — Acceptance Gates

Per build plan §3, every gate listed here must be **100% feasible** at slice-spec time: runnable in the current repo state, no third-party action required, reproducible by the `superpowers:code-reviewer` skill.

## 1. Repository-state gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A1.1` | `services/egress_proxy/src/subscription.rs` exists with public `classify()` function | `grep -qE '^pub fn classify' services/egress_proxy/src/subscription.rs` |
| `A1.2` | `services/egress_proxy/src/subscription_meter.rs` exists with public `meter_only_estimate()` + `evaluate_cap()` | `grep -qE 'pub async fn meter_only_estimate' services/egress_proxy/src/subscription_meter.rs && grep -qE 'pub async fn evaluate_cap' services/egress_proxy/src/subscription_meter.rs` |
| `A1.3` | `services/egress_proxy/src/subscription_cap_store.rs` exists with `SubscriptionCapStore` trait | `grep -qE 'pub trait SubscriptionCapStore' services/egress_proxy/src/subscription_cap_store.rs` |
| `A1.4` | `routing.rs` contains a `chatgpt.com/backend-api/codex/responses` row | `grep -qE 'backend-api/codex/responses' services/egress_proxy/src/routing.rs` |
| `A1.5` | `proto/spendguard/common/v1/common.proto` declares `ReservationSource` enum with `BYOK` + `SUBSCRIPTION_METER` values | `grep -qE 'RESERVATION_SOURCE_SUBSCRIPTION_METER' proto/spendguard/common/v1/common.proto` |
| `A1.6` | Migration `0044_audit_outbox_reservation_source.sql` exists | `test -f services/canonical_ingest/migrations/0044_audit_outbox_reservation_source.sql` |
| `A1.7` | Migration `0045_subscription_caps.sql` exists | `test -f services/canonical_ingest/migrations/0045_subscription_caps.sql` |
| `A1.8` | Migration `0046_audit_outbox_import_source.sql` exists | `test -f services/canonical_ingest/migrations/0046_audit_outbox_import_source.sql` |
| `A1.9` | `services/importer_anthropic/Cargo.toml` exists; pkg name `spendguard-importer-anthropic` | `cargo metadata --format-version 1 \| jq -e '.packages[] \| select(.name == "spendguard-importer-anthropic")'` |
| `A1.10` | `services/importer_openai/Cargo.toml` exists; pkg name `spendguard-importer-openai` | `cargo metadata --format-version 1 \| jq -e '.packages[] \| select(.name == "spendguard-importer-openai")'` |
| `A1.11` | `docs/site-v2/src/content/docs/integrations/subscription-meter-claude-code-pro.md` exists | `test -f docs/site-v2/src/content/docs/integrations/subscription-meter-claude-code-pro.md` |
| `A1.12` | `docs/site-v2/src/content/docs/integrations/subscription-meter-codex-chatgpt.md` exists | `test -f docs/site-v2/src/content/docs/integrations/subscription-meter-codex-chatgpt.md` |
| `A1.13` | `README.md` `## Adapter integrations` table includes a "Subscription meter" row | `grep -q 'Subscription meter' README.md` |
| `A1.14` | Fixtures committed: `services/egress_proxy/tests/fixtures/subscription/{claude_code_pro,codex_chatgpt_plus,byok_anthropic,byok_openai,ambiguous_cli_byok}_session.har` all present | `for f in claude_code_pro_session codex_chatgpt_plus_session byok_anthropic byok_openai ambiguous_cli_byok; do test -f "services/egress_proxy/tests/fixtures/subscription/$f.har" \|\| exit 1; done` |
| `A1.15` | `services/egress_proxy/tests/fixtures/subscription/PROVENANCE.md` exists with redaction script SHA-256 pinned | `grep -qE 'redact_har\.py.*sha256' services/egress_proxy/tests/fixtures/subscription/PROVENANCE.md` |

## 2. Build gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A2.1` | Workspace builds | `cargo build --workspace --locked` exits 0 |
| `A2.2` | Egress proxy builds with new modules | `cargo build -p spendguard-egress-proxy --release --locked` exits 0 |
| `A2.3` | Sidecar builds with subscription branch | `cargo build -p spendguard-sidecar --release --locked` exits 0 |
| `A2.4` | Importer stubs build (default features only) | `cargo build -p spendguard-importer-anthropic --locked && cargo build -p spendguard-importer-openai --locked` exits 0 |
| `A2.5` | Importer stubs do NOT pull HTTP client (live feature off) | `cargo tree -p spendguard-importer-anthropic -e=normal \| grep -v reqwest \| grep -v hyper-tls` (returns nothing matching reqwest/hyper-tls) |
| `A2.6` | No new warnings | `cargo build --workspace -- -D warnings` exits 0 |
| `A2.7` | Clippy clean for new modules | `cargo clippy -p spendguard-egress-proxy -p spendguard-sidecar -p spendguard-importer-anthropic -p spendguard-importer-openai --all-targets -- -D warnings` exits 0 |
| `A2.8` | `cargo deny check` passes | `cargo deny check` exits 0 |
| `A2.9` | Proto codegen runs clean | `cargo build -p spendguard-common --locked` exits 0 and `ReservationSource` enum present in generated code (`grep -qE 'pub enum ReservationSource' services/common/target/debug/build/*/out/spendguard.common.v1.rs`) |

## 3. Unit-test gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A3.1` | All classifier unit tests green | `cargo test -p spendguard-egress-proxy --lib subscription::tests` exits 0 |
| `A3.2` | All meter-estimate unit tests green | `cargo test -p spendguard-egress-proxy --lib subscription_meter::tests` exits 0 |
| `A3.3` | All cap-store unit tests green | `cargo test -p spendguard-egress-proxy --lib subscription_cap_store::tests` exits 0 |
| `A3.4` | Routing addition tests green | `cargo test -p spendguard-egress-proxy --lib routing::tests::routes_codex` exits 0 |
| `A3.5` | Sidecar branch tests green | `cargo test -p spendguard-sidecar subscription_meter` exits 0 |
| `A3.6` | Importer contract tests green | `cargo test -p spendguard-importer-anthropic && cargo test -p spendguard-importer-openai` exits 0 |
| `A3.7` | Classifier token-never-logged enforcement test green | `cargo test -p spendguard-egress-proxy classify_never_logs_full_token` exits 0 |

## 4. Fixture-driven integration-test gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A4.1` | All HAR-fixture integration tests green | `cargo test -p spendguard-egress-proxy --test subscription_meter_e2e` exits 0 |
| `A4.2` | BYOK regression tests green (D13 must not break BYOK path) | `cargo test -p spendguard-egress-proxy --test subscription_meter_e2e byok_anthropic_session_uses_ledger byok_openai_session_uses_ledger` exits 0 |
| `A4.3` | Mixed session test green | `cargo test -p spendguard-egress-proxy --test subscription_meter_e2e mixed_session_byok_then_subscription` exits 0 |
| `A4.4` | Hard-cap integration tests green | `cargo test -p spendguard-egress-proxy --test hard_cap_synthetic_429` exits 0 |
| `A4.5` | Soft-cap alert dispatch tests green | `cargo test -p spendguard-egress-proxy --test soft_cap_alert` exits 0 |

## 5. Schema migration gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A5.1` | Migrations apply cleanly to a fresh PG 16 instance | `make -C deploy/demo demo-up && psql "$DATABASE_URL" -c "SELECT column_name FROM information_schema.columns WHERE table_name = 'audit_outbox' AND column_name = 'reservation_source';" \| grep reservation_source` |
| `A5.2` | Migration idempotency: apply twice → no error | `psql "$DATABASE_URL" -f services/canonical_ingest/migrations/0044_audit_outbox_reservation_source.sql` succeeds twice in a row |
| `A5.3` | Existing rows backfill to `byok` | `psql "$DATABASE_URL" -c "SELECT count(*) FILTER (WHERE reservation_source != 'byok') = 0 AS all_byok FROM audit_outbox WHERE occurred_at < (SELECT min(occurred_at) FROM audit_outbox);" \| grep -q t` (only meaningful on a DB that had pre-D13 rows) |
| `A5.4` | CHECK constraint rejects invalid value | `psql "$DATABASE_URL" -c "INSERT INTO audit_outbox (...) VALUES (..., 'invalid_source');"` returns SQLSTATE `23514` |
| `A5.5` | Partial index `idx_audit_outbox_subscription_meter` exists with correct predicate | `psql "$DATABASE_URL" -c "SELECT indexdef FROM pg_indexes WHERE indexname = 'idx_audit_outbox_subscription_meter';" \| grep -qE "reservation_source = 'subscription_meter'"` |
| `A5.6` | `subscription_caps` table has RLS enabled | `psql "$DATABASE_URL" -c "SELECT relrowsecurity FROM pg_class WHERE relname = 'subscription_caps';" \| grep -q t` |
| `A5.7` | `subscription_caps` RLS isolation verified across two tenants | dedicated test runs as two roles and confirms cross-tenant SELECT returns 0 rows |
| `A5.8` | Migration 0044/0045/0046 listed in `migration_inventory.toml` | `grep -qE '0044_audit_outbox_reservation_source' services/canonical_ingest/migration_inventory.toml && grep -qE '0045_subscription_caps' services/canonical_ingest/migration_inventory.toml && grep -qE '0046_audit_outbox_import_source' services/canonical_ingest/migration_inventory.toml` |

## 6. Demo-mode regression gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A6.1` | `make -C deploy/demo demo-verify-subscription-meter-claude-code` exits 0 | Replays Claude Code Pro HAR → asserts meter audit row + no ledger row. |
| `A6.2` | `make -C deploy/demo demo-verify-subscription-meter-codex` exits 0 | Same for Codex / ChatGPT-OAuth HAR. |
| `A6.3` | `make -C deploy/demo demo-verify-subscription-hard-cap` exits 0 | Mode=`hard_cap`, threshold=$0.00, replay → 429 returned, audit row `decision = STOP_RUN_PROJECTION`, upstream never called. |
| `A6.4` | Verifier SQL committed | `test -f deploy/demo/verify_step_subscription_meter_claude_code.sql && test -f deploy/demo/verify_step_subscription_meter_codex.sql` |
| `A6.5` | Pre-existing BYOK demo regression: `make -C deploy/demo demo-verify-litellm-real` still exits 0 | Baseline BYOK path unbroken. |
| `A6.6` | Pre-existing pricing-table demo regression: `make -C deploy/demo demo-verify-pricing` still exits 0 | Pricing snapshot loading still works. |
| `A6.7` | `make -C deploy/demo demo-clean` removes D13-specific artefacts | After clean, `subscription_caps` table empty, audit rows from demo replay purged. |

## 7. Performance gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A7.1` | Classifier p99 < 50 µs | `cargo test -p spendguard-egress-proxy --release -- --ignored classify_p99_under_50us` exits 0 |
| `A7.2` | `meter_only_estimate` p99 < 5 ms on fallback path | `cargo test -p spendguard-egress-proxy --release -- --ignored meter_only_estimate_p99_under_5ms_fallback_path` exits 0 |
| `A7.3` | Hard-cap short-circuit total proxy latency p99 < 2 ms | `cargo test -p spendguard-egress-proxy --release -- --ignored hard_cap_short_circuit_p99_under_2ms` exits 0 |

## 8. Security gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A8.1` | Authorization token never logged in full | `cargo test -p spendguard-egress-proxy classify_never_logs_full_token` exits 0 |
| `A8.2` | Slack soft-cap payload redacts Authorization | `cargo test -p spendguard-egress-proxy soft_cap_slack_payload_redacts_oauth_token` exits 0 |
| `A8.3` | Hard-cap response body cross-tenant leak test green | `cargo test -p spendguard-egress-proxy hard_cap_does_not_leak_other_tenant_usage_in_429` exits 0 |
| `A8.4` | Classifier handles malformed Authorization safely | `cargo test -p spendguard-egress-proxy classify_handles_authorization_with_null_byte classify_handles_giant_user_agent` exits 0 |
| `A8.5` | Fixture provenance documents redaction script hash | `grep -qE 'sha256:[0-9a-f]{64}' services/egress_proxy/tests/fixtures/subscription/PROVENANCE.md` |
| `A8.6` | No fixture contains real OAuth tokens (sentinel-only) | `grep -rE '(sk-ant-oat01-[A-Za-z0-9_-]{20,}\|eyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,})' services/egress_proxy/tests/fixtures/subscription/ \| grep -v FAKE_` exits 1 (no matches) |
| `A8.7` | `subscription_caps` RLS isolation enforced | `cargo test -p spendguard-egress-proxy subscription_caps_rls_isolation` exits 0 |

## 9. Acceptance scenario gate (primary headline gate)

**The headline acceptance scenario** (from the deliverable prompt):

> Against a recorded Claude Code Pro session (replay via fixture, not live), the proxy emits correct estimated $ audit events with reservation source-tagged `subscription_meter`. Soft-cap mode alerts but doesn't block. Hard-cap mode returns 429 at the configured threshold.

This is verified by `A9.1` — `A9.3` running end-to-end against the demo stack:

| ID | Gate | Verification command |
|----|------|----------------------|
| `A9.1` | Meter mode against Claude Code Pro fixture emits correct estimated $ | `make -C deploy/demo demo-verify-subscription-meter-claude-code` exits 0; verifier SQL asserts `amount_micro_usd > 0`, `reservation_source = 'subscription_meter'`, `model LIKE 'claude%'` |
| `A9.2` | Soft-cap mode alerts but does NOT block | `make -C deploy/demo demo-verify-subscription-soft-cap` exits 0; mock Slack endpoint logs receive a POST; upstream stub records the forwarded request |
| `A9.3` | Hard-cap mode returns 429 at configured threshold | `make -C deploy/demo demo-verify-subscription-hard-cap` exits 0; verifier SQL asserts 429 returned, audit row `decision = STOP_RUN_PROJECTION`, upstream stub records zero forwarded requests |

`A9.1` is the **merge-blocking gate** that maps directly to the deliverable prompt's acceptance criterion. `A9.2` and `A9.3` extend it to cover all three modes.

## 10. Importer integration-point gate

Per design §5, importers ship as **stub crates with contract tests only** — live polling is deferred to when the vendor APIs open. The gate confirms the contract is locked:

| ID | Gate | Verification command |
|----|------|----------------------|
| `A10.1` | Anthropic importer contract test green | `cargo test -p spendguard-importer-anthropic import_record_to_audit_row_sets_subscription_meter import_record_to_audit_row_sets_import_source_anthropic import_record_schema_matches_pg_check_constraint` exits 0 |
| `A10.2` | OpenAI importer contract test green | `cargo test -p spendguard-importer-openai import_record_to_audit_row_sets_subscription_meter import_record_to_audit_row_sets_import_source_openai` exits 0 |
| `A10.3` | Both importers default-feature build is HTTP-free | `cargo tree -p spendguard-importer-anthropic -p spendguard-importer-openai -e=normal \| grep -vE '(reqwest\|hyper-tls\|tokio-tungstenite)'` (returns no matches for live HTTP deps) |
| `A10.4` | Contract documented in README | `grep -qE 'spendguard-importer-anthropic.*stub' services/importer_anthropic/README.md && grep -qE 'spendguard-importer-openai.*stub' services/importer_openai/README.md` |

## 11. Documentation gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A11.1` | Claude Code Pro integration doc page mentions all three modes | `grep -qE 'meter\|soft_cap\|hard_cap' docs/site-v2/src/content/docs/integrations/subscription-meter-claude-code-pro.md` (all three matched) |
| `A11.2` | Doc explicitly says "meter, not enforcement" | `grep -qE 'meter.*not.*enforce\|cannot.*enforce' docs/site-v2/src/content/docs/integrations/subscription-meter-claude-code-pro.md` |
| `A11.3` | Doc warns hard-cap shows synthetic 429 = degraded UX | `grep -qE '429\|degraded\|appear.*broken' docs/site-v2/src/content/docs/integrations/subscription-meter-claude-code-pro.md` |
| `A11.4` | Doc cross-links to D02 (CA install prerequisite) | `grep -qE 'D02\|closed-cli-install' docs/site-v2/src/content/docs/integrations/subscription-meter-claude-code-pro.md` |
| `A11.5` | Codex doc page parallel content | Same four checks against `…/subscription-meter-codex-chatgpt.md` |

## 12. Anti-regression gate

| ID | Gate | Verification command |
|----|------|----------------------|
| `A12.1` | Existing BYOK Anthropic integration test still green | `cargo test -p spendguard-egress-proxy routes_anthropic_messages` exits 0 |
| `A12.2` | Existing pricing-table integration test still green | `cargo test -p spendguard-egress-proxy estimate_call_cost_falls_back_to_strategy_a_when_predictor_absent` exits 0 |
| `A12.3` | Existing sidecar ledger-write integration test still green | `cargo test -p spendguard-sidecar reserve_v2_commit_estimated_writes_ledger_entries` exits 0 |
| `A12.4` | No existing `audit_outbox` row's behaviour changes | Pre-/post-D13 query: `SELECT count(*) FROM audit_outbox WHERE reservation_source = 'byok'` is monotonically non-decreasing across the migration. |

`A12.x` collectively ensures D13 is purely additive — no BYOK regression.
