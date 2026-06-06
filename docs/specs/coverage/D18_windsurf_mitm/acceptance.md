# D18 — Acceptance Gates

Per build plan §3, every gate listed here must be **100% feasible** at slice-spec time: runnable in the current repo state, no third-party action required, reproducible by the `superpowers:code-reviewer` skill, no live capture against `server.codeium.com` ever required.

## 1. Repository-state gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A1.1` | `services/windsurf_codec/Cargo.toml` exists with `name = "spendguard-windsurf-codec"` and `publish = false` | `grep -qE 'name = "spendguard-windsurf-codec"' services/windsurf_codec/Cargo.toml && grep -qE 'publish = false' services/windsurf_codec/Cargo.toml` |
| `A1.2` | `services/windsurf_codec/src/lib.rs` exposes `decode_request_frame` and `decode_response_frame` | `grep -qE 'pub fn decode_request_frame' services/windsurf_codec/src/lib.rs && grep -qE 'pub fn decode_response_frame' services/windsurf_codec/src/lib.rs` |
| `A1.3` | `services/windsurf_codec/src/wire.rs` exists with a local minimal proto descriptor (no dep on Codeium-owned `.proto`) | `test -f services/windsurf_codec/src/wire.rs && grep -qE 'CascadeRequestPb' services/windsurf_codec/src/wire.rs && ! grep -rE 'extern.*codeium' services/windsurf_codec/` |
| `A1.4` | `services/windsurf_codec/src/version.rs` declares `KNOWN_WIRE_VERSIONS` const | `grep -qE 'pub const KNOWN_WIRE_VERSIONS' services/windsurf_codec/src/version.rs` |
| `A1.5` | `services/windsurf_codec/src/passthrough.rs` declares byte-perfect tee | `grep -qE 'pub struct DecoderTap' services/windsurf_codec/src/passthrough.rs` |
| `A1.6` | `services/egress_proxy/src/experimental.rs` exists with two-channel `windsurf_codec_enabled` | `grep -qE 'pub fn windsurf_codec_enabled' services/egress_proxy/src/experimental.rs` |
| `A1.7` | `routing.rs` contains a `server.codeium.com` row | `grep -qE 'server\.codeium\.com' services/egress_proxy/src/routing.rs` |
| `A1.8` | `routing.rs` contains a `windsurf-server.codeium.com` row | `grep -qE 'windsurf-server\.codeium\.com' services/egress_proxy/src/routing.rs` |
| `A1.9` | Both Codeium routes carry `experimental: true` | `grep -B1 -A8 'codeium\.com' services/egress_proxy/src/routing.rs \| grep -qE 'experimental: true'` (per row) |
| `A1.10` | `proto/spendguard/common/v1/common.proto` declares `PROVIDER_KIND_WINDSURF_CASCADE` | `grep -qE 'PROVIDER_KIND_WINDSURF_CASCADE' proto/spendguard/common/v1/common.proto` |
| `A1.11` | Migration `0048_audit_outbox_experimental_codec.sql` exists | `test -f services/canonical_ingest/migrations/0048_audit_outbox_experimental_codec.sql` |
| `A1.12` | All 6 fixture files present | `for f in cascade_chat_simple cascade_chat_with_tools cascade_chat_streaming cascade_chat_error cascade_chat_unknown_wire_version cascade_chat_truncated; do test -f "services/windsurf_codec/tests/fixtures/$f.windsurf-frames" \|\| exit 1; done` |
| `A1.13` | `services/windsurf_codec/tests/fixtures/PROVENANCE.md` exists with per-fixture entries | `grep -qE 'capture date' services/windsurf_codec/tests/fixtures/PROVENANCE.md && grep -qE 'redact_windsurf_frames\.py.*sha256' services/windsurf_codec/tests/fixtures/PROVENANCE.md` |
| `A1.14` | `services/windsurf_codec/tests/fixtures/FORMAT.md` documents the `.windsurf-frames` sidecar header | `grep -qE 'timestamp_micros' services/windsurf_codec/tests/fixtures/FORMAT.md` |
| `A1.15` | `scripts/redact_windsurf_frames.py` exists | `test -f scripts/redact_windsurf_frames.py` |
| `A1.16` | `docs/customer/sow-windsurf-mitm.md` exists | `test -f docs/customer/sow-windsurf-mitm.md` |
| `A1.17` | `README.md` `## Adapter integrations` table includes Windsurf row with `experimental — SOW only` badge | `grep -qE 'Windsurf.*experimental — SOW only' README.md` |
| `A1.18` | README row anchors to the SOW doc, NOT to `docs/site-v2/` | `grep -E 'Windsurf' README.md \| grep -qE 'docs/customer/sow-windsurf-mitm\.md' && ! grep -E 'Windsurf' README.md \| grep -qE 'docs/site-v2'` |
| `A1.19` | Windsurf is NOT in the public Starlight docs nav | `! grep -rE 'windsurf' docs/site-v2/astro.config.* docs/site-v2/src/content/docs/index.md` |

## 2. Build gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A2.1` | Workspace builds | `cargo build --workspace --locked` exits 0 |
| `A2.2` | Codec crate builds | `cargo build -p spendguard-windsurf-codec --release --locked` exits 0 |
| `A2.3` | Egress proxy builds with Codeium routing + experimental gate | `cargo build -p spendguard-egress-proxy --release --locked` exits 0 |
| `A2.4` | Sidecar builds (additive proto change only) | `cargo build -p spendguard-sidecar --release --locked` exits 0 |
| `A2.5` | Codec crate has zero HTTP-client deps (parser-only) | `cargo tree -p spendguard-windsurf-codec -e=normal \| grep -vE '(reqwest\|hyper-tls\|tokio-tungstenite)'` (no matches for HTTP-client deps) |
| `A2.6` | No new warnings | `cargo build --workspace -- -D warnings` exits 0 |
| `A2.7` | Clippy clean for new modules | `cargo clippy -p spendguard-windsurf-codec -p spendguard-egress-proxy --all-targets -- -D warnings` exits 0 |
| `A2.8` | `cargo deny check` passes | `cargo deny check` exits 0 |
| `A2.9` | Proto codegen runs clean | `cargo build -p spendguard-common --locked` exits 0 and `PROVIDER_KIND_WINDSURF_CASCADE` enum value present in generated code |

## 3. Unit-test gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A3.1` | All wire/framing unit tests green | `cargo test -p spendguard-windsurf-codec wire::tests` exits 0 |
| `A3.2` | All version-registry unit tests green | `cargo test -p spendguard-windsurf-codec version::tests` exits 0 |
| `A3.3` | All top-level decode entry tests green | `cargo test -p spendguard-windsurf-codec lib::tests` exits 0 |
| `A3.4` | Passthrough tee tests green | `cargo test -p spendguard-windsurf-codec passthrough::tests` exits 0 |
| `A3.5` | `decoder_never_logs_message_content` green | `cargo test -p spendguard-windsurf-codec decoder_never_logs_message_content` exits 0 |
| `A3.6` | Experimental-gate unit tests green | `cargo test -p spendguard-egress-proxy experimental::tests` exits 0 |
| `A3.7` | Routing addition tests green | `cargo test -p spendguard-egress-proxy --lib routing::tests::routes_server_codeium routes_windsurf_server_codeium windsurf_routes_marked_experimental` exits 0 |

## 4. Fixture-driven integration-test gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A4.1` | All 6 `.windsurf-frames` fixture decoders green | `cargo test -p spendguard-windsurf-codec --test decode_request --test decode_response` exits 0 |
| `A4.2` | Unsupported-wire-version fail-closed test green | `cargo test -p spendguard-windsurf-codec --test unsupported_wire_version` exits 0 |
| `A4.3` | Byte-perfect passthrough equivalence test green | `cargo test -p spendguard-windsurf-codec --test passthrough_byte_equivalence` exits 0 |
| `A4.4` | Egress-proxy E2E integration tests green | `cargo test -p spendguard-egress-proxy --test windsurf_mitm_e2e` exits 0 |
| `A4.5` | Meta-test `provenance_md_lists_every_fixture` green | `cargo test -p spendguard-windsurf-codec provenance_md_lists_every_fixture` exits 0 |
| `A4.6` | Meta-test `known_wire_versions_const_matches_fixtures` green | `cargo test -p spendguard-windsurf-codec known_wire_versions_const_matches_fixtures` exits 0 |
| `A4.7` | Redaction script SHA matches `PROVENANCE.md` | `cargo test -p spendguard-windsurf-codec provenance_md_redaction_sha_matches_script` exits 0 |
| `A4.8` | Redaction script pytest green | `pytest scripts/test_redact_windsurf_frames.py` exits 0 |

## 5. Schema migration gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A5.1` | Migration 0048 applies cleanly to a fresh PG 16 instance | `make -C deploy/demo demo-up && psql "$DATABASE_URL" -c "SELECT column_name FROM information_schema.columns WHERE table_name = 'audit_outbox' AND column_name = 'experimental_codec';" \| grep experimental_codec` |
| `A5.2` | Migration idempotent | `psql "$DATABASE_URL" -f services/canonical_ingest/migrations/0048_audit_outbox_experimental_codec.sql` succeeds twice consecutively |
| `A5.3` | CHECK accepts both windsurf + cursor anchors | `psql "$DATABASE_URL" -c "INSERT INTO audit_outbox (..., experimental_codec) VALUES (..., 'windsurf_managed_cascade');"` and `'cursor_byok_managed'` both succeed |
| `A5.4` | CHECK rejects unknown value | `psql "$DATABASE_URL" -c "INSERT … experimental_codec = 'invalid';"` returns SQLSTATE `23514` |
| `A5.5` | Partial index `idx_audit_outbox_experimental_codec` exists with correct predicate | `psql "$DATABASE_URL" -c "SELECT indexdef FROM pg_indexes WHERE indexname = 'idx_audit_outbox_experimental_codec';" \| grep -qE 'experimental_codec IS NOT NULL'` |
| `A5.6` | Migration 0048 listed in `migration_inventory.toml` | `grep -qE '0048_audit_outbox_experimental_codec' services/canonical_ingest/migration_inventory.toml` |

## 6. Demo-mode regression gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A6.1` | `make -C deploy/demo demo-verify-windsurf-mitm-fixture` exits 0 | Replay `cascade_chat_simple` → asserts codec audit row written + ledger entry committed |
| `A6.2` | `make -C deploy/demo demo-verify-windsurf-mitm-fixture FIXTURE=cascade_chat_streaming` exits 0 | Streaming fixture → commit on final-frame usage |
| `A6.3` | `make -C deploy/demo demo-verify-windsurf-mitm-fixture FIXTURE=cascade_chat_error` exits 0 | Error fixture → release fires, no ledger commit |
| `A6.4` | `make -C deploy/demo demo-verify-windsurf-mitm-fixture FIXTURE=cascade_chat_unknown_wire_version` exits NON-zero with clean message | Codec blocks fail-closed; expected non-zero with reason logged |
| `A6.5` | Verifier SQL committed | `test -f deploy/demo/verify_step_windsurf_mitm.sql` |
| `A6.6` | Pre-existing BYOK demo regression: `make -C deploy/demo demo-verify-litellm-real` still exits 0 | D18 is strictly additive |
| `A6.7` | Pricing-table demo regression: `make -C deploy/demo demo-verify-pricing` still exits 0 | Pricing snapshot loading unbroken |
| `A6.8` | `make demo-clean` removes D18-specific artifacts | After clean, audit rows with `experimental_codec` purged |

## 7. Experimental-posture gates (locked from design §3)

| ID | Gate | Verification command |
|----|------|----------------------|
| `A7.1` | Two-channel opt-in enforced: env alone is insufficient | `cargo test -p spendguard-egress-proxy windsurf_codec_disabled_by_config_returns_503` exits 0 |
| `A7.2` | Two-channel opt-in enforced: config alone is insufficient | `cargo test -p spendguard-egress-proxy windsurf_codec_disabled_by_env_returns_503` exits 0 |
| `A7.3` | Boot warning emitted exactly once on enable | `cargo test -p spendguard-egress-proxy windsurf_codec_boot_warning_emitted_exactly_once` exits 0 |
| `A7.4` | Boot warning includes structured fields: `kind`, `codec`, `vendor_protocol`, `support_tier`, `last_verified_capture` | `cargo test -p spendguard-egress-proxy windsurf_codec_boot_warning_includes_last_verified_capture` exits 0 |
| `A7.5` | Unknown wire-version fails closed (does NOT silently best-effort) | `cargo test -p spendguard-egress-proxy windsurf_codec_unknown_wire_version_blocks_request` exits 0 |
| `A7.6` | Decoder failure does NOT block request (degrades to pass-through) | `cargo test -p spendguard-egress-proxy windsurf_codec_decode_failure_falls_through_to_passthrough` exits 0 |
| `A7.7` | Byte-perfect pass-through preserved end-to-end | `cargo test -p spendguard-egress-proxy windsurf_codec_byte_perfect_passthrough` exits 0 |
| `A7.8` | No live capture in CI: grep proves it | `! grep -rE 'reqwest::Client.*server\.codeium\.com\|server\.codeium\.com.*reqwest' services/windsurf_codec/ services/egress_proxy/tests/` returns 0 matches |

## 8. SOW / docs gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A8.1` | SOW doc explicitly says "experimental" + "may break without notice" | `grep -qE 'experimental' docs/customer/sow-windsurf-mitm.md && grep -qE 'may break without notice\|without notice' docs/customer/sow-windsurf-mitm.md` |
| `A8.2` | SOW doc explicitly says "no SLA" | `grep -qE 'no SLA\|no service-level' docs/customer/sow-windsurf-mitm.md` |
| `A8.3` | SOW doc cross-links D02 root CA install | `grep -qE 'D02\|root CA install\|closed-cli-install' docs/customer/sow-windsurf-mitm.md` |
| `A8.4` | SOW doc has customer + SpendGuard signature blocks | `grep -cE '_{20,}' docs/customer/sow-windsurf-mitm.md` returns >= 2 |
| `A8.5` | SOW doc states no Helm default | `grep -qE 'no Helm default\|not a Helm default' docs/customer/sow-windsurf-mitm.md` |
| `A8.6` | SOW doc states codec break ≠ SpendGuard bug | `grep -qE 'codec break\|not a SpendGuard bug' docs/customer/sow-windsurf-mitm.md` |
| `A8.7` | README badge text is exactly `experimental — SOW only` | `grep -qE 'Windsurf.*experimental — SOW only' README.md` (em-dash exact) |
| `A8.8` | README does NOT link Windsurf to public Starlight docs | `grep -E 'Windsurf' README.md \| grep -qE 'docs/customer/' && ! grep -E 'Windsurf' README.md \| grep -qE 'docs/site-v2/'` |
| `A8.9` | `docs/customer/` directory is NOT in Starlight sidebar config | `! grep -rE 'docs/customer' docs/site-v2/astro.config.*` |

## 9. Security / fixture-redaction gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A9.1` | No fixture contains real OAuth tokens (sentinel-only) | `! grep -rE 'Bearer [A-Za-z0-9_-]{20,}' services/windsurf_codec/tests/fixtures/*.windsurf-frames \| grep -v FAKE_` returns 0 |
| `A9.2` | `PROVENANCE.md` pins redaction-script SHA-256 | `grep -qE 'redact_windsurf_frames\.py.*sha256:[0-9a-f]{64}' services/windsurf_codec/tests/fixtures/PROVENANCE.md` |
| `A9.3` | Decoder never logs `messages[*].content` | `cargo test -p spendguard-windsurf-codec decoder_never_logs_message_content` exits 0 |
| `A9.4` | Egress proxy never logs message content | `cargo test -p spendguard-egress-proxy windsurf_codec_does_not_log_message_content` exits 0 |
| `A9.5` | Fixture size cap enforced (4 MiB) | `cargo test -p spendguard-windsurf-codec decoder_handles_giant_frame` exits 0 |
| `A9.6` | Random-bytes fuzz harness does not panic | `cargo test -p spendguard-windsurf-codec --release --features fuzz -- --ignored codec_does_not_panic_on_random_bytes` exits 0 (operator-only, not merge gate) |

## 10. Performance gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A10.1` | `decode_request_frame` p99 < 2 ms | `cargo test -p spendguard-windsurf-codec --release -- --ignored decode_request_p99_under_2ms` exits 0 |
| `A10.2` | `decode_response_frame` chunk p99 < 500 µs | `cargo test -p spendguard-windsurf-codec --release -- --ignored decode_response_chunk_p99_under_500us` exits 0 |
| `A10.3` | Passthrough tee zero-extra-allocation | `cargo test -p spendguard-windsurf-codec --release -- --ignored passthrough_tee_zero_extra_allocation` exits 0 |
| `A10.4` | Boot warning emit p99 < 100 µs | `cargo test -p spendguard-egress-proxy --release -- --ignored boot_warning_emit_under_100us` exits 0 |

## 11. Acceptance scenario gate (primary headline gate)

**The headline acceptance scenario** (from the deliverable prompt + sibling D17 shape):

> Against recorded `.windsurf-frames` fixtures (replay, not live), the proxy:
> (a) decodes Cascade request frames,
> (b) reserves against budget pre-dispatch,
> (c) commits on first usage frame from upstream,
> (d) releases on error / unknown wire version,
> (e) is gated off unless BOTH `SPENDGUARD_EXPERIMENTAL_CODECS=1` AND `spendguard.toml` enable it,
> (f) emits an `experimental_codec_enabled` stderr warning on every boot,
> (g) is documented as SOW-only with a customer SOW addendum template, no public docs surface, and an `experimental — SOW only` README badge.

This is verified by `A11.1` — `A11.7` running end-to-end against the demo stack:

| ID | Gate | Verification command |
|----|------|----------------------|
| `A11.1` | (a)+(b)+(c): fixture replay produces decoded request + ledger commit | `make -C deploy/demo demo-verify-windsurf-mitm-fixture` exits 0; verifier SQL asserts `experimental_codec = 'windsurf_managed_cascade'` audit row + ledger entry |
| `A11.2` | (d) error fixture releases reservation | `make -C deploy/demo demo-verify-windsurf-mitm-fixture FIXTURE=cascade_chat_error` exits 0 |
| `A11.3` | (d) unknown wire version blocks | `make -C deploy/demo demo-verify-windsurf-mitm-fixture FIXTURE=cascade_chat_unknown_wire_version` exits non-zero with `windsurf_wire_version_unsupported` reason logged |
| `A11.4` | (e) two-channel opt-in enforced | `A7.1` + `A7.2` |
| `A11.5` | (f) boot warning emitted with full structured fields | `A7.3` + `A7.4` |
| `A11.6` | (g) SOW doc + README badge + no public docs surface | `A8.1` — `A8.9` collectively |
| `A11.7` | Headline gate is merge-blocking | The slice plan PR cannot merge unless `A11.1` — `A11.6` all green |

## 12. Anti-regression gate

| ID | Gate | Verification command |
|----|------|----------------------|
| `A12.1` | Existing BYOK Anthropic integration test still green | `cargo test -p spendguard-egress-proxy routes_anthropic_messages` exits 0 |
| `A12.2` | Existing OpenAI BYOK integration test still green | `cargo test -p spendguard-egress-proxy routes_openai_chat_completions` exits 0 |
| `A12.3` | Existing sidecar ledger-write integration test still green | `cargo test -p spendguard-sidecar reserve_v2_commit_estimated_writes_ledger_entries` exits 0 |
| `A12.4` | Existing D13 subscription-meter tests still green (if D13 already merged) | `cargo test -p spendguard-egress-proxy --test subscription_meter_e2e` exits 0 |
| `A12.5` | Pre-existing audit_outbox row schema unchanged for non-experimental rows | Pre-/post-D18 query: existing rows have `experimental_codec IS NULL` after migration 0048 |

`A12.x` collectively ensures D18 is purely additive — no BYOK regression, no subscription-meter regression, no audit-row schema breakage.
