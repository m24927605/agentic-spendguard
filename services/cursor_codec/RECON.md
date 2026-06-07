# Cursor Connect-RPC Wire Format — Reconnaissance Notes

> **EXPERIMENTAL.** This document is SpendGuard's own black-box observation of the
> Cursor IDE Agent's outbound wire format toward `api.cursor.sh`. It is **NOT**
> derived from vendor source, and it is **NOT** vendor-endorsed. It is the
> reverse-engineering notebook that underpins the `services/cursor_codec` crate.
>
> See [`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md) §1 for
> the standing loud warning, §6 for the experimental-marker contract, and §8
> decision 6 for the explicit "no vendor `.proto` committed verbatim" stance.

## 1. Reconnaissance status (SLICE 1)

SLICE 1 commits **the reconstruction methodology and what is known about the
framing layer in public**. It does not yet commit field-level evidence from a
real Cursor capture — that arrives in SLICE 8 alongside the `.cursor-rpc`
fixture corpus and `PROTOCOL.md`. SLICE 1 stands up the substrate; later
slices land the observed envelope shape.

The framing layer is **public Connect-RPC** (connect.build) and is well-specified
upstream. The envelope is **proprietary Cursor protobuf** and is what this
folder reverse-engineers slice-by-slice. SLICE 1 therefore documents the
framing layer in detail and leaves the envelope shape as a minimal observation
placeholder for SLICE 3.

## 2. What is known in public (framing layer)

Cursor's Agent runtime communicates with `api.cursor.sh` over HTTPS using the
Connect-RPC protocol (connect.build), a Buf-authored alternative to gRPC-Web
that speaks both HTTP/1.1 and HTTP/2. The framing layer of Connect-RPC is
publicly specified at <https://connectrpc.com/docs/protocol#streaming-rpcs>;
no reverse-engineering is required to parse it.

### 2.1 Frame layout

A single Connect-RPC frame on the wire looks like this:

```
+--------+-----------------+---------------------------+
| flags  | length (u32 BE) |   payload (length bytes)  |
| 1 byte |    4 bytes      |                           |
+--------+-----------------+---------------------------+
```

The fixed 5-byte prefix is identical for unary requests, unary responses, and
each chunk of a server-streaming response. The reader implementation in
`src/framing.rs` consumes this prefix and emits a `Frame` with `flags` and
`payload`.

### 2.2 Flag byte semantics

| Bit (mask) | Name             | Connect-RPC meaning                                   |
|------------|------------------|-------------------------------------------------------|
| `0x01`     | compressed       | Payload is compressed (algorithm advertised in headers, gzip / zstd / br) |
| `0x02`     | end-of-stream    | This frame is the **trailers** frame; payload is a length-prefixed metadata block (server-streaming only) |
| `0x04..0x80` | reserved       | Per spec; readers MUST treat as malformed today        |

SLICE 2 implements the framing layer such that:

* `0x00` is a normal data frame.
* `0x02` is the end-of-stream marker. The payload of an EOS frame is the
  Connect-RPC trailers blob (JSON for HTTP/1.1 unary fallback, length-delimited
  metadata for HTTP/2 streaming). SLICE 2 reads it as an opaque payload; SLICE
  3+ parse it.
* Any other flag combination is rejected as `InvalidFlag(_)` — the codec is
  best-effort gating, but the parser is strict so we never accidentally split
  a malformed stream into half-recognised frames.

Compression (`0x01`) is **acknowledged but not decompressed in SLICE 2**. The
reader returns the compressed payload verbatim and exposes the flag bit to the
caller; envelope decode (SLICE 3+) is gated on `flags & 0x01 == 0`. This
matches the design contract `W2` in
[`review-standards.md`](../../docs/specs/coverage/D17_cursor_mitm/review-standards.md)
§4: compression must be acted on, not silently ignored.

### 2.3 Maximum frame length

The Connect-RPC default frame ceiling matches gRPC's 4 MiB. Cursor traffic
observed empirically by community PoCs (`cursor-byok`, `cursorflow`) never
exceeds this, but the reader carries an explicit cap so a malformed length
prefix cannot allocate gigabytes. The SLICE 2 reader defaults to **8 MiB** to
leave headroom for streamed multi-file context blobs and rejects anything
larger with `OversizedFrame { length }`. Callers can override the cap via
`ConnectRpcReader::with_max_frame_len` to tighten or loosen per-deployment.

## 3. What is known from community PoCs (envelope layer hints)

This section is informational only — it does **not** commit a wire format.
`cursor-byok` and `cursorflow` are referenced as public PoCs that have
demonstrated MITM against `api.cursor.sh` for non-commercial interop. Their
observations align with the framing layer described above. They have not
published a complete `.proto`; the envelope shape below is a minimal
placeholder for SLICE 3 to expand from real capture evidence:

| Field          | Tag (placeholder) | Wire type    | Source of hint               |
|----------------|-------------------|--------------|------------------------------|
| `messages`     | 1                 | repeated msg | Community PoC observation    |
| `model`        | 2                 | string       | Community PoC observation    |
| `system`       | 3                 | string opt   | Community PoC observation    |
| `max_tokens`   | 4                 | uint32 opt   | Cursor docs (public API ref) |
| `temperature`  | 5                 | float opt    | Cursor docs (public API ref) |

SLICE 3 commits the protobuf description under `src/proto/cursor.proto` with
an explicit attribution header saying it is SpendGuard's own description of an
observed shape, not vendor source. PROTOCOL.md (SLICE 8 deliverable) will
land the hex evidence for each field once a real capture is in hand.

## 4. Why not record real Cursor traffic in SLICE 1

The SLICE 1 scope deliberately stops short of recording real `api.cursor.sh`
traffic because:

1. **Capture requires running the Cursor binary** and intercepting its TLS
   handshake with a SpendGuard-issued leaf certificate that carries the
   `api.cursor.sh` SAN. That is the D02 / SLICE 8 deliverable — it is not
   ready in SLICE 1.
2. **CI must not depend on a live Cursor binary** per
   [`review-standards.md`](../../docs/specs/coverage/D17_cursor_mitm/review-standards.md)
   §6 (`C1`) — even capture is a documented-but-uncalled tool.
3. **Synthetic fixtures crafted from the public framing spec are sufficient**
   to drive the SLICE 2 parser to ≥8 unit tests with full branch coverage.
   They are explicitly labelled as synthetic so no reviewer mistakes them
   for evidence of the real envelope shape.

The deviation, in plain terms: **SLICE 1 ships synthetic fixtures, not real
captures.** Real captures land in SLICE 8 once the egress-proxy + D02 leaf
SAN extension is wired. The synthetic fixtures are valid Connect-RPC frames
constructed by SpendGuard so the SLICE 2 parser has something to chew on
without requiring a Cursor install. The envelope payloads they carry are
placeholder protobuf — enough to exercise the framing reader, not enough to
claim observation of the real Cursor envelope.

## 5. Legal posture (one paragraph)

This codec implements **reverse-engineered interoperability**. The wire
format described here is observed from outside the Cursor binary; no vendor
source code is included. SpendGuard customers who enable the codec do so
under an Enterprise SOW that explicitly acknowledges (a) the codec can
break whenever Cursor changes their wire protocol, and (b) the customer is
responsible for confirming their own Cursor terms of service permit on-host
MITM of outbound traffic. The codec ships gated behind the
`cursor-mitm-experimental` feature flag and a stderr banner that prints on
every process start. See `README.md` for the customer-facing disclaimer.
