# D18 — Windsurf Managed-Cascade MITM Codec (SOW-only, experimental)

**Status:** Spec — Tier 3, build plan §2.3. **Parent:** [`framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md) Archetype III. **Sibling:** [`D17_cursor_mitm`](../D17_cursor_mitm/design.md). **Depends on:** [`D02`](../D02_closed_cli_install/design.md) (root CA + `HTTPS_PROXY`). **Owner:** Backend Architect.

## 1. Problem

Windsurf managed Cascade mode talks to `server.codeium.com` / `windsurf-server.codeium.com` over a proprietary protobuf-over-HTTP/2 wire bundling prompt, tool calls, and streaming output into Cascade frames — not OpenAI / Anthropic endpoints. Community `windsurf-proxy` proves decode is possible; the wire is undocumented and vendor-mutable. D02 / D11 / D12 / D13 do not gate it.

## 2. Goals / non-goals

**In:** MITM via D02's CA; `services/windsurf_codec/` crate decoding Cascade frames; reserve at decode, commit at first usage frame, release on failure; `.windsurf-frames` fixtures; `experimental: true` flag; stderr boot warning; SOW addendum.

**Out:** GA; Helm default; legacy Codeium Chat protocol; Windsurf desktop sign-in; non-Cascade modes (D02 covers BYOK); live capture in CI; codec autoupdate; auto-CA install.

## 3. Experimental posture (locked, shared with D17)

Every Archetype III deliverable inherits this verbatim. Deviation is a Blocker.

1. **Two-channel opt-in.** Routing row `experimental: true`. Codeium routes refused unless `SPENDGUARD_EXPERIMENTAL_CODECS=1` AND `spendguard.toml` `[experimental.windsurf_codec] enabled = true`. Either alone insufficient.
2. **Boot stderr warning.** Exactly one structured `WARN` per boot: `kind="experimental_codec_enabled"`, `codec="windsurf_managed_cascade"`, `vendor_protocol="undocumented"`, `support_tier="sow_only"`, `last_verified_capture` from `PROVENANCE.md`. Test-asserted.
3. **SOW gate doc.** `docs/customer/sow-windsurf-mitm.md` is the only customer-facing surface; NOT linked from `docs/site-v2`. Template requires customer signature acknowledging (a) protocol can change without notice, (b) codec break stops gating until re-capture, (c) no SLA, (d) no Helm default.
4. **README badge.** `## Adapter integrations` row marks "Windsurf (Cascade)" with `experimental — SOW only` linking to the SOW doc.
5. **Demo separation.** `demo-verify-windsurf-mitm-fixture` only; never live. Compose stub returns canned frames.
6. **Wire-version pinning.** Every fixture embeds `cascade_wire_version` (explicit string or SHA-256 of first 64 bytes of preamble). Codec advertises decodable versions. Unknown → `UnsupportedWireVersion` → `STOP_RUN_PROJECTION`, `reason_code = "windsurf_wire_version_unsupported"`; request not forwarded.
7. **No live capture in CI.** Fixtures recorded out-of-band by an operator under SOW; redaction script SHA pinned in `PROVENANCE.md` (D13 convention).

## 4. Architecture

New `ProviderKind::WindsurfCascade =>` match arm in `egress_proxy::forward.rs`. Flow: `route()` → `experimental::gate` (disabled → synthetic 503, `error.code = "experimental_codec_disabled"`) → `windsurf_codec::decode_request_frame()` → existing `decision::estimate_call_cost` → `sidecar::RequestDecision` reserve → forward upstream byte-perfect with an `mpsc` tee to `decode_response_frame()` → first frame with populated `usage.input_tokens` triggers `sidecar::CommitEstimated` (latched once); `finish_reason="error"` / RST_STREAM / `UnsupportedWireVersion` / 60s timeout triggers `sidecar::ReleaseReservation`. Codec is **decode-and-observe**, never transform.

**Routing.** `routing.rs` appends two rows: `server.codeium.com` and `windsurf-server.codeium.com` + `^/exa\.language_server_pb\.LanguageServerService/CascadeChat$` → `WindsurfCascade` / `WindsurfCascadeFrame` / `WindsurfPassThrough` / `TokenizerKind::Openai` / `experimental: true`.

**Frame format + tokenizer.** Cascade is protobuf in gRPC-Web framing (5-byte length prefix + body) over HTTP/2 DATA. Hand-written `prost` descriptor in `wire.rs` covers `messages`, `model_name`, `max_tokens`, `tool_declarations.name`, `usage.{input,output}_tokens`, `finish_reason`. No dep on Codeium-owned `.proto`. Tokenizer `Openai` (BPE) — Cascade routes to GPT-4-class per public Codeium docs.

**Pass-through.** Codec consumes `mpsc` clones via `try_send`, never mutates the wire. Decode failure on a **known** version does NOT block — logs `decoder_skipped` (`reason_code = "windsurf_decode_failed"`), forwards with no reservation. Codec break degrades to no-gate; SOW states this.

## 5. Fixtures

6 `.windsurf-frames` files in `services/windsurf_codec/tests/fixtures/`: `cascade_chat_simple`, `_with_tools`, `_streaming`, `_error`, `_unknown_wire_version`, `_truncated`. Format: HTTP/2 DATA payloads with a 16-byte sidecar header per frame (`u64_be timestamp_micros + u32_be stream_id + u32_be payload_len`); see `FORMAT.md`. Redaction replaces `messages[*].content`, `tool_declarations[*].schema`, `usage.workspace_id`, `Authorization` with `FAKE_*` sentinels. `PROVENANCE.md` pins date, redaction SHA-256, wire version, SOW ID.

## 6. Slices (8)

| Slice | Title | Size |
|-------|-------|------|
| `COV_75_d18_recon_and_wire_descriptor` | Recon + `wire.rs` descriptor + parser units | M |
| `COV_76_d18_codec_module_skeleton` | Codec crate + decode entries + errors | M |
| `COV_77_d18_routing_and_experimental_gate` | Codeium routes + opt-in + boot warning | S |
| `COV_78_d18_reserve_commit_wiring` | `forward.rs`: decode → reserve → commit → release | M |
| `COV_79_d18_passthrough_and_decoder_skip` | Byte-perfect tee + `decoder_skipped` audit | M |
| `COV_80_d18_fixture_tests` | 6 fixtures + integration tests + redaction + `PROVENANCE.md` | M |
| `COV_81_d18_docs_and_sow` | SOW addendum + README badge + warning | S |
| `COV_82_d18_demo_mode` | `demo-verify-windsurf-mitm-fixture` + replay + verifier SQL | M |

## 7. Locked decisions

1. SOW-only, experimental. No GA, no Helm default, no public docs link.
2. Two-channel opt-in (env + spendguard.toml); either alone insufficient.
3. Byte-perfect pass-through; decode failure degrades to no-gate, never blocks request.
4. Tokenizer `Openai` (BPE) until Codeium discloses model — assumption in code + SOW.
5. Unknown wire version → block with `windsurf_wire_version_unsupported`; never best-effort.
6. Local proto descriptor only; no dep on Codeium-owned `.proto`.
7. No live capture in CI; fixtures recorded out-of-band under SOW.
8. R5 panel summarizer: **Security Engineer** (MITM + undocumented wire + IDE root CA dominate; same as D02 / D13 / D17).
9. Codec break is an SOW-stated risk, not a SpendGuard bug; customer signs.
10. README badge exactly `experimental — SOW only`; reviewer rejects softer wording.
