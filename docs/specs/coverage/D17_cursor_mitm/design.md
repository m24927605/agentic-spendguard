# D17 — Cursor MITM Codec (SOW-only, experimental)

**Status:** Spec — Tier 3 backlog, build plan §2.3. **EXPERIMENTAL. NOT GA. SOW-only.**
**Parent strategy:** [`framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md), Archetype III — Proprietary on-device protocol.
**Owner:** Backend Architect. R5 summarizer override: **Security Engineer**.
**Predecessor:** [`D02_closed_cli_install`](../D02_closed_cli_install/design.md). D17 reuses D02 trust store, does NOT issue a new root.

## 1. Loud warning

> **DO NOT SHIP AS A GA FEATURE.** Gated to Enterprise SOW customers who signed the maintenance addendum. Codec breaks whenever Cursor changes the wire protocol. Customer accepts break-window risk. Every install path, Cargo manifest, and doc page MUST carry the experimental marker (§6).

## 2. Problem

Cursor Agent does not call `api.openai.com` / `api.anthropic.com`. It calls `api.cursor.sh` over private Connect-RPC carrying Cursor's proprietary message envelope. Patterns 1/2/3 (model middleware, base-URL swap, OpenAI-compatible egress proxy) cannot see the messages — wire bytes are Cursor-internal protobuf, not OpenAI JSON.

Community PoCs (`cursor-byok`, `cursorflow`) prove MITM + protocol translation is mechanically feasible: install a CA the Cursor binary trusts, decode framing, extract Cursor's message shape, translate to OpenAI/Anthropic, run gating, translate back. The cost is a reverse-engineered codec the vendor can invalidate at any release.

## 3. Goals

1. New `services/cursor_codec/` crate (`spendguard-cursor-codec`) marked `[package.metadata.experimental]` and gated by `--features cursor-mitm-experimental`.
2. Decode Connect-RPC framing for unary + server-streaming RPCs against `api.cursor.sh`.
3. Translate captured Cursor `ChatRequest` / `ChatResponseChunk` envelope to/from canonical OpenAI Chat Completions shape.
4. Run reserve → pass-through → commit / release via `services/ledger`.
5. Re-encode upstream response to Cursor wire — byte-for-byte identical for fields SpendGuard did not modify.
6. Fixture-driven offline tests against committed `.cursor-rpc` recordings — no live Cursor traffic in CI.
7. Demo mode `cursor_mitm_fixture` replays a fixture and asserts the audit chain.
8. SOW addendum template `docs/customer/sow-cursor-mitm.md` with break-window SLA + customer maintenance burden + disclaimer.

## 4. Non-goals

- Live Cursor traffic in CI. Codec break = SOW customer ticket.
- Cursor login / auth handling. Existing Cursor session token forwarded opaquely.
- Cursor non-Agent surfaces (Tab autocomplete, Cmd-K inline edits). Agent only.
- New CA install. D02 root covers `api.cursor.sh`; SAN list extended, not re-issued.
- Windsurf codec — separate D18.

## 5. Architecture

```
Cursor IDE ──TLS→ SpendGuard egress proxy (D02 leaf w/ api.cursor.sh SAN)
                       │
                       ├─ if Host == api.cursor.sh AND feature `cursor-mitm-experimental`
                       │     │
                       │     ▼
                       │  services/cursor_codec
                       │     ├─ framing::ConnectRpcReader  → DecodedFrame
                       │     ├─ envelope::CursorRequest::decode → ChatRequest
                       │     ├─ translator::to_openai → CanonicalRequest
                       │     │    └→ ledger::reserve → reservation_id
                       │     ├─ upstream HTTPS to api.cursor.sh (re-encoded forward)
                       │     ├─ envelope::CursorResponseStream::decode (server-streaming)
                       │     ├─ translator::to_openai_chunks → CanonicalResponseChunks
                       │     │    └→ ledger::commit_estimated / release
                       │     └─ framing::ConnectRpcWriter → original Cursor wire
                       │
                       └─ else: D02 default pass-through (no codec)
```

`api.cursor.sh` routing is OFF by default. Customer flips `[cursor_mitm] enabled = true` in `services/egress_proxy/config/sites.toml` after signing the SOW. Leaf-cert SAN extension is gated by the same flag — without it the proxy returns upstream's real SNI and Cursor's pinning check rejects.

## 6. Experimental marker (loud, three places)

1. **Cargo manifest:** `services/cursor_codec/Cargo.toml` carries `[package.metadata.experimental] reason = "Reverse-engineered Cursor wire protocol. Breaks on vendor release."`
2. **Stderr warning on every install path:** `spendguard install --include cursor` and `egress_proxy` boot (feature on) emit to stderr: `[EXPERIMENTAL] cursor-mitm codec enabled. Codec break SLA: see docs/customer/sow-cursor-mitm.md. DO NOT SHIP IN GA CONFIG.`
3. **Docs:** `docs/customer/sow-cursor-mitm.md` has `Status: EXPERIMENTAL — SOW only` above the fold and `noindex` front-matter.

## 7. Slice plan (9 slices)

| Slice | Title | Size |
|-------|-------|------|
| `COV_S17_01_d17_recon` | Connect-RPC framing recon + fixture format | M |
| `COV_S17_02_d17_framing_parser` | `framing::ConnectRpcReader` + length-prefixed decode | M |
| `COV_S17_03_d17_envelope_extract` | Cursor `ChatRequest` / `ChatResponseChunk` proto + decode | L |
| `COV_S17_04_d17_codec_skeleton` | Crate skeleton + feature-flag wiring | S |
| `COV_S17_05_d17_translator` | Cursor↔OpenAI canonical translation (request + chunk) | M |
| `COV_S17_06_d17_reserve_commit` | Wire reserve/commit/release against `services/ledger` | M |
| `COV_S17_07_d17_response_reencode` | Re-encode upstream → Cursor wire, byte-for-byte preserve | M |
| `COV_S17_08_d17_fixtures_and_tests` | `.cursor-rpc` golden fixtures + replay harness | M |
| `COV_S17_09_d17_docs_sow_demo` | SOW doc + demo mode `cursor_mitm_fixture` | S |

## 8. Locked decisions

1. **Feature gate** `cursor-mitm-experimental` on `services/egress_proxy` AND `services/cli`. Default builds exclude the crate entirely.
2. **No live Cursor traffic in CI** — recorded fixtures only. Capture script under `services/cursor_codec/tools/capture/` documented, never CI-invoked.
3. **Byte-for-byte preservation** of unmodified fields. Translator round-trips unknown proto fields via `prost_types::UnknownFields` so vendor additions survive.
4. **Codec version pin** — every fixture tagged with `cursor_min_version` / `cursor_max_version`. Replay fails loudly when live Cursor reports a version outside the tested window.
5. **R5 panel summarizer override: Security Engineer.** Reverse-engineered codec + closed-binary MITM is the dominant risk surface; overrides build-plan default Software Architect.
6. **No vendor `.proto` committed verbatim.** Schema reconstructed from observed wire bytes; SpendGuard's own description, attribution in `services/cursor_codec/PROTOCOL.md`.
