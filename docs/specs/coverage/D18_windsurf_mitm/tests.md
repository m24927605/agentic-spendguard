# D18 — Tests

Companion to [`design.md`](design.md) and [`implementation.md`](implementation.md). Fixture-driven primary — every gate runs against recorded `.windsurf-frames` files; nothing in CI ever reaches `server.codeium.com`. Live capture is operator-only, gated behind `#[ignore] #[cfg(feature = "live-capture")]`.

## 1. Unit tests — `services/windsurf_codec/`

### 1.1 `wire.rs` — frame framing

| Test | Asserts |
|------|---------|
| `strip_grpc_web_prefix_happy_path` | Valid 5-byte prefix + identity-encoded body → returns `(WireVersion, Bytes)`; body length matches prefix. |
| `strip_grpc_web_prefix_truncated_prefix` | `buf.len() < 5` → `Err(TruncatedPrefix)`. |
| `strip_grpc_web_prefix_truncated_body` | Prefix says 200 bytes, buf has 50 → `Err(TruncatedBody { expected: 205, got: 55 })`. |
| `strip_grpc_web_prefix_gzip_unsupported` | Prefix compression flag set → `Err(MissingField("gzip_unsupported"))`. Regression check: no silent gzip decode. |
| `strip_grpc_web_prefix_zero_length_body` | Prefix says 0 bytes → returns empty `Bytes`, no panic. |
| `parse_request_extracts_model_name` | Fixture `cascade_chat_simple` → `CascadeRequest.model_name == "claude-3-5-sonnet"` (or whatever the fixture pins). |
| `parse_request_extracts_messages_in_order` | Fixture with 3 messages → role+content survive in array order. |
| `parse_request_extracts_tool_decls` | Fixture `cascade_chat_with_tools` → `tool_declarations.len() == 1`, `name == "read_file"`. |
| `parse_request_max_tokens_optional` | Fixture without `max_tokens` → `CascadeRequest.max_tokens.is_none()`. |
| `parse_request_unknown_proto_fields_ignored` | Hand-built frame with proto tag 999 → decoder succeeds, ignores. |

### 1.2 `version.rs` — wire-version registry

| Test | Asserts |
|------|---------|
| `is_known_explicit_v2_0` | `WireVersion::Explicit("cascade.v2.0")` → `true`. |
| `is_known_explicit_v2_1` | `WireVersion::Explicit("cascade.v2.1")` → `true`. |
| `is_known_unknown_version_returns_false` | `WireVersion::Explicit("cascade.v9.9")` → `false`. |
| `is_known_preamble_hash_unregistered_returns_false` | Random SHA-256 not in env → `false`. |
| `is_known_preamble_hash_env_registered_returns_true` | `SPENDGUARD_WINDSURF_PREAMBLE_HASHES=<hex>` set → matching hash returns `true`. |
| `wire_version_display_explicit` | `Display` format: `"explicit:cascade.v2.0"`. |
| `wire_version_display_preamble_hash` | Format: `"preamble_sha256:<64hex>"`. |
| `known_wire_versions_const_matches_fixtures` | Every fixture's pinned `cascade_wire_version` (from `PROVENANCE.md`) is in `KNOWN_WIRE_VERSIONS` — meta-test, fails if a fixture is added without registering its version. |

### 1.3 `lib.rs` — top-level decode entry points

| Test | Asserts |
|------|---------|
| `decode_request_frame_known_version` | Fixture `cascade_chat_simple` → `Ok(CascadeRequest)`. |
| `decode_request_frame_unknown_version_fails_closed` | Fixture `cascade_chat_unknown_wire_version` → `Err(UnsupportedWireVersion(_))`. **No silent best-effort.** |
| `decode_request_frame_truncated_returns_truncated_err` | Fixture `cascade_chat_truncated` → `Err(TruncatedBody{..})` or `Err(TruncatedPrefix)`, NOT a panic. |
| `decode_response_frame_extracts_usage_when_present` | Fixture `cascade_chat_streaming` final frame → `usage.input_tokens > 0 && usage.output_tokens > 0`. |
| `decode_response_frame_chunk_without_usage` | Mid-stream chunk → `usage.is_none()`, `text_chunk.is_some()`. |
| `decode_response_frame_error_finish_reason` | Fixture `cascade_chat_error` → `finish_reason == Some("error")`. |
| `decoder_never_logs_message_content` | Capture `tracing` events during decode of `cascade_chat_simple`; assert no event field contains `messages[*].content` substring. Enforced via `tracing-test` capture. |

### 1.4 `passthrough.rs` — byte-perfect tee

| Test | Asserts |
|------|---------|
| `tap_observe_returns_unchanged_bytes` | `tap.observe(&chunk)` returns the SAME byte slice; no allocation, no mutation. |
| `tap_observe_when_decoder_buffer_full` | Send 1000 chunks rapidly into a `mpsc::channel(64)` → no upstream-bytes loss (forwarded bytes equal to input bytes), but `warn!` emitted with `kind="windsurf_decoder_tap_dropped"`. |
| `tap_observe_decoder_dead_does_not_block` | Drop the receiver mid-stream → `try_send` errs, observe continues to return chunks, no panic, no deadlock. |

## 2. Fixture-driven integration tests

`services/windsurf_codec/tests/decode_request.rs`, `decode_response.rs`, `passthrough_byte_equivalence.rs`, `unsupported_wire_version.rs`.

| Test | Fixture | Asserts |
|------|---------|---------|
| `cascade_chat_simple_decodes_to_expected_request` | `cascade_chat_simple.windsurf-frames` | Request shape matches golden value in test source. |
| `cascade_chat_with_tools_round_trip` | `cascade_chat_with_tools.windsurf-frames` | Both request (tool decl present) AND response (tool-call output frame) decode. |
| `cascade_chat_streaming_yields_usage_on_final_frame` | `cascade_chat_streaming.windsurf-frames` | First N-1 frames yield `text_chunk` only; frame N yields `usage`. |
| `cascade_chat_error_yields_finish_reason_error` | `cascade_chat_error.windsurf-frames` | Final frame `finish_reason == "error"`; no usage; reserve path must call `ReleaseReservation`. |
| `cascade_chat_unknown_wire_version_fails_closed` | `cascade_chat_unknown_wire_version.windsurf-frames` | `Err(UnsupportedWireVersion(_))`; integration test asserts proxy emits `STOP_RUN_PROJECTION` with `reason_code = "windsurf_wire_version_unsupported"`. |
| `cascade_chat_truncated_skips_decoder_does_not_block_proxy` | `cascade_chat_truncated.windsurf-frames` | Decoder returns `Err`; proxy continues to forward upstream bytes byte-perfectly; audit row carries `decoder_skipped` + `reason_code = "windsurf_decode_failed"`. |

### 2.1 Fixture provenance (gates)

`services/windsurf_codec/tests/fixtures/PROVENANCE.md` MUST contain, for each fixture:

- Capture date (ISO-8601)
- Capturing operator initials
- Source tool + version (e.g. `Windsurf 1.10.2`)
- Cascade wire version (explicit string or preamble SHA-256)
- Redaction script: `scripts/redact_windsurf_frames.py` (SHA-256 pinned, verified at test runtime)
- SHA-256 of the **original** request `Authorization` header (audit only — original token never committed)
- Confirmation that no customer prompt content survived redaction
- SOW under which capture occurred (signed contract ID)

A meta-test `provenance_md_lists_every_fixture` greps `tests/fixtures/*.windsurf-frames` and asserts every file has a matching entry in `PROVENANCE.md` — fails the build if a fixture is added without provenance.

### 2.2 Redaction script test

`scripts/redact_windsurf_frames.py` has its own pytest (`scripts/test_redact_windsurf_frames.py`):

| Test | Asserts |
|------|---------|
| `redact_replaces_authorization_header` | Input frame with `Authorization: Bearer foo123` → output has `Authorization: Bearer FAKE_TOKEN_<hex>`. |
| `redact_replaces_message_content` | Input `messages[0].content = "real prompt"` → output `= "<redacted prompt>"`. |
| `redact_preserves_usage_counts` | Input `usage.input_tokens = 1234` → output keeps `1234` (deterministic test math). |
| `redact_preserves_wire_version` | Input `cascade_wire_version = "cascade.v2.1"` → output preserves field. |
| `redact_idempotent` | Redact twice → same output (no double-prefix). |
| `redact_script_sha_matches_provenance` | After running the script, recompute its SHA-256 and confirm equality to the value pinned in `PROVENANCE.md`. |

## 3. Egress proxy integration tests

`services/egress_proxy/tests/windsurf_mitm_e2e.rs`:

| Test | Asserts |
|------|---------|
| `windsurf_codec_disabled_by_env_returns_503` | `SPENDGUARD_EXPERIMENTAL_CODECS` unset + config enabled → proxy returns 503 with body `{"error":{"code":"experimental_codec_disabled","codec":"windsurf_managed_cascade"}}`. |
| `windsurf_codec_disabled_by_config_returns_503` | Env set + config NOT enabled → 503 same shape. |
| `windsurf_codec_enabled_two_channel_decodes_and_reserves` | Both set → fixture replay → audit row written with `experimental_codec = 'windsurf_managed_cascade'`, ledger entry committed. |
| `windsurf_codec_decode_failure_falls_through_to_passthrough` | Replay `cascade_chat_truncated` → audit row `decoder_skipped`, upstream bytes forwarded byte-perfectly (asserted by SHA-256 comparing input bytes to recorded upstream bytes). |
| `windsurf_codec_unknown_wire_version_blocks_request` | Replay `cascade_chat_unknown_wire_version` → proxy returns 503, audit row `decision = STOP_RUN_PROJECTION`, `reason_code = "windsurf_wire_version_unsupported"`, upstream stub records 0 forwarded requests. |
| `windsurf_codec_error_finish_reason_releases_reservation` | Replay `cascade_chat_error` → `ReleaseReservation` called exactly once, ledger commit NOT called. |
| `windsurf_codec_boot_warning_emitted_exactly_once` | Restart proxy with codec enabled → exactly one `WARN` event with `kind="experimental_codec_enabled"`, `codec="windsurf_managed_cascade"`. |
| `windsurf_codec_boot_warning_includes_last_verified_capture` | The emitted `WARN` event's `last_verified_capture` field equals the ISO-8601 date in `PROVENANCE.md`. |
| `windsurf_codec_byte_perfect_passthrough` | Sum SHA-256 of inbound bytes; sum SHA-256 of bytes the upstream stub received; assert equality. Codec MUST NOT mutate the wire. |
| `windsurf_codec_does_not_log_message_content` | Fixture replay produces `tracing` log buffer; assert no log entry contains the (non-redacted) content of any decoded `CascadeMessage`. |

## 4. Routing tests

`services/egress_proxy/src/routing.rs` unit tests:

| Test | Asserts |
|------|---------|
| `routes_server_codeium_com_cascade_chat` | `route("server.codeium.com", "/exa.language_server_pb.LanguageServerService/CascadeChat")` → `ProviderKind::WindsurfCascade`. |
| `routes_windsurf_server_codeium_com_cascade_chat` | Same for `windsurf-server.codeium.com`. |
| `windsurf_routes_marked_experimental` | Both rows carry `experimental: true`. |
| `windsurf_routes_do_not_match_other_codeium_paths` | `/exa.language_server_pb.LanguageServerService/Health` → no match (falls through to default Codeium passthrough or 404). |
| `windsurf_route_tokenizer_kind_is_openai` | Locked decision #4 — reviewer-verifiable. |

## 5. Schema migration tests

| Test | Asserts |
|------|---------|
| `0048_apply_and_rollback_idempotent` | Apply twice → no error. Rollback removes `experimental_codec` column cleanly. |
| `0048_check_constraint_accepts_windsurf` | `INSERT … experimental_codec = 'windsurf_managed_cascade'` succeeds. |
| `0048_check_constraint_accepts_cursor_anchor` | `INSERT … experimental_codec = 'cursor_byok_managed'` succeeds (cross-D17 anchor). |
| `0048_check_constraint_rejects_unknown` | `INSERT … experimental_codec = 'wat'` → SQLSTATE `23514`. |
| `0048_partial_index_exists_with_predicate` | `pg_indexes` confirms `idx_audit_outbox_experimental_codec` with `WHERE experimental_codec IS NOT NULL`. |
| `0048_null_column_for_byok_rows` | Existing BYOK rows have `experimental_codec IS NULL` (default). |

## 6. Demo-mode regression tests

| ID | Command | Asserts |
|----|---------|---------|
| `T6.1` | `make -C deploy/demo demo-verify-windsurf-mitm-fixture` exits 0 | Replay `cascade_chat_simple` → codec audit row + ledger entry. |
| `T6.2` | `make -C deploy/demo demo-verify-windsurf-mitm-fixture FIXTURE=cascade_chat_streaming` exits 0 | Streaming fixture decodes; commit fires on final-frame usage. |
| `T6.3` | `make -C deploy/demo demo-verify-windsurf-mitm-fixture FIXTURE=cascade_chat_error` exits 0 | Release fires; no ledger commit. |
| `T6.4` | `make -C deploy/demo demo-verify-windsurf-mitm-fixture FIXTURE=cascade_chat_unknown_wire_version` exits NON-zero | Codec blocks; expected fail-closed. |
| `T6.5` | `make -C deploy/demo demo-verify-litellm-real` exits 0 (regression) | Pre-existing BYOK demo still passes — D18 strictly additive. |
| `T6.6` | `make -C deploy/demo demo-verify-pricing` exits 0 (regression) | Pricing snapshot still loads. |

## 7. Negative / red-team tests

| Test | Asserts |
|------|---------|
| `decoder_handles_giant_frame` | 16 MiB synthetic frame → decoder allocates bounded, no OOM, returns `Err` if size > configured cap (4 MiB). |
| `decoder_handles_zero_byte_stream` | Empty inbound stream → `Err(TruncatedPrefix)`, no panic, proxy still completes pass-through. |
| `decoder_handles_repeated_unknown_wire_version_does_not_log_payload` | 100 unknown-version frames → audit row count == 100, log buffer size bounded, NO frame payload bytes appear in logs. |
| `decoder_protobuf_recursion_depth_bounded` | Hand-built frame with deeply nested message → decoder enforces depth limit, returns `Err`. |
| `codec_does_not_panic_on_random_bytes` | Fuzz harness: 100k random-byte buffers → no panics, every result is `Ok | Err`. (`#[ignore] #[cfg(feature = "fuzz")]` flag.) |
| `fixture_files_contain_no_real_oauth_tokens` | grep every `.windsurf-frames` file for `Bearer [A-Za-z0-9_\-]{20,}` outside the `FAKE_` prefix → no matches. Same posture as D13. |
| `provenance_md_redaction_sha_matches_script` | Recompute `sha256sum scripts/redact_windsurf_frames.py` → equals value in `PROVENANCE.md`. |

## 8. Documentation tests

| Test | Asserts |
|------|---------|
| `sow_template_present` | `test -f docs/customer/sow-windsurf-mitm.md`. |
| `sow_template_mentions_experimental` | `grep -qE 'experimental\|may break without notice'` matches. |
| `sow_template_mentions_no_sla` | `grep -qE 'no SLA\|no service-level'` matches. |
| `sow_template_links_d02_ca_install` | `grep -qE 'D02\|root CA install'` matches. |
| `sow_template_has_signature_blocks` | `grep -qE '_{20,}'` (underscore signature lines) matches twice (customer + SpendGuard). |
| `readme_windsurf_badge_correct_wording` | `grep -qE 'experimental — SOW only'` matches (exact text, no softer wording). |
| `readme_windsurf_row_does_not_link_public_docs` | `grep` confirms the badge anchors to `docs/customer/sow-windsurf-mitm.md`, NOT to `docs/site-v2/`. |
| `windsurf_codec_not_in_public_docs_nav` | `grep -rE 'windsurf' docs/site-v2/astro.config.* docs/site-v2/src/content/docs/index.md` → returns no nav-link match. |

## 9. Performance gates

| Test | Asserts |
|------|---------|
| `decode_request_p99_under_2ms` | 10k iterations over `cascade_chat_simple` → p99 < 2 ms. |
| `decode_response_chunk_p99_under_500us` | Mid-stream chunk decode p99 < 500 µs. |
| `passthrough_tee_zero_extra_allocation` | Benchmark asserts `observe()` does not allocate beyond the chunk's existing `Bytes` refcount bump. |
| `boot_warning_emit_under_100us` | Boot-time warning emit p99 < 100 µs. |

## 10. Test inventory summary

- Unit tests: ~40 across `wire.rs`, `version.rs`, `lib.rs`, `passthrough.rs`.
- Fixture-driven integration: 6 `.windsurf-frames` fixtures + 6 primary integration tests + 1 meta-provenance test.
- Egress proxy integration: 10 tests in `windsurf_mitm_e2e.rs`.
- Routing: 5 tests.
- Migration 0048: 6 tests.
- Demo regression: 6.
- Negative / red-team: 7.
- Docs / SOW: 8.
- Performance: 4.

**Total ~92 tests.** Zero live capture in CI. Every gate runs in `cargo test --workspace` + `make -C deploy/demo demo-verify-*` + `grep` on docs. Live capture is the `#[ignore] #[cfg(feature = "live-capture")]` operator-only test, not a merge gate.
