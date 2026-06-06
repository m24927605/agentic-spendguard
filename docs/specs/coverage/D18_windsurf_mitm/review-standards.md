# D18 — Review Standards

Slice-specific checklist for the `superpowers:code-reviewer` skill across `COV_75` … `COV_82`. Each slice review consults this file plus [`acceptance.md`](acceptance.md) plus the repo-wide coding standards. The R5 panel summarizer is locked to **Security Engineer** (design §7 locked decision #8): MITM + undocumented vendor wire + IDE-resident root CA make security framing dominant over architecture framing for arbitration weighting.

## 1. Experimental-posture assertions (locked, shared with D17)

D18 + D17 share the experimental posture. Any diff touching the codec module, the experimental gate, the routing rows, the SOW doc, or the README badge MUST satisfy every assertion. Reviewer flags as **Blocker** on any fail.

| ID | Assertion |
|----|-----------|
| `E1` | Two-channel opt-in: `experimental::windsurf_codec_enabled` MUST require BOTH `SPENDGUARD_EXPERIMENTAL_CODECS=1` AND `spendguard.toml`'s `[experimental.windsurf_codec] enabled = true`. Either alone returns `false`. Reviewer reads the gate function and confirms `&&`, not `\|\|`. |
| `E2` | Boot stderr warning fires on every proxy start with the codec enabled, exactly once. Structured fields `kind="experimental_codec_enabled"`, `codec="windsurf_managed_cascade"`, `vendor_protocol="undocumented"`, `support_tier="sow_only"`, `last_verified_capture=<ISO-8601>` are all present. Reviewer greps the diff for the literal `tracing::warn!` block and confirms field set. |
| `E3` | `last_verified_capture` is read from `PROVENANCE.md` at boot, not hard-coded. Reviewer confirms a file read, rejects a string literal. |
| `E4` | Routing rows for `server.codeium.com` AND `windsurf-server.codeium.com` BOTH carry `experimental: true`. Reviewer greps the routing table; any row missing the flag is a Blocker. |
| `E5` | Codeium routes are inert when the codec is gated off: requests return a synthetic 503 with `error.code = "experimental_codec_disabled"`. Upstream is **not** reached. Reviewer reads the forward branch and confirms early-return before any upstream IO. |
| `E6` | Unknown wire version fails closed: `WindsurfCodecError::UnsupportedWireVersion` → proxy emits `STOP_RUN_PROJECTION` with `reason_code = "windsurf_wire_version_unsupported"`. NEVER a silent best-effort decode. |
| `E7` | Decoder failure on a known wire version degrades to byte-perfect pass-through (no reservation, no commit) and emits `decoder_skipped` audit. The request is NOT blocked. This is the SOW-stated fallback contract — reviewer rejects diffs that block on decode failure. |
| `E8` | README badge text matches exactly `experimental — SOW only` (em-dash, not hyphen). No softer wording. Reviewer greps for the literal string. |
| `E9` | README badge anchors to `docs/customer/sow-windsurf-mitm.md`, NOT to `docs/site-v2/`. |
| `E10` | `docs/customer/sow-windsurf-mitm.md` is NOT linked from `docs/site-v2/astro.config.*` or any Starlight sidebar. Reviewer greps the docs-site config. |
| `E11` | SOW doc explicitly states (a) experimental, (b) vendor protocol may change without notice, (c) no SLA, (d) no Helm default, (e) codec break is not a SpendGuard bug, (f) customer authorizes D02 root CA install. All six clauses present. |
| `E12` | No Helm chart change in this deliverable. Reviewer greps the diff for `deploy/helm/` — any change is a Blocker for D18 (must be a separate, explicitly-non-default chart). |
| `E13` | No public docs page (`docs/site-v2/src/content/docs/integrations/windsurf*.md`) is created. The SOW doc at `docs/customer/` is the ONLY customer-facing surface. |

## 2. Threat-model assertions

D18 installs a root CA on customer IDE hosts (via D02), MITMs an undocumented vendor wire, and runs a hand-written protobuf descriptor. The threat surface is wide. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `T1` | Codec NEVER logs decoded `messages[*].content`, `tool_declarations[*].schema`, or any field that could carry customer prompt content. Reviewer greps for `tracing::*!()` calls near `CascadeMessage`, `CascadeRequest`, `CascadeResponseDelta` and reads each match. Enforced by test `decoder_never_logs_message_content`. |
| `T2` | No `Authorization` header value passes through the codec to logs. The codec does NOT inspect HTTP headers — it parses gRPC-Web payloads only. Reviewer confirms by grep: no `http::HeaderMap` import in `services/windsurf_codec/src/*.rs`. |
| `T3` | Protobuf descriptor is hand-written, local-only. No `build.rs` that imports a Codeium-owned `.proto`. No `include_proto!` macro referencing external schemas. Reviewer reads `Cargo.toml` and `build.rs` (if present) and confirms. |
| `T4` | Frame size is bounded: any inbound `.windsurf-frames` payload > 4 MiB is rejected with `Err(TruncatedBody{..})` or a size-cap error. Reviewer confirms a cap constant + test `decoder_handles_giant_frame`. |
| `T5` | Protobuf recursion depth is bounded (default `prost` limit, or explicit). Reviewer confirms the test `decoder_protobuf_recursion_depth_bounded` is present. |
| `T6` | Fuzz harness (`#[ignore] #[cfg(feature = "fuzz")]`) covers random-bytes input. Reviewer confirms the test is present, but it is NOT a merge gate (operator-only). |
| `T7` | Decoder is stateless and pure: `fn decode_request_frame(buf: &Bytes) -> Result<...>`. No global mutable state, no IO. Reviewer confirms by signature read. |
| `T8` | Pass-through tee MUST NOT modify the upstream byte stream. `DecoderTap::observe` returns the input slice unchanged (same `Bytes` clone, no copy or mutation). Reviewer reads the function and confirms `chunk` flows out unchanged. Asserted by `windsurf_codec_byte_perfect_passthrough`. |
| `T9` | Fixture files contain only redacted sentinels. Reviewer greps every committed `.windsurf-frames` for token-like patterns (`Bearer [A-Za-z0-9_-]{20,}`); any non-`FAKE_`-prefixed match is a Blocker. |
| `T10` | `PROVENANCE.md` pins SHA-256 of `redact_windsurf_frames.py`. Reviewer recomputes the hash and confirms equality. |
| `T11` | The `experimental_codec` audit column never carries free-form customer text. CHECK constraint enumerates exactly `('windsurf_managed_cascade', 'cursor_byok_managed')`. Reviewer reads migration 0048 and confirms. |
| `T12` | `SPENDGUARD_WINDSURF_PREAMBLE_HASHES` env var (for hash-pinned wire versions) is parsed defensively — invalid hex is silently skipped, never panics. Reviewer reads `version::env_pinned_hashes` and confirms `filter_map(|s| hex::decode(...).ok())`. |
| `T13` | The 503 returned when the codec is gated off MUST NOT leak which env var or config field is missing. Body shape is fixed: `{"error":{"code":"experimental_codec_disabled","codec":"windsurf_managed_cascade"}}`. No detail beyond this. |
| `T14` | No client cert / private key material is bundled with the codec crate. Reviewer greps `services/windsurf_codec/` for `.pem`, `.key`, `.crt`, `.p12` — no matches. |

## 3. Cross-tier correctness assertions

`COV_77` (routing + gate), `COV_78` (reserve/commit wiring), `COV_79` (passthrough), and `COV_80` (fixtures) span the codec + proxy + sidecar + canonical-ingest tiers. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `X1` | `ProviderKind::WINDSURF_CASCADE` proto value is additive: tag = 11 (next free), default behaviour for legacy clients unchanged. |
| `X2` | Migration 0048 is additive: existing `audit_outbox` rows have `experimental_codec IS NULL` after apply — no UPDATE pass over old rows. |
| `X3` | CHECK constraint on `experimental_codec` lists BOTH `'windsurf_managed_cascade'` AND `'cursor_byok_managed'` (cross-D17 anchor). Reviewer confirms both values to avoid a follow-up migration when D17 lands. |
| `X4` | Sidecar `RequestDecision` is reused unchanged; the codec produces a `ClaimEstimate` with `provider_kind = WINDSURF_CASCADE` and the existing sidecar code path handles it. No new sidecar method introduced. |
| `X5` | `CommitEstimated` fires on the FIRST response frame that carries a populated `usage.input_tokens > 0`, not on stream end. Mid-stream commit is required because Cascade streams may run > 60s. Reviewer reads the commit trigger and confirms. |
| `X6` | `ReleaseReservation` fires on (a) `finish_reason = "error"`, (b) `UnsupportedWireVersion`, (c) HTTP/2 RST_STREAM before any usage frame, (d) proxy timeout. Reviewer enumerates each branch in `forward.rs` and confirms exactly one release path per condition. |
| `X7` | Tokenizer kind for Codeium routes is `Openai` (BPE). This is a stated assumption per design §7 locked decision #4. Reviewer confirms code comment cites the assumption + cross-links the SOW doc. |
| `X8` | The Codex / Cursor / Windsurf experimental-codec config block lives under a single `[experimental]` namespace in `spendguard.toml`. Adding a new experimental codec (D17 Cursor) MUST NOT require restructuring the namespace. Reviewer reads the deserializer and confirms additive sub-key pattern. |
| `X9` | Routing table textual order: Codeium rows appended AFTER existing Anthropic / OpenAI rows. Regex is restrictive enough that order doesn't matter for matching, but textual ordering eases code review. |
| `X10` | `audit_outbox.experimental_codec` is updated in every downstream SELECT projection (analytics, control plane forwarder, dashboard query) — OR the column is explicitly excluded with a code comment justifying why. Reviewer greps `SELECT … FROM audit_outbox` across the repo. |

## 4. Codec correctness matrix (`COV_75` + `COV_76`)

| ID | Assertion |
|----|-----------|
| `M1` | `KNOWN_WIRE_VERSIONS` const is the SINGLE source of truth. Any decoder branch that handles a version not in the const is a Blocker. Reviewer greps for `match wire_version` patterns and confirms exhaustiveness against the const. |
| `M2` | Adding a new wire version requires (a) appending to `KNOWN_WIRE_VERSIONS`, (b) adding a matching fixture in `tests/fixtures/`, (c) updating `PROVENANCE.md`. Meta-test `known_wire_versions_const_matches_fixtures` enforces (a)+(b); reviewer manually confirms (c). |
| `M3` | Local proto descriptor (`wire.rs`) declares ONLY the fields D18 reads. Unknown proto fields are tolerated (prost default behaviour) — reviewer confirms no `#[prost(unknown_fields)]` deny attribute. |
| `M4` | `strip_grpc_web_prefix` returns `Err(MissingField("gzip_unsupported"))` when the compression flag is set. NO silent gzip decode path. Reviewer reads the function and confirms. |
| `M5` | `detect_version` tries explicit `cascade_wire_version` first, falls back to preamble SHA-256 only if explicit field missing. Reviewer reads the function and confirms ordering. |
| `M6` | Preamble SHA-256 is computed over the FIRST `min(64, body.len())` bytes, not the whole body. Reviewer reads the hash construction and confirms. |
| `M7` | `parse_response` is implemented in `COV_75` (not `todo!()`). Reviewer rejects any merge with a `todo!()` in the response-decode path. |

## 5. Reserve / commit / release fork assertions (`COV_78`)

| ID | Assertion |
|----|-----------|
| `R1` | The codec branch in `forward.rs` is a NEW match arm `ProviderKind::WindsurfCascade =>`, NOT a modification to an existing BYOK or OpenAI arm. Reviewer reads the diff and confirms the BYOK arms are byte-identical to pre-D18. |
| `R2` | For `WindsurfCascade`, the experimental gate runs FIRST. If gated off, return 503 immediately, no further code paths execute. Reviewer confirms the early-return is the literal first line of the arm. |
| `R3` | The decoder side-task is `tokio::spawn`'d with a bounded `mpsc::channel(64)`. Reviewer confirms the bound and that the spawn handle is `await`'d (not detached). |
| `R4` | If the decoder side-task returns `Err`, the forward path still produces a valid HTTP response to the IDE (pass-through fallback). The `match decoder_handle.await?` arm for `Err` MUST emit `audit::emit_decoder_skipped` AND return `Ok(upstream_resp)`. Reviewer reads the arm and confirms. |
| `R5` | `tenant_id` for the codec audit row comes from the existing proxy-side resolution (the inbound mTLS peer or header-derived identity), NOT from any decoded Cascade field. Reviewer confirms by reading the audit emit and tracking `tenant_id` provenance. |
| `R6` | `pricing_version` written into the codec audit row matches what BYOK writes for the same call — both read from the same `PricingSnapshot`. |
| `R7` | Commit fires EXACTLY ONCE per request, on the first usage-bearing frame. Multiple usage frames (vendor sends incremental tallies) MUST NOT trigger multiple commits. Reviewer reads the commit-trigger guard and confirms a `commit_fired: bool` latch. |

## 6. Passthrough byte-equivalence assertions (`COV_79`)

| ID | Assertion |
|----|-----------|
| `P1` | The tee path uses `Bytes::clone` (refcount bump, no copy), NOT `Vec::clone` or `Bytes::copy_from_slice`. Reviewer reads the tap implementation. |
| `P2` | When the decoder's `mpsc` channel is full (`try_send` fails), the WIRE bytes are still forwarded — the tap drops the observation, never the upstream byte. Reviewer reads the drop path. |
| `P3` | The integration test `windsurf_codec_byte_perfect_passthrough` SHA-256s both input and upstream-received bytes and asserts equality. Reviewer reads the test and confirms the assertion. |
| `P4` | No `String::from_utf8` or text-encoding-conversion call in the forward path. The codec parses protobuf binary only; the proxy never decodes the wire as text. |

## 7. Fixture provenance assertions (`COV_80`)

| ID | Assertion |
|----|-----------|
| `F1` | Every `.windsurf-frames` file has a matching entry in `PROVENANCE.md`. Meta-test enforces. |
| `F2` | `PROVENANCE.md` lists, per fixture: capture date (ISO-8601), capturing-operator initials, source-tool version, cascade wire version, redaction-script SHA-256, SOW contract ID. |
| `F3` | `tests/fixtures/FORMAT.md` documents the 16-byte sidecar header per frame (timestamp_micros + stream_id + payload_len). Reviewer confirms. |
| `F4` | `scripts/redact_windsurf_frames.py` SHA-256 in `PROVENANCE.md` equals `sha256sum scripts/redact_windsurf_frames.py` at review time. Reviewer recomputes and confirms. |
| `F5` | Redaction script is itself tested (`scripts/test_redact_windsurf_frames.py`) — 6 cases: replace auth, replace content, preserve usage, preserve wire version, idempotent, SHA matches. |
| `F6` | NO live capture script committed. Reviewer greps for any file that imports `reqwest` or `hyper` AND mentions `server.codeium.com` — no matches outside of `#[cfg(feature = "live-capture")]` operator code paths. |

## 8. Demo + SOW + docs assertions (`COV_81` + `COV_82`)

| ID | Assertion |
|----|-----------|
| `D1` | Demo target `demo-verify-windsurf-mitm-fixture` accepts a `FIXTURE=<name>` env override; default is `cascade_chat_simple`. Reviewer reads the Makefile target. |
| `D2` | Demo compose file declares a `windsurf_stub_upstream` service that returns canned response frames. Never reaches `server.codeium.com`. |
| `D3` | Verifier SQL `verify_step_windsurf_mitm.sql` asserts (a) `experimental_codec = 'windsurf_managed_cascade'` audit row present, (b) ledger entry committed (or released, depending on fixture). |
| `D4` | `make demo-clean` purges D18-specific rows (audit rows with `experimental_codec IS NOT NULL` + ledger entries from the demo replay). |
| `D5` | Demo replay harness `replay_windsurf_frames.py` does NOT send `FAKE_*` tokens to `api.openai.com` or any other real vendor — upstream is the stub service in compose. |
| `D6` | SOW doc has signature blocks for BOTH customer AND SpendGuard rep (two underscore-lined signature regions). |
| `D7` | SOW doc includes capture-cadence clause: SpendGuard provides one re-capture per pool of paid SOW hours. |
| `D8` | The SOW doc is at `docs/customer/sow-windsurf-mitm.md`. `docs/customer/` is NOT a Starlight content collection. |

## 9. Migration assertions

| ID | Assertion |
|----|-----------|
| `G1` | Migration number 0048 does not collide with existing 0048+ files. Reviewer runs `ls services/canonical_ingest/migrations/004[789]*.sql` and confirms only D18 owns 0048. (If D17 ships 0047 first, D18 keeps 0048; if D18 ships first, take 0047 instead — reviewer rejects collisions.) |
| `G2` | `migration_inventory.toml` updated with checksum-pinned 0048. |
| `G3` | Down-migration exists if convention requires (matches pattern of existing 004x migrations). |
| `G4` | Partial index `idx_audit_outbox_experimental_codec` has matching predicate `WHERE experimental_codec IS NOT NULL` — typed-narrowed for planner. |

## 10. R1-R5 escalation criteria

| Round | Blocker count | Action |
|-------|--------------|--------|
| R1 | 0 → MERGE | none |
| R1 | ≥ 1 → dispatch same implementer with findings | typical 3-6 findings on first review (D18 surface is the widest in Tier 3: codec + gate + routing + reserve/commit + passthrough + 6 fixtures + SOW + migration) |
| R2-R4 | drop to 0 → MERGE | follow normal cadence |
| R5 | ≥ 1 Blocker → Staff+ panel arbitration | panel composition per build plan §1.3 |

**R5 panel summarizer:** Security Engineer (design §7 locked decision #8). Rationale: every Archetype III deliverable concentrates MITM + undocumented-vendor-wire + IDE-resident root CA risks; security framing dominates.

## 11. Per-slice review focus

| Slice | Focus areas |
|-------|-------------|
| `COV_75_d18_recon_and_wire_descriptor` | §2 (T3, T4, T5, T7), §4 (M1-M7) |
| `COV_76_d18_codec_module_skeleton` | §2 (T1, T2, T8), §4 (M1, M3-M5, M7) |
| `COV_77_d18_routing_and_experimental_gate` | §1 (E1-E5, E13), §2 (T11, T13), §3 (X1, X8, X9) |
| `COV_78_d18_reserve_commit_wiring` | §1 (E6), §3 (X4, X5, X6, X7), §5 (R1-R7) |
| `COV_79_d18_passthrough_and_decoder_skip` | §1 (E7), §2 (T8), §6 (P1-P4) |
| `COV_80_d18_fixture_tests` | §2 (T9, T10), §7 (F1-F6) |
| `COV_81_d18_docs_and_sow` | §1 (E8-E13), §8 (D6-D8) |
| `COV_82_d18_demo_mode` | §3 (X2, X3, X10), §8 (D1-D5), §9 (G1-G4) |

Each slice's review pass only consults its focus areas (plus repo-wide standards); reviewer is NOT asked to re-check the whole list for every slice.

## 12. D17 alignment notes

D17 (Cursor MITM) shares §1 (experimental posture), §2 partial (T1-T8), §3 partial (X3, X8), and §6 (passthrough). When D17 lands first, D18's review re-reads only the codec-specific assertions (§4, §5, §7). When D18 lands first, D17 will inherit §1 verbatim — reviewer should treat any §1 deviation in D17 as a Blocker.
