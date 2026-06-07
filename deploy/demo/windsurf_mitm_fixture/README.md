# DEMO_MODE=windsurf_mitm_fixture — D18 SLICE 82

> **EXPERIMENTAL — SOW only.** This demo runs the
> [`services/windsurf_codec`](../../../services/windsurf_codec/)
> crate's reverse-engineered Windsurf / Codeium Cascade codec
> against the SLICE 80 synthetic `.windsurf-rpc` fixture corpus. It
> does NOT boot the Windsurf IDE binary and does NOT touch
> `server.codeium.com` or `windsurf-server.codeium.com`.
>
> See [`services/windsurf_codec/SOW.md`](../../../services/windsurf_codec/SOW.md)
> for the customer-facing legal posture and break-window SLA.

## What this demo proves

| # | Assertion | Evidence |
|---|-----------|----------|
| 1 | The codec parses gRPC-Web framing | All 6 fixtures decode the framing layer |
| 2 | The codec decodes the Cascade envelope on known wire versions | `cascade_chat_simple` / `_with_tools` / `_streaming` / `_error` decode |
| 3 | The codec rejects unknown wire versions fail-closed | `cascade_chat_unknown_wire_version` reports `unsupported_wire_version_seen=true`, no reserve |
| 4 | The codec emits `decoder_skipped` on known-version body failures | `cascade_chat_truncated` reports `decoder_skipped=true`, no reserve |
| 5 | Reserve fires per decoded request | 4 total mock-sidecar reserves across corpus |
| 6 | Commit fires only on successful upstream | 3 commits (simple + with_tools + streaming) |
| 7 | Upstream error releases the reservation | `cascade_chat_error` upstream_error=true, no commit |
| 8 | Tool-call finish_reason preserved | `cascade_chat_with_tools` finish_reason=`tool_calls` |
| 9 | Byte-for-byte preservation across the corpus | `all_frames_round_trip=true` for every fixture |
| 10 | Upstream-forward path is wired | 4 counting-stub POSTs land (1 per decoded request) |
| 11 | Legal posture preserved | `verify_step_windsurf_mitm_fixture.sql` asserts no `codeium.com` / `wsf_` references in audit chain |

## Running the demo

```bash
make -C deploy/demo demo-up DEMO_MODE=windsurf_mitm_fixture
```

The demo brings up:

* The base SpendGuard stack (postgres + ledger + canonical-ingest +
  sidecar + outbox-forwarder + webhook-receiver).
* `counting-stub` — a Python 3.12 mock OpenAI provider.
* `windsurf-mitm-fixture-runner` — a Rust 1.87 container that
  compiles + runs the
  [`services/windsurf_codec/examples/windsurf_mitm_fixture_demo.rs`](../../../services/windsurf_codec/examples/windsurf_mitm_fixture_demo.rs)
  example.

The runner reports:

```
WINDSURF_MITM_FIXTURE_DEMO_OK
  fixtures: 6
  total_reserves: 4
  total_commits: 3
  total_upstream_errors: 1
  total_unsupported_wire_version: 1
  total_decoder_skipped: 1
  byte_for_byte_round_trip: true
```

The `make` target then runs
[`verify_step_windsurf_mitm_fixture.sql`](../verify_step_windsurf_mitm_fixture.sql)
against the ledger DB to assert the legal posture (no
`codeium.com` / `wsf_` / `sk-codeium-` references in the audit
chain).

## What this demo deliberately does NOT do

* **Boot the Windsurf IDE binary.** The codec is exercised against
  recorded synthetic fixtures only. Live Cascade traffic exercises
  happen only in SOW-customer deployments under their own legal
  sign-off.
* **Touch `server.codeium.com`.** The counting-stub stands in for
  the upstream. The codec never gets a real Codeium URL.
* **Persist real Windsurf session tokens.** The mock-sidecar lane
  is in-process; nothing lands in the ledger DB. The verify SQL
  asserts the negative side of that.
* **Test live decoding of a future Cascade wire version.** The 6
  fixtures cover `cascade.v2.0` and `cascade.v2.1` per the
  registry. A future Cascade release triggers a SOW re-capture
  workflow, not a demo update.

## When this demo fails

The runner exits non-zero on any of:

* A fixture's `all_frames_round_trip != true` (byte-for-byte
  preservation violated).
* Reserve/commit counts drift from the documented expectation.
* The counting-stub delta doesn't match the count of decoded
  requests (upstream-forward path drifted).
* A fixture is missing or fails to parse.

The verify SQL exits non-zero if any of the following land in the
audit chain:

* `codeium.com` or `windsurf-server.codeium.com` host references
  (codec MUST NOT leak the upstream host).
* Codeium credential prefixes (`sk-codeium-`, `wsf_`,
  `codeium_pat_`, `cdm_`).

## See also

* [`services/windsurf_codec/README.md`](../../../services/windsurf_codec/README.md)
* [`services/windsurf_codec/SOW.md`](../../../services/windsurf_codec/SOW.md)
* [`services/windsurf_codec/PROTOCOL.md`](../../../services/windsurf_codec/PROTOCOL.md)
* [`services/windsurf_codec/fixtures/PROVENANCE.md`](../../../services/windsurf_codec/fixtures/PROVENANCE.md)
* [`docs/specs/coverage/D18_windsurf_mitm/design.md`](../../../docs/specs/coverage/D18_windsurf_mitm/design.md)
