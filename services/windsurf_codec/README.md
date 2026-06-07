# spendguard-windsurf-codec

> **EXPERIMENTAL — SOW only. DO NOT SHIP AS A GA FEATURE.**
>
> This crate reverse-engineers the Windsurf IDE Cascade runtime's
> outbound wire protocol against `server.codeium.com` /
> `windsurf-server.codeium.com`. The wire format is observed from
> outside the Windsurf binary; no vendor source is included. The
> codec will break whenever Codeium changes their wire format.
>
> Gated to Enterprise SOW customers who signed the maintenance
> addendum. See [`SOW.md`](SOW.md) and
> [`docs/customer/sow-windsurf-mitm.md`](../../docs/customer/sow-windsurf-mitm.md).
> See [`docs/specs/coverage/D18_windsurf_mitm/design.md`](../../docs/specs/coverage/D18_windsurf_mitm/design.md)
> §1 for the standing loud warning.

## Status

D18 SLICE 75-82 (this batch):

* SLICE 75-76: framing reconnaissance documented in
  [`RECON.md`](RECON.md); fixture format documented in
  [`fixtures/README.md`](fixtures/README.md); Cascade envelope
  proto in [`src/proto/windsurf.proto`](src/proto/windsurf.proto);
  typed decode helpers in [`src/envelope.rs`](src/envelope.rs);
  wire-version registry in [`src/version.rs`](src/version.rs).
* SLICE 77: Codeium endpoint detection in [`src/routing.rs`](src/routing.rs)
  + experimental opt-in gate in [`src/experimental.rs`](src/experimental.rs)
  + stderr boot warning.
* SLICE 78: forward state machine in [`src/forward.rs`](src/forward.rs)
  with reserve / commit / release lifecycle, OpenAI canonical
  translation in [`src/translate.rs`](src/translate.rs), and
  byte-perfect re-encode in [`src/reencode.rs`](src/reencode.rs).
* SLICE 79: byte-perfect passthrough + `decoder_skipped` audit
  events in [`src/passthrough.rs`](src/passthrough.rs).
* SLICE 80: 6 synthetic `.windsurf-rpc` fixtures with PROVENANCE +
  redaction policy in [`fixtures/PROVENANCE.md`](fixtures/PROVENANCE.md);
  replay harness in [`src/replay.rs`](src/replay.rs); integration
  tests in [`tests/integration_test.rs`](tests/integration_test.rs).
* SLICE 81: this README + [`SOW.md`](SOW.md) customer addendum +
  experimental badge.
* SLICE 82: demo mode `windsurf_mitm_fixture` in
  [`../../deploy/demo/windsurf_mitm_fixture/`](../../deploy/demo/windsurf_mitm_fixture/).

## Loud experimental markers

Per D18 design.md §3 decisions 1-4, four loud markers MUST be present
at all times. A reviewer rejects any diff that removes any of them.

1. **Cargo manifest:** `Cargo.toml` carries
   `[package.metadata.experimental]` with
   `reason = "Reverse-engineered Windsurf / Codeium Cascade wire protocol. Breaks on vendor release."`
2. **Stderr banner:** `assert_experimental_banner_emitted()` in
   [`src/lib.rs`](src/lib.rs) prints the experimental notice to stderr
   on first call per process:
   ```
   [EXPERIMENTAL] windsurf-mitm codec active. Vendor protocol:
   undocumented. Support tier: SOW only. SOW:
   services/windsurf_codec/SOW.md. DO NOT SHIP IN GA CONFIG.
   ```
3. **Two-channel opt-in:** both
   `SPENDGUARD_EXPERIMENTAL_CODECS=1` (env var) AND
   `[experimental.windsurf_codec] enabled = true`
   (`spendguard.toml`) are required before any Cascade route is
   honoured. Either alone is insufficient.
4. **SOW addendum doc:** [`SOW.md`](SOW.md) carries
   `Status: EXPERIMENTAL — SOW only` above the fold,
   `noindex: true` in the front-matter, and the literal warning
   `DO NOT SHIP AS A GA FEATURE` as a top-level callout.

## Building

The crate lives outside the default workspace (see [workspace
exclude](../../Cargo.toml)) to keep default builds clean. To build
locally:

```sh
cargo build --manifest-path services/windsurf_codec/Cargo.toml
cargo test  --manifest-path services/windsurf_codec/Cargo.toml
```

With the `mitm` feature on:

```sh
cargo build --manifest-path services/windsurf_codec/Cargo.toml --features mitm
cargo test  --manifest-path services/windsurf_codec/Cargo.toml --features mitm
```

## Layout

```
services/windsurf_codec/
├── Cargo.toml                          # [package.metadata.experimental]; mitm feature
├── build.rs                            # prost-build for src/proto/windsurf.proto
├── README.md                           # this file
├── RECON.md                            # gRPC-Web framing recon + legal posture
├── PROTOCOL.md                         # field-by-field hex evidence
├── SOW.md                              # customer SOW addendum (signed)
├── fixtures/
│   ├── README.md                       # fixture format spec
│   ├── PROVENANCE.md                   # SOW provenance ledger
│   └── synthetic/                      # 6 synthetic .windsurf-rpc fixtures
├── examples/
│   ├── regenerate_fixtures.rs          # reproducible fixture corpus
│   └── windsurf_mitm_fixture_demo.rs   # SLICE 82 demo runner
├── tests/
│   └── integration_test.rs             # SLICE 80 fixture replay tests
└── src/
    ├── lib.rs                          # public API + assert_experimental_banner_emitted
    ├── framing.rs                      # GrpcWebReader + Frame + tests
    ├── envelope.rs                     # decode_request_frame + decode_response_frame
    ├── version.rs                      # WireVersion registry + detect_version
    ├── error.rs                        # WindsurfCodecError enum
    ├── routing.rs                      # Codeium endpoint detection
    ├── experimental.rs                 # two-channel opt-in gate + boot warning
    ├── passthrough.rs                  # byte-perfect tee + decoder_skipped events
    ├── reencode.rs                     # byte-for-byte re-encode helpers
    ├── translate.rs                    # Cascade ↔ OpenAI canonical translation
    ├── forward.rs                      # MITM forward state machine
    ├── openai_models.rs                # canonical OpenAI shape
    ├── replay.rs                       # .windsurf-rpc fixture replay harness
    ├── windsurf_proto.rs               # prost-generated envelope types re-export
    └── proto/
        └── windsurf.proto              # SpendGuard's own observed wire shape
```

## Adapter integrations badge

This crate ships under the **experimental — SOW only** badge.
Reviewer rejects softer wording (e.g. "experimental — preview",
"beta", "early access").

## Legal posture (one paragraph)

This codec implements **reverse-engineered interoperability**. The wire
format described in [`RECON.md`](RECON.md) and
[`src/proto/windsurf.proto`](src/proto/windsurf.proto) is observed from
outside the Windsurf binary; no vendor source code is included.
SpendGuard customers who enable the codec do so under an Enterprise SOW
that explicitly acknowledges (a) the codec can break whenever Codeium
changes their wire protocol, and (b) the customer is responsible for
confirming their own Windsurf / Codeium terms of service permit on-host
MITM of outbound traffic. The codec is gated behind the
`windsurf-mitm-experimental` workspace feature flag (applied at
`services/egress_proxy` and `services/cli`) AND the
`SPENDGUARD_EXPERIMENTAL_CODECS=1` env var; both must agree before
any Cascade route is honoured.

## Why this exists (one paragraph)

Windsurf IDE's managed Cascade mode does not call `api.openai.com` /
`api.anthropic.com` from the Cascade Agent surface. It calls
`server.codeium.com` / `windsurf-server.codeium.com` over private
gRPC-Web carrying Codeium's proprietary Cascade envelope.
SpendGuard's standard adapters (model middleware, base-URL swap,
OpenAI-compatible egress proxy) cannot see the messages — wire bytes
are Codeium-internal protobuf, not OpenAI JSON. SOW customers who
deploy Windsurf at the workforce scale and want SpendGuard budget
gating on Cascade sessions accept the codec-break risk in exchange
for full session coverage. See
[`design.md`](../../docs/specs/coverage/D18_windsurf_mitm/design.md) §2.
