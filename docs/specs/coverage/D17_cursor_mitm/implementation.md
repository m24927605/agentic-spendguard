# D17 — Implementation

Companion to [`design.md`](design.md). Lays out crate boundaries, module layout, key types, feature-flag wiring, and the egress-proxy hook point.

> **EXPERIMENTAL — SOW only.** Every code path described below is feature-gated. Default builds do not include `services/cursor_codec`. See [`design.md`](design.md) §6.

## 1. Crate layout

New workspace member: `services/cursor_codec` (library crate, `spendguard-cursor-codec`).

```
services/cursor_codec/
├── Cargo.toml                  # [package.metadata.experimental]; feature gates
├── PROTOCOL.md                 # observed wire format + attribution; NOT vendor proto verbatim
├── proto/
│   └── cursor_observed.proto   # SpendGuard's reconstructed description of observed shape
├── fixtures/
│   ├── unary_chat_v1.cursor-rpc           # tagged with cursor_min_version/cursor_max_version
│   ├── streaming_chat_v1.cursor-rpc
│   └── partial_truncation_v1.cursor-rpc   # connection-drop golden
├── tools/
│   └── capture/                # documented; NOT wired into CI
│       └── README.md
├── src/
│   ├── lib.rs                  # public API: CodecPipeline, FeatureFlag::assert_enabled
│   ├── framing/
│   │   ├── mod.rs
│   │   ├── reader.rs           # ConnectRpcReader (length-prefixed)
│   │   └── writer.rs           # ConnectRpcWriter (preserve order, preserve unknown fields)
│   ├── envelope/
│   │   ├── mod.rs
│   │   ├── request.rs          # CursorChatRequest decode/encode
│   │   └── response.rs         # CursorChatResponseChunk decode/encode (server-streaming)
│   ├── translator/
│   │   ├── mod.rs
│   │   ├── to_openai.rs        # Cursor → canonical OpenAI ChatCompletionRequest
│   │   ├── to_openai_chunks.rs # Cursor stream chunk → canonical Delta
│   │   └── from_openai.rs      # canonical → Cursor (re-encode path)
│   ├── pipeline.rs             # CodecPipeline::process: reserve → forward → commit/release
│   ├── version_gate.rs         # parse Cursor client UA; gate to fixture-covered range
│   └── experimental_banner.rs  # stderr banner emitted on first use per process
└── tests/
    ├── framing_roundtrip.rs
    ├── envelope_decode.rs
    ├── translator_canonical.rs
    ├── pipeline_fixture_replay.rs   # the golden test
    └── byte_for_byte_preserve.rs
```

Workspace `Cargo.toml` adds (gated):

```toml
[workspace]
members = [
    # ...existing...
    "services/cursor_codec",     # experimental; included unconditionally so cargo metadata sees it,
                                 # but its compilation is feature-gated within egress_proxy
]
```

## 2. Feature-flag wiring

`services/egress_proxy/Cargo.toml`:

```toml
[features]
default = []
cursor-mitm-experimental = ["dep:spendguard-cursor-codec"]

[dependencies]
spendguard-cursor-codec = { path = "../cursor_codec", optional = true }
```

`services/cli/Cargo.toml` mirrors the feature so `spendguard install --include cursor` is only available in experimental builds.

## 3. Key types

```rust
// services/cursor_codec/src/lib.rs
use std::sync::OnceLock;

pub use framing::{ConnectRpcReader, ConnectRpcWriter, DecodedFrame};
pub use envelope::{CursorChatRequest, CursorChatResponseChunk};
pub use pipeline::{CodecPipeline, PipelineConfig};
pub use version_gate::{CursorClientVersion, VersionGateOutcome};

/// One-shot stderr banner per process. Called from every public entry point.
pub fn assert_experimental_banner_emitted() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        eprintln!(
            "[EXPERIMENTAL] cursor-mitm codec active. \
             Codec break SLA: docs/customer/sow-cursor-mitm.md. \
             DO NOT SHIP IN GA CONFIG."
        );
    });
}
```

```rust
// services/cursor_codec/src/pipeline.rs
pub struct CodecPipeline {
    ledger: Arc<dyn spendguard_ledger::Client>,
    upstream: Arc<dyn UpstreamConnector>,    // dials api.cursor.sh
    config: PipelineConfig,
}

pub struct PipelineConfig {
    pub tenant_id: TenantId,
    pub budget_ref: BudgetRef,
    pub fixture_replay_mode: bool,           // true in CI; false in SOW production
    pub version_gate: CursorVersionRange,    // min/max tested Cursor client version
}

impl CodecPipeline {
    /// Entry from egress_proxy when Host header == api.cursor.sh.
    pub async fn process(
        &self,
        req: hyper::Request<hyper::Body>,
    ) -> Result<hyper::Response<hyper::Body>, CodecError> {
        crate::assert_experimental_banner_emitted();
        self.config.version_gate.assert_supported(req.headers())?;

        let (parts, body) = req.into_parts();
        let frame = ConnectRpcReader::new(body).read_unary().await?;
        let cursor_req = CursorChatRequest::decode(frame.payload())?;
        let canonical = translator::to_openai::translate(&cursor_req)?;

        let reservation = self.ledger.reserve(&canonical, &self.config).await?;

        let upstream_resp = self.upstream.forward_raw(parts, frame).await?;
        let cursor_resp = self.decode_response(upstream_resp).await?;
        let canonical_resp = translator::to_openai_chunks::translate(&cursor_resp)?;

        match canonical_resp.outcome() {
            Outcome::Completed(actual) => self.ledger.commit(reservation, actual).await?,
            Outcome::Failed(reason)    => self.ledger.release(reservation, reason).await?,
        }

        // Byte-for-byte preserving re-encode of fields SpendGuard did not modify.
        let reencoded = ConnectRpcWriter::reencode_preserving_unknown(cursor_resp)?;
        Ok(reencoded.into_response())
    }
}
```

## 4. Framing parser (Slice S17_02)

Connect-RPC uses a 5-byte frame prefix: `[flags:u8][length:u32 big-endian][payload]`. `flags & 0x01` marks compressed; `flags & 0x02` marks trailing metadata in server-streaming responses.

`framing::ConnectRpcReader` is built on `tokio::io::AsyncRead` + `tokio_util::codec::FramedRead` with a custom `Decoder` that emits `DecodedFrame { flags, payload: Bytes }`. Server-streaming consumed as `Stream<Item = DecodedFrame>`. Compressed frames decoded only after the version gate has confirmed Cursor client is in-range — otherwise an unknown compression algorithm short-circuits to `CodecError::UnsupportedCompression`.

`ConnectRpcWriter` preserves frame ordering and the original flag byte. Re-encoding of the response payload uses `prost::Message::encoded_len` to ensure length-prefix correctness; unknown proto fields (vendor additions) are passed through via `UnknownFields`.

## 5. Envelope decode (Slice S17_03)

`proto/cursor_observed.proto` describes the **observed** shape. Field numbering matches what is on the wire today; field names are SpendGuard-chosen because the vendor's source is closed. `PROTOCOL.md` documents:

- Date captured.
- Cursor client version range.
- Field-by-field observation evidence (hex dump excerpts).
- Explicit statement that this is a black-box observation, not vendor source.

`CursorChatRequest::decode` uses `prost::Message::decode`. The struct has `#[prost(message, optional, tag = "...", boxed)]` with `unknown_fields: prost_types::UnknownFields` to survive vendor additions.

## 6. Translator (Slice S17_05)

```rust
// services/cursor_codec/src/translator/to_openai.rs
pub fn translate(c: &CursorChatRequest) -> Result<CanonicalChatRequest, TranslatorError> {
    Ok(CanonicalChatRequest {
        model: map_cursor_model_to_openai(&c.model)?,
        messages: c.messages.iter().map(map_message).collect::<Result<_, _>>()?,
        tools: c.tools.iter().map(map_tool).collect::<Result<_, _>>()?,
        max_tokens: c.max_tokens,
        // unknown fields survive: stored on CanonicalChatRequest.extensions
        extensions: c.unknown_fields.clone().into(),
    })
}
```

Model mapping table lives in `translator/mod.rs` as `MODEL_MAP: &[(&str, CanonicalModel)]`. Any model not in the table returns `TranslatorError::UnknownModel(name)`; the pipeline routes it to release-and-pass-through instead of failing closed (the codec is best-effort gating, not a hard policy gate per design §2 SOW disclaimer).

## 7. Egress proxy hook (Slice S17_04)

`services/egress_proxy/src/routing.rs` gains a `cursor_mitm` arm gated by `#[cfg(feature = "cursor-mitm-experimental")]`:

```rust
#[cfg(feature = "cursor-mitm-experimental")]
fn route_cursor(req: &Request<Body>) -> Option<RouteTarget> {
    use spendguard_cursor_codec::CodecPipeline;
    if req.uri().host() == Some("api.cursor.sh") {
        return Some(RouteTarget::Codec(Arc::clone(&self.cursor_codec)));
    }
    None
}
```

Build without the feature: the arm is `cfg`-stripped and `api.cursor.sh` falls through to the default `PassThrough` route, which is itself disabled because `sites.toml` does not list `api.cursor.sh` outside the SOW config.

## 8. Demo mode wiring (Slice S17_09)

`Makefile`:

```
.PHONY: demo-cursor-mitm-fixture
demo-cursor-mitm-fixture:
	@echo '[EXPERIMENTAL] cursor-mitm demo — fixture replay only.'
	cargo run -p spendguard-egress-proxy --features cursor-mitm-experimental \
		-- --demo cursor_mitm_fixture --fixture services/cursor_codec/fixtures/streaming_chat_v1.cursor-rpc
```

`services/egress_proxy/src/demo.rs` gains a `cursor_mitm_fixture` arm that loads the named fixture, pipes it through `CodecPipeline::process` with `fixture_replay_mode = true`, asserts reserve + commit landed in the in-memory ledger stub, and prints the canonical audit chain to stdout for human inspection.

## 9. SOW addendum doc (Slice S17_09)

`docs/customer/sow-cursor-mitm.md` is generated from a checked-in template. Template file: `docs/customer/sow-cursor-mitm.md.tmpl`. The build pipeline does NOT auto-rewrite it; product / legal owns updates. Doc front-matter sets `noindex: true` so it does not appear in public search.
