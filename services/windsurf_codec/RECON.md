# Windsurf / Codeium Cascade Wire Format — Reconnaissance Notes

> **EXPERIMENTAL.** This document is SpendGuard's own black-box observation of
> the Windsurf IDE Cascade runtime's outbound wire format toward
> `server.codeium.com` / `windsurf-server.codeium.com`. It is **NOT** derived
> from vendor source, and it is **NOT** vendor-endorsed. It is the reverse-
> engineering notebook that underpins the `services/windsurf_codec` crate.
>
> See [`design.md`](../../docs/specs/coverage/D18_windsurf_mitm/design.md) §1
> for the standing loud warning, §3 for the experimental-marker contract, and
> §3 decision 6 for the explicit "no vendor `.proto` committed verbatim"
> stance.

## 1. Reconnaissance status (SLICE 75)

SLICE 75 commits **the reconstruction methodology and what is known about the
framing layer in public**. It does not yet commit field-level evidence from a
real Windsurf / Codeium capture — that arrives in SOW-customer-side workflows
alongside `recorded_*.windsurf-rpc` fixtures and recorded-side manifests.
SLICE 75 stands up the substrate; later slices land the observed envelope
shape via the synthetic fixtures under [`fixtures/synthetic/`](fixtures/synthetic/).

The framing layer is **public gRPC-Web** (<https://github.com/grpc/grpc-web>)
and is well-specified upstream. The envelope is **proprietary Codeium
protobuf** and is what this folder reverse-engineers slice-by-slice. SLICE 75
therefore documents the framing layer in detail and leaves the envelope
shape as a minimal observation placeholder for SLICE 76.

## 2. What is known in public (framing layer)

Windsurf's Cascade runtime communicates with `server.codeium.com` and
`windsurf-server.codeium.com` over HTTPS using gRPC-Web framing
(<https://github.com/grpc/grpc-web/blob/master/doc/PROTOCOL-WEB.md>). The
framing layer of gRPC-Web is publicly specified upstream; no reverse-
engineering is required to parse it.

### 2.1 Frame layout

A single gRPC-Web frame on the wire looks like this:

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

| Bit (mask) | Name             | gRPC-Web meaning                                |
|------------|------------------|--------------------------------------------------|
| `0x01`     | compressed       | Payload is compressed (algorithm advertised in headers, gzip / br) |
| `0x02`     | end-of-stream    | This frame is the **trailers** frame |
| `0x80`     | gRPC-Web trailers (alt) | Some gRPC-Web implementations use this bit instead of `0x02` |
| `0x40, 0x20, 0x10, 0x08, 0x04` | reserved | Per spec; readers MUST treat as malformed today |

SLICE 76 implements the framing layer such that:

* `0x00` is a normal data frame.
* `0x02` OR `0x80` is the end-of-stream marker. The payload is the
  gRPC-Web trailers blob.
* Any other flag combination is rejected as `InvalidFlag` — the codec
  is best-effort gating, but the parser is strict so we never
  accidentally split a malformed stream into half-recognised frames.

Compression (`0x01`) is **acknowledged but not decompressed in SLICE 76**.
The reader returns the compressed payload verbatim and exposes the flag
bit to the caller; envelope decode (SLICE 76+) is gated on
`flags & 0x01 == 0`. This matches D17's `W2` contract.

### 2.3 Maximum frame length

The gRPC default ceiling is 4 MiB. Windsurf Cascade traffic observed in
public Codeium documentation and community PoCs (`windsurf-proxy`) does
not appear to exceed this, but the reader carries an explicit cap so a
malformed length prefix cannot allocate gigabytes. The SLICE 76 reader
defaults to **8 MiB** to leave headroom for Cascade's larger context
blobs and rejects anything larger with `OversizedFrame { length }`.
Callers can override the cap via `GrpcWebReader::with_max_frame_len`.

## 3. What is known from community PoCs (envelope layer hints)

This section is informational only — it does **not** commit a wire format.
The community `windsurf-proxy` project demonstrated MITM against
`server.codeium.com` for non-commercial interop research. Their
observations align with the framing layer described above; they have not
published a complete `.proto`. The envelope shape below is a minimal
placeholder for SLICE 76 to expand:

| Field          | Tag (placeholder) | Wire type    | Source of hint               |
|----------------|-------------------|--------------|------------------------------|
| `messages`     | 1                 | repeated msg | Community PoC observation    |
| `model_name`   | 2                 | string       | Codeium public docs          |
| `max_tokens`   | 3                 | uint32 opt   | Codeium public docs          |
| `tool_declarations` | 4            | repeated msg | Cascade Agent mode docs      |
| `workspace_id` | 5                 | string opt   | Community PoC observation    |
| `cascade_wire_version` | 99        | string opt   | SpendGuard-pinned version stamp |

SLICE 76 commits the protobuf description under
`src/proto/windsurf.proto` with an explicit attribution header saying it
is SpendGuard's own description of an observed shape, not vendor source.

## 4. Why not record real Windsurf traffic in SLICE 75

The SLICE 75 scope deliberately stops short of recording real
`server.codeium.com` traffic because:

1. **Capture requires running the Windsurf binary** and intercepting its
   TLS handshake with a SpendGuard-issued leaf certificate that carries
   the `server.codeium.com` SAN. That is the SOW-customer-side workflow
   — it is not ready in SLICE 75.
2. **CI must not depend on a live Windsurf binary** per design.md §3
   decision 7 — even capture is a documented-but-uncalled tool.
3. **Synthetic fixtures crafted from the public gRPC-Web framing spec
   are sufficient** to drive the SLICE 76+ parser to ≥10 unit tests
   with full branch coverage. They are explicitly labelled as synthetic
   so no reviewer mistakes them for evidence of the real envelope shape.

The deviation, in plain terms: **SLICE 75 ships synthetic fixtures, not
real captures.** Real captures land in SOW-customer-side deployments
under the customer's own legal sign-off. The synthetic fixtures are
valid gRPC-Web frames constructed by SpendGuard so the SLICE 76 parser
has something to chew on without requiring a Windsurf install. The
envelope payloads they carry are placeholder protobuf — enough to
exercise the framing reader, not enough to claim observation of the
real Cascade envelope.

## 5. Legal posture (one paragraph)

This codec implements **reverse-engineered interoperability**. The wire
format described here is observed from outside the Windsurf binary; no
vendor source code is included. SpendGuard customers who enable the
codec do so under an Enterprise SOW that explicitly acknowledges (a) the
codec can break whenever Codeium changes their wire protocol, and (b)
the customer is responsible for confirming their own Windsurf /
Codeium terms of service permit on-host MITM of outbound traffic. The
codec ships gated behind the `windsurf-mitm-experimental` feature flag
and a stderr banner that prints on every process start. See
[`README.md`](README.md) for the customer-facing disclaimer.
