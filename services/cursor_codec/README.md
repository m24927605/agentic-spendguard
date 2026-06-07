# spendguard-cursor-codec

> **EXPERIMENTAL — SOW only. DO NOT SHIP AS A GA FEATURE.**
>
> This crate reverse-engineers the Cursor IDE Agent's outbound wire
> protocol against `api.cursor.sh`. The wire format is observed from
> outside the Cursor binary; no vendor source is included. The codec
> will break whenever Cursor changes their wire format.
>
> Gated to Enterprise SOW customers who signed the maintenance
> addendum. See [`docs/customer/sow-cursor-mitm.md`](../../docs/customer/sow-cursor-mitm.md)
> (lands in SLICE 9). See
> [`docs/specs/coverage/D17_cursor_mitm/design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md)
> §1 for the standing loud warning.

## Status

D17 SLICE 1-4 (this branch):

* SLICE 1: framing reconnaissance documented in [`RECON.md`](RECON.md);
  fixture format documented in [`fixtures/README.md`](fixtures/README.md);
  synthetic fixtures committed under [`fixtures/synthetic/`](fixtures/synthetic/).
* SLICE 2: Connect-RPC length-prefixed framing reader
  ([`src/framing.rs`](src/framing.rs)) with ≥8 unit tests.
* SLICE 3: Cursor envelope protobuf description
  ([`src/proto/cursor.proto`](src/proto/cursor.proto)) + typed
  decode helpers ([`src/envelope.rs`](src/envelope.rs)) with ≥6
  unit tests.
* SLICE 4: crate skeleton + `mitm` feature flag + stderr banner +
  this README.

Out of scope for SLICE 1-4 (deferred to later slices):

* Translator: Cursor envelope ↔ canonical OpenAI Chat Completions (SLICE 5).
* Reserve / commit / release wiring against `services/ledger` (SLICE 6).
* Response re-encode preserving byte-for-byte the fields SpendGuard did
  not modify (SLICE 7).
* `.cursor-rpc` golden fixture corpus + replay harness (SLICE 8).
* SOW addendum doc + demo mode `cursor_mitm_fixture` (SLICE 9).

## Loud experimental markers

Per D17 design.md §6, three loud markers MUST be present at all times:

1. **Cargo manifest:** `Cargo.toml` carries
   `[package.metadata.experimental]` with
   `reason = "Reverse-engineered Cursor wire protocol. Breaks on vendor release."`
2. **Stderr banner:** `assert_experimental_banner_emitted()` in
   [`src/lib.rs`](src/lib.rs) prints the experimental notice to stderr
   on first call per process.
3. **SOW addendum doc:** `docs/customer/sow-cursor-mitm.md` (lands in
   SLICE 9) carries `Status: EXPERIMENTAL — SOW only` above the fold,
   `noindex: true` front-matter, and the literal warning
   `DO NOT SHIP AS A GA FEATURE`.

## Building

The crate lives outside the default workspace (see [workspace
exclude](../../Cargo.toml)) to keep default builds clean. To build
locally:

```sh
cargo build --manifest-path services/cursor_codec/Cargo.toml
cargo test  --manifest-path services/cursor_codec/Cargo.toml
```

With the `mitm` feature on (the per-crate gate; the workspace-level
`cursor-mitm-experimental` feature is applied at the egress-proxy and
CLI consumer level):

```sh
cargo build --manifest-path services/cursor_codec/Cargo.toml --features mitm
```

## Layout

```
services/cursor_codec/
├── Cargo.toml                  # [package.metadata.experimental]; mitm feature
├── build.rs                    # prost-build for src/proto/cursor.proto
├── README.md                   # this file
├── RECON.md                    # Connect-RPC framing recon + legal posture
├── fixtures/
│   ├── README.md               # fixture format spec
│   └── synthetic/              # synthetic Connect-RPC frames (SLICE 1)
└── src/
    ├── lib.rs                  # public API + assert_experimental_banner_emitted
    ├── framing.rs              # ConnectRpcReader + Frame + tests
    ├── cursor_proto.rs         # prost-generated envelope types re-export
    ├── envelope.rs             # decode_chat_request / decode_chat_response_chunk
    └── proto/
        └── cursor.proto        # SpendGuard's own observed wire shape
```

## Legal posture (one paragraph)

This codec implements **reverse-engineered interoperability**. The wire
format described in [`RECON.md`](RECON.md) and
[`src/proto/cursor.proto`](src/proto/cursor.proto) is observed from
outside the Cursor binary; no vendor source code is included. SpendGuard
customers who enable the codec do so under an Enterprise SOW that
explicitly acknowledges (a) the codec can break whenever Cursor changes
their wire protocol, and (b) the customer is responsible for confirming
their own Cursor terms of service permit on-host MITM of outbound
traffic. The codec is gated behind the `cursor-mitm-experimental`
workspace feature flag (applied at `services/egress_proxy` and
`services/cli`) and a stderr banner that prints on every process start.

## Why this exists (one paragraph)

Cursor IDE does not call `api.openai.com` / `api.anthropic.com` from the
Agent surface. It calls `api.cursor.sh` over private Connect-RPC carrying
Cursor's proprietary message envelope. SpendGuard's standard adapters
(model middleware, base-URL swap, OpenAI-compatible egress proxy) cannot
see the messages — wire bytes are Cursor-internal protobuf, not OpenAI
JSON. SOW customers who deploy Cursor at the workforce scale and want
SpendGuard budget gating on Cursor sessions accept the codec-break risk
in exchange for full session coverage. See
[`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md) §2.
