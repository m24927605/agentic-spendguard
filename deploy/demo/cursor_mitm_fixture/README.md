# `DEMO_MODE=cursor_mitm_fixture` — D17 SLICE 9 demo

> **EXPERIMENTAL — SOW only. DO NOT SHIP AS A GA FEATURE.**
>
> This demo exercises the Cursor IDE MITM codec
> ([`services/cursor_codec/`](../../../services/cursor_codec/))
> against the four SLICE 8 synthetic `.cursor-rpc` fixtures. It does
> NOT boot the Cursor IDE binary, does NOT touch `api.cursor.sh`, and
> does NOT capture any live Cursor session. The legal posture in
> [`services/cursor_codec/SOW.md`](../../../services/cursor_codec/SOW.md)
> §5 forbids that in CI.

## What this demo proves

1. The codec replays four synthetic `.cursor-rpc` fixtures via
   [`spendguard_cursor_codec::replay::replay_fixture`](../../../services/cursor_codec/src/replay.rs):
   * `synthetic_multiturn_v1` — 4-message conversation, 3-chunk reply.
   * `synthetic_tool_calls_v1` — Cursor Agent tool-call envelope.
   * `synthetic_error_response_v1` — upstream gRPC INTERNAL error.
   * `synthetic_long_stream_v1` — 13-chunk streaming reply.
2. The codec mock-sidecar lane records 4 reserves (1 per fixture) and
   3 commits (1 per success fixture; the error fixture skips commit
   per the `P3` release-and-pass-through contract).
3. Byte-for-byte round-trip preservation (`W5`) holds across every
   frame in every fixture.
4. The translated canonical OpenAI request body POSTs to the in-
   network counting-stub, demonstrating the upstream-forward hook is
   wired and reachable.

## Topology

```
postgres + ledger + canonical-ingest + sidecar + outbox-forwarder
                                 ▲
                                 │ (sidecar boots so the SOW
                                 │  deployment posture is faithfully
                                 │  reflected; runner uses the mock
                                 │  sidecar lane for assertions)
                                 │
                              docker compose -f compose.yaml \
                                            -f cursor_mitm_fixture/docker-compose.yaml
                                 │
   ┌─────────────────────────────┴───────────────────────────────┐
   │                                                             │
counting-stub                          cursor-mitm-fixture-runner (rust:1.83)
:8765/v1/chat/completions               │  cargo run --example cursor_mitm_fixture_demo
counter at :8765/_count                 │      ↓
   ▲                                    │  loads 4 fixtures
   │                                    │      ↓
   │ POST translated OpenAI body        │  replay_fixture: framing+envelope decode → translate → mock sidecar
   └────────────────────────────────────┤      ↓
                                        │  POST translated body to counting-stub
                                        │      ↓
                                        │  assert delta == 4, reserves == 4, commits == 3, errors == 1
                                        │      ↓
                                        │  exit 0 + print CURSOR_MITM_FIXTURE_DEMO_OK
                                        │
                                        ▼
                                 stdout: structured per-fixture report
                                         (verify SQL runs against ledger DB
                                          after a 5s drain wait)
```

The base sidecar stack still boots because the SOW deployment model
includes the sidecar — this demo reflects that posture even though
the codec's mock sidecar lane drives the assertions. The Customer's
real deployment wires the codec's `SidecarHandle` (under
`--features mitm`) to the real sidecar UDS gRPC surface.

## Run

From the repo root:

```sh
make -C deploy/demo demo-up DEMO_MODE=cursor_mitm_fixture
```

Expected output ends with:

```
CURSOR_MITM_FIXTURE_DEMO_OK
  fixtures: 4
  total_reserves: 4
  total_commits: 3
  total_upstream_errors: 1
  byte_for_byte_round_trip: true
  synthetic_multiturn_v1: reserves=1 commits=1 req_frames=1 resp_chunks=3 finish_reason=Some("stop") ...
  synthetic_tool_calls_v1: reserves=1 commits=1 req_frames=1 resp_chunks=2 finish_reason=Some("tool_calls") ...
  synthetic_error_response_v1: reserves=1 commits=0 req_frames=1 resp_chunks=0 finish_reason=None ...
  synthetic_long_stream_v1: reserves=1 commits=1 req_frames=1 resp_chunks=13 finish_reason=Some("stop") ...
[cursor-mitm-fixture-runner] PASS
```

The Makefile target then runs
[`deploy/demo/verify_step_cursor_mitm_fixture.sql`](../verify_step_cursor_mitm_fixture.sql)
against the ledger DB to confirm the counting-stub-hit-count gate
holds (and that the codec stayed offline — zero `api.cursor.sh`
references in any audit row).

## DEVIATION vs `services/cursor_codec/SOW.md` §6 demo

The SOW doc references this demo as the SOW customer's acceptance
gate. The deviations from a hypothetical "boot the Cursor binary and
MITM it" demo are documented in
[`SOW.md`](../../../services/cursor_codec/SOW.md) §5 (legal posture)
and `design.md` §1 (loud warning). In summary:

* The runner uses the **mock sidecar lane** (`InMemorySidecar`) for
  reserve/commit assertions, not the real UDS gRPC sidecar surface.
  The real sidecar boots but isn't dialed by the runner.
* The fixtures are **synthetic**, not real Cursor captures. Real-
  capture exercises live in SOW-customer-side artifacts behind their
  internal change-management workflow.
* The Cursor IDE binary is **not booted**. Per
  [`design.md`](../../../docs/specs/coverage/D17_cursor_mitm/design.md)
  §1, on-host MITM is the SOW customer's responsibility under their
  own legal sign-off.

The SOW customer's deployment exercises the same codec against live
Cursor traffic with their own validated capture path. This demo
proves the codec correctness in isolation.

## Reset between runs

```sh
make -C deploy/demo demo-down
```

The fixture corpus is committed to the workspace; the demo never
mutates fixtures. Cargo's `target/` directory is reused between runs
so the second `demo-up` is fast.
