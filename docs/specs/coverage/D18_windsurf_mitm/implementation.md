# D18 — Implementation

Companion to [`design.md`](design.md). Lays out crate layout, key types, code skeleton, the routing delta, the experimental gate, and the demo wiring. The codec runs **outside** the existing egress proxy module graph as a new workspace crate so the experimental surface can be removed wholesale by deleting one directory.

## 1. Files touched

```
proto/spendguard/common/v1/common.proto              # +ProviderKind::WINDSURF_CASCADE enum value (additive)

services/windsurf_codec/                             # NEW crate
    Cargo.toml                                       # publish = false
    README.md                                        # "experimental — SOW only"
    src/lib.rs                                       # public API: decode_request_frame, decode_response_frame
    src/wire.rs                                      # local minimal proto descriptor for Cascade frames
    src/error.rs                                     # WindsurfCodecError enum
    src/passthrough.rs                               # byte-perfect tee that feeds decoder + forwards upstream
    src/version.rs                                   # known wire-version registry
    tests/fixtures/
        FORMAT.md
        PROVENANCE.md
        cascade_chat_simple.windsurf-frames
        cascade_chat_with_tools.windsurf-frames
        cascade_chat_streaming.windsurf-frames
        cascade_chat_error.windsurf-frames
        cascade_chat_unknown_wire_version.windsurf-frames
        cascade_chat_truncated.windsurf-frames
    tests/decode_request.rs
    tests/decode_response.rs
    tests/passthrough_byte_equivalence.rs
    tests/unsupported_wire_version.rs

services/egress_proxy/
    src/routing.rs                                   # +Codeium rows, experimental=true
    src/experimental.rs                              # NEW — two-channel opt-in gate
    src/forward.rs                                   # +WindsurfCascade branch (decode + reserve + commit)
    src/main.rs                                      # +boot stderr warning
    src/lib.rs                                       # re-export experimental module
    tests/fixtures/windsurf/                         # symlink to ../../../windsurf_codec/tests/fixtures
    tests/windsurf_mitm_e2e.rs                       # integration test using fixture replay

services/canonical_ingest/migrations/
    0048_audit_outbox_experimental_codec.sql         # +experimental_codec TEXT NULL column for dashboards

deploy/demo/
    Makefile                                         # +demo-verify-windsurf-mitm-fixture
    verify_step_windsurf_mitm.sql
    runtime/windsurf_mitm_demo.sh
    runtime/replay_windsurf_frames.py

docs/customer/
    sow-windsurf-mitm.md                             # NEW — customer SOW addendum template (not in site-v2)

README.md                                            # +Windsurf row with "experimental — SOW only" badge
```

Migration 0047 is reserved for D17; D18 takes 0048. (If D17 ships first with a different migration count, the slice plan re-numbers at PR time — reviewer rejects any number collision.)

## 2. Proto delta

```proto
// proto/spendguard/common/v1/common.proto
//
// Additive enum value. WINDSURF_CASCADE = 11 (next free tag after the
// last existing ProviderKind). Reviewer confirms tag by grep.
enum ProviderKind {
  // ...existing values...
  PROVIDER_KIND_WINDSURF_CASCADE = 11;
}
```

No other proto changes. The codec produces normal `ClaimEstimate` / `CommitEstimated` payloads with `provider_kind = WINDSURF_CASCADE`.

## 3. Schema migration

### 3.1 `0048_audit_outbox_experimental_codec.sql`

```sql
-- D18 — audit-row tag identifying the experimental codec that produced
-- the row. NULL for non-experimental (default BYOK / subscription_meter
-- / etc.) traffic. Enables dashboards to filter "codec-mediated" rows.
ALTER TABLE audit_outbox
  ADD COLUMN experimental_codec TEXT NULL
    CHECK (experimental_codec IS NULL OR experimental_codec IN
           ('windsurf_managed_cascade', 'cursor_byok_managed'));

CREATE INDEX idx_audit_outbox_experimental_codec
  ON audit_outbox (tenant_id, experimental_codec, occurred_at)
  WHERE experimental_codec IS NOT NULL;
```

`'cursor_byok_managed'` is pre-listed in the CHECK constraint to avoid a follow-up migration when D17 lands; reviewer confirms the cross-D17 anchor.

## 4. `services/windsurf_codec/` crate

### 4.1 `Cargo.toml`

```toml
[package]
name = "spendguard-windsurf-codec"
version = "0.0.1"
edition = "2021"
publish = false        # experimental — SOW only
description = "Experimental Windsurf managed-Cascade MITM codec. SOW-only."

[dependencies]
bytes = "1"
prost = "0.13"
thiserror = "1"
tokio = { version = "1", features = ["io-util", "macros"] }
tracing = "0.1"
spendguard-common = { path = "../common" }

[dev-dependencies]
tokio = { version = "1", features = ["full"] }
hex = "0.4"
```

No `reqwest`, no HTTP client — the codec is parse-only; the proxy owns the network.

### 4.2 `src/lib.rs`

```rust
//! Experimental Windsurf managed-Cascade codec.
//!
//! SOW-ONLY. Decoder for the proprietary Cascade wire format used by
//! `server.codeium.com`. The vendor protocol is undocumented; this
//! decoder is fixture-driven and may break without notice when Codeium
//! ships a wire-version bump. See `design.md` §3 for the experimental
//! posture and §4.4 for the pass-through fallback contract.

pub mod error;
pub mod passthrough;
pub mod version;
pub mod wire;

pub use error::WindsurfCodecError;
pub use version::{KNOWN_WIRE_VERSIONS, WireVersion};

use bytes::Bytes;

/// Decoded view of a Cascade request frame. Only the fields SpendGuard
/// uses for cost estimation are populated.
#[derive(Debug, Clone)]
pub struct CascadeRequest {
    pub model_name: String,
    pub messages: Vec<CascadeMessage>,
    pub tool_declarations: Vec<CascadeToolDecl>,
    pub max_tokens: Option<i64>,
    pub wire_version: WireVersion,
}

#[derive(Debug, Clone)]
pub struct CascadeMessage {
    pub role: String,        // user | assistant | system | tool
    pub content: String,     // redacted in fixtures
}

#[derive(Debug, Clone)]
pub struct CascadeToolDecl {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct CascadeResponseDelta {
    pub text_chunk: Option<String>,
    pub finish_reason: Option<String>,
    pub usage: Option<CascadeUsage>,
    pub wire_version: WireVersion,
}

#[derive(Debug, Clone, Copy)]
pub struct CascadeUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
}

/// Decode a single Cascade request frame from a 5-byte-prefixed gRPC-Web
/// payload. Returns `Err(UnsupportedWireVersion)` if the version stamp
/// is not in `KNOWN_WIRE_VERSIONS`.
pub fn decode_request_frame(buf: &Bytes) -> Result<CascadeRequest, WindsurfCodecError> {
    let (version, body) = wire::strip_grpc_web_prefix(buf)?;
    if !version::is_known(&version) {
        return Err(WindsurfCodecError::UnsupportedWireVersion(version));
    }
    wire::parse_request(&version, body)
}

/// Decode a single Cascade response delta. Streaming responses produce
/// many frames; SpendGuard cares about the first frame carrying a
/// populated `usage.input_tokens` value (commit-estimated trigger).
pub fn decode_response_frame(buf: &Bytes) -> Result<CascadeResponseDelta, WindsurfCodecError> {
    let (version, body) = wire::strip_grpc_web_prefix(buf)?;
    if !version::is_known(&version) {
        return Err(WindsurfCodecError::UnsupportedWireVersion(version));
    }
    wire::parse_response(&version, body)
}
```

### 4.3 `src/error.rs`

```rust
use thiserror::Error;
use crate::WireVersion;

#[derive(Debug, Error)]
pub enum WindsurfCodecError {
    #[error("buffer too short for gRPC-Web length prefix")]
    TruncatedPrefix,
    #[error("gRPC-Web payload truncated: expected {expected} bytes, got {got}")]
    TruncatedBody { expected: usize, got: usize },
    #[error("protobuf decode failed: {0}")]
    Protobuf(#[from] prost::DecodeError),
    #[error("unsupported wire version: {0:?}")]
    UnsupportedWireVersion(WireVersion),
    #[error("required field missing: {0}")]
    MissingField(&'static str),
}
```

### 4.4 `src/version.rs`

```rust
//! Wire-version registry. Each captured fixture pins a version; the
//! codec advertises which versions it can decode. An inbound frame
//! whose version is not in this list MUST fail closed (no silent
//! best-effort decode).

use std::fmt;

/// Cascade wire version. Either an explicit `cascade_wire_version`
/// field from the response envelope, or a SHA-256 of the first 64
/// bytes of the streaming preamble for frames that lack the field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WireVersion {
    Explicit(String),     // e.g. "cascade.v2.1"
    PreambleHash([u8; 32]),
}

impl fmt::Display for WireVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Explicit(s) => write!(f, "explicit:{s}"),
            Self::PreambleHash(h) => write!(f, "preamble_sha256:{}", hex::encode(h)),
        }
    }
}

/// Known wire versions this codec can decode. Reviewer rejects any
/// expansion not accompanied by a fixture in
/// `tests/fixtures/cascade_chat_*.windsurf-frames`.
pub const KNOWN_WIRE_VERSIONS: &[&str] = &[
    "cascade.v2.0",
    "cascade.v2.1",
];

pub fn is_known(v: &WireVersion) -> bool {
    match v {
        WireVersion::Explicit(s) => KNOWN_WIRE_VERSIONS.contains(&s.as_str()),
        // Preamble-hash versions are pinned per-fixture in PROVENANCE.md
        // and registered at boot via SPENDGUARD_WINDSURF_PREAMBLE_HASHES.
        WireVersion::PreambleHash(h) => {
            // Reads env at startup; cached. Stub here for skeleton.
            crate::version::env_pinned_hashes().contains(h)
        }
    }
}

fn env_pinned_hashes() -> &'static [[u8; 32]] {
    // Skeleton: returns a `OnceLock<Vec<[u8; 32]>>` populated from
    // SPENDGUARD_WINDSURF_PREAMBLE_HASHES (comma-separated hex strings).
    static HASHES: std::sync::OnceLock<Vec<[u8; 32]>> = std::sync::OnceLock::new();
    HASHES.get_or_init(|| {
        std::env::var("SPENDGUARD_WINDSURF_PREAMBLE_HASHES")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .filter_map(|s| hex::decode(s.trim()).ok())
                    .filter_map(|v| v.try_into().ok())
                    .collect()
            })
            .unwrap_or_default()
    })
}
```

### 4.5 `src/wire.rs` (skeleton)

```rust
//! Local minimal proto descriptor for Cascade frames. We do NOT pull in
//! Codeium-owned `.proto` files (not public). We hand-declare only the
//! fields D18 needs.

use bytes::Bytes;
use prost::Message;
use crate::{CascadeRequest, CascadeResponseDelta, WindsurfCodecError, WireVersion};

#[derive(Clone, PartialEq, Message)]
pub struct CascadeRequestPb {
    #[prost(string, tag = "1")]
    pub model_name: String,
    #[prost(message, repeated, tag = "2")]
    pub messages: Vec<CascadeMessagePb>,
    #[prost(message, repeated, tag = "3")]
    pub tool_declarations: Vec<CascadeToolDeclPb>,
    #[prost(int64, optional, tag = "4")]
    pub max_tokens: Option<i64>,
    #[prost(string, optional, tag = "99")]
    pub cascade_wire_version: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
pub struct CascadeMessagePb {
    #[prost(string, tag = "1")]
    pub role: String,
    #[prost(string, tag = "2")]
    pub content: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct CascadeToolDeclPb {
    #[prost(string, tag = "1")]
    pub name: String,
}

pub fn strip_grpc_web_prefix(buf: &Bytes) -> Result<(WireVersion, Bytes), WindsurfCodecError> {
    if buf.len() < 5 {
        return Err(WindsurfCodecError::TruncatedPrefix);
    }
    let compressed = buf[0] != 0;
    let len = u32::from_be_bytes(buf[1..5].try_into().unwrap()) as usize;
    if buf.len() < 5 + len {
        return Err(WindsurfCodecError::TruncatedBody {
            expected: 5 + len,
            got: buf.len(),
        });
    }
    let body = buf.slice(5..5 + len);
    if compressed {
        // gzip path stub — Cascade uses identity in captured fixtures.
        // Reviewer rejects merging a real gzip path without a fixture.
        return Err(WindsurfCodecError::MissingField("gzip_unsupported"));
    }
    // Try explicit version first; fall back to preamble hash.
    let version = detect_version(&body);
    Ok((version, body))
}

fn detect_version(body: &Bytes) -> WireVersion {
    // Cheap peek: try-decode envelope, look for cascade_wire_version.
    if let Ok(req) = CascadeRequestPb::decode(body.clone()) {
        if let Some(v) = req.cascade_wire_version {
            return WireVersion::Explicit(v);
        }
    }
    // Fallback: SHA-256 of first 64 bytes of body.
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&body[..body.len().min(64)]);
    WireVersion::PreambleHash(h.finalize().into())
}

pub fn parse_request(_v: &WireVersion, body: Bytes) -> Result<CascadeRequest, WindsurfCodecError> {
    let pb = CascadeRequestPb::decode(body)?;
    Ok(CascadeRequest {
        model_name: pb.model_name,
        messages: pb.messages.into_iter()
            .map(|m| crate::CascadeMessage { role: m.role, content: m.content })
            .collect(),
        tool_declarations: pb.tool_declarations.into_iter()
            .map(|t| crate::CascadeToolDecl { name: t.name })
            .collect(),
        max_tokens: pb.max_tokens,
        wire_version: pb.cascade_wire_version
            .map(WireVersion::Explicit)
            .unwrap_or_else(|| WireVersion::Explicit("cascade.v2.0".into())),
    })
}

pub fn parse_response(_v: &WireVersion, _body: Bytes)
    -> Result<CascadeResponseDelta, WindsurfCodecError>
{
    // Response descriptor (omitted in this skeleton; mirrors request).
    todo!("parse Cascade response delta — implemented in COV_75")
}
```

### 4.6 `src/passthrough.rs`

```rust
//! Byte-perfect tee. Upstream traffic is forwarded byte-for-byte; the
//! decoder runs in parallel on a clone of the stream. Decode failure
//! NEVER blocks the request — it logs and forwards (no reservation).

use bytes::Bytes;
use tokio::sync::mpsc;
use tracing::warn;

pub struct DecoderTap {
    pub frames_tx: mpsc::Sender<Bytes>,
}

impl DecoderTap {
    /// Feed each upstream chunk into the tap. Returns the same bytes
    /// unchanged for the caller to forward.
    pub fn observe<'a>(&self, chunk: &'a Bytes) -> &'a Bytes {
        if self.frames_tx.try_send(chunk.clone()).is_err() {
            // Decoder is slow or dead — drop the observation; never
            // backpressure the wire. Audit-row writer will emit
            // `decoder_skipped` when it notices missing frames.
            warn!(kind = "windsurf_decoder_tap_dropped",
                  "decoder tap backpressure — upstream forwarded without observation");
        }
        chunk
    }
}
```

## 5. Egress proxy integration

### 5.1 `experimental.rs` — two-channel gate

```rust
//! D18 §3 — experimental codec opt-in gate. Both env var AND config
//! file MUST agree before any experimental codec is enabled.

use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct ExperimentalConfig {
    #[serde(default)]
    pub windsurf_codec: WindsurfExperimentalConfig,
    #[serde(default)]
    pub cursor_codec: CursorExperimentalConfig,
}

#[derive(Debug, Deserialize, Default)]
pub struct WindsurfExperimentalConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Default)]
pub struct CursorExperimentalConfig {
    #[serde(default)]
    pub enabled: bool,
}

/// Returns true iff BOTH env var AND config say yes.
pub fn windsurf_codec_enabled(cfg: &ExperimentalConfig) -> bool {
    let env_ok = std::env::var("SPENDGUARD_EXPERIMENTAL_CODECS").as_deref() == Ok("1");
    env_ok && cfg.windsurf_codec.enabled
}
```

### 5.2 Boot warning (`main.rs` addition)

```rust
if experimental::windsurf_codec_enabled(&cfg.experimental) {
    let last_verified = std::fs::read_to_string(
        "services/windsurf_codec/tests/fixtures/PROVENANCE.md"
    ).ok()
     .and_then(|s| s.lines()
        .find(|l| l.starts_with("last_verified_capture:"))
        .map(|l| l.trim_start_matches("last_verified_capture:").trim().to_string()))
     .unwrap_or_else(|| "unknown".into());

    tracing::warn!(
        kind = "experimental_codec_enabled",
        codec = "windsurf_managed_cascade",
        vendor_protocol = "undocumented",
        support_tier = "sow_only",
        last_verified_capture = %last_verified,
        "experimental Windsurf codec enabled — vendor wire may change without notice"
    );
}
```

### 5.3 `forward.rs` Cascade branch (sketch)

```rust
ProviderKind::WindsurfCascade => {
    if !experimental::windsurf_codec_enabled(&state.cfg.experimental) {
        // Route exists but codec is gated off — refuse with 503 and
        // explicit reason so SOW operators see the misconfiguration.
        return synthetic_503_codec_disabled("windsurf_managed_cascade");
    }

    let (tap_tx, mut tap_rx) = mpsc::channel(64);
    let tap = passthrough::DecoderTap { frames_tx: tap_tx };

    // Spawn decoder side-task; never blocks the forward path.
    let decoder_handle = tokio::spawn(async move {
        let mut request_buf = bytes::BytesMut::new();
        while let Some(chunk) = tap_rx.recv().await {
            request_buf.extend_from_slice(&chunk);
            if let Ok(req) = windsurf_codec::decode_request_frame(&request_buf.clone().freeze()) {
                return Ok(req);
            }
        }
        Err(WindsurfCodecError::TruncatedBody { expected: 0, got: request_buf.len() })
    });

    // Forward upstream, teeing through the tap (byte-perfect).
    let upstream_resp = forward_with_tap(&req, tap).await?;

    match decoder_handle.await? {
        Ok(req) => reserve_and_commit(req, upstream_resp).await,
        Err(_)  => {
            audit::emit_decoder_skipped("windsurf_managed_cascade").await;
            // Pass-through fallback: forward upstream response unchanged,
            // no reservation, no commit, no release. Per design §4.4.
            Ok(upstream_resp)
        }
    }
}
```

`reserve_and_commit` reuses the existing helper from BYOK path — same `sidecar.RequestDecision` + `CommitEstimated` + `ReleaseReservation` plumbing. The codec only changes how the `ClaimEstimate` is **built**, not how the ledger is updated.

## 6. Routing additions

```rust
// services/egress_proxy/src/routing.rs (append after existing rows)
RoutingRow {
    inbound_host: "server.codeium.com",
    inbound_path_regex: r"^/exa\.language_server_pb\.LanguageServerService/CascadeChat$",
    upstream: "https://server.codeium.com",
    provider_kind: ProviderKind::WindsurfCascade,
    request_shape: RequestShape::WindsurfCascadeFrame,
    encoder: Encoder::WindsurfPassThrough,
    tokenizer_kind: TokenizerKind::Openai,  // §7 locked decision #4
    experimental: true,
},
RoutingRow {
    inbound_host: "windsurf-server.codeium.com",
    inbound_path_regex: r"^/exa\.language_server_pb\.LanguageServerService/CascadeChat$",
    upstream: "https://windsurf-server.codeium.com",
    provider_kind: ProviderKind::WindsurfCascade,
    request_shape: RequestShape::WindsurfCascadeFrame,
    encoder: Encoder::WindsurfPassThrough,
    tokenizer_kind: TokenizerKind::Openai,
    experimental: true,
},
```

## 7. Demo wiring

`deploy/demo/runtime/windsurf_mitm_demo.sh`:

```bash
#!/usr/bin/env bash
# D18 demo: replay a recorded .windsurf-frames fixture through the proxy
# and assert (a) the codec decoded the frame, (b) the reservation was
# committed, (c) the audit row carries experimental_codec=windsurf_managed_cascade.
set -euo pipefail

FIXTURE="${1:-cascade_chat_simple}"

# Two-channel opt-in (per design §3).
export SPENDGUARD_EXPERIMENTAL_CODECS=1
cat > /tmp/spendguard-demo.toml <<EOF
[experimental.windsurf_codec]
enabled = true
EOF

docker compose -f deploy/demo/compose.yaml \
    --env-file /tmp/spendguard-demo.toml \
    up -d egress_proxy sidecar canonical_ingest windsurf_stub_upstream

python3 deploy/demo/runtime/replay_windsurf_frames.py \
    --fixture "services/windsurf_codec/tests/fixtures/${FIXTURE}.windsurf-frames" \
    --proxy http://localhost:8443

psql "$DATABASE_URL" -f deploy/demo/verify_step_windsurf_mitm.sql
```

`deploy/demo/verify_step_windsurf_mitm.sql`:

```sql
DO $$
DECLARE
    codec_count INT;
    commit_count INT;
BEGIN
    SELECT count(*) INTO codec_count
      FROM audit_outbox
     WHERE tenant_id = 'demo'
       AND experimental_codec = 'windsurf_managed_cascade';
    ASSERT codec_count >= 1, 'expected >= 1 windsurf codec audit row';

    SELECT count(*) INTO commit_count
      FROM ledger_entries
     WHERE tenant_id = 'demo'
       AND created_at > now() - interval '5 minutes';
    ASSERT commit_count >= 1, 'expected at least one ledger entry from windsurf reserve+commit';
END $$;
```

## 8. SOW addendum (`docs/customer/sow-windsurf-mitm.md`)

Template structure (full prose in COV_81):

1. Recital naming the customer + paid Windsurf seat count.
2. Statement that SpendGuard Windsurf Cascade codec is `experimental`, vendor-protocol-mediated, and may break without notice.
3. Acknowledgement: codec break → no cost gating until SpendGuard ships a re-capture.
4. Acknowledgement: no SLA, no public docs surface, no Helm default.
5. Capture cadence: SpendGuard provides one re-capture per paid hour pool (initial 8h, top-up at customer expense).
6. Customer authorizes installation of SpendGuard root CA on IDE-host machines (cross-link to D02 install).
7. Signature blocks (customer + SpendGuard rep).
