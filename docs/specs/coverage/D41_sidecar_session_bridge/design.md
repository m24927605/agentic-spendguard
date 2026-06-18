# D41 sidecar-to-ledger session bridge

**Status:** Spec - LOCKED 2026-06-13.
**Parent substrate:** [`D41_session_reservation_substrate`](../D41_session_reservation_substrate/design.md).
**Unblocks:** [`D41_voice_livekit_pipecat`](../D41_voice_livekit_pipecat/design.md).
**Owner sub-agent:** Backend Architect.

> This bridge turns the already-shipped D41 session ledger substrate into a
> runtime path usable by adapters. It does not change the adapter-facing
> session RPC contract and it does not add LiveKit or Pipecat behavior.

## 1. Problem

`D41_session_reservation_substrate` shipped the ledger tables, SQL entry
procedures, Rust ledger wrappers, SDK request builders, and local direct-ledger
demo. During closeout, the sidecar gained compile-safe `ReserveSession`,
`CommitSessionDelta`, and `ReleaseSession` handlers, but those handlers are
intentional fail-closed `UNIMPLEMENTED` stubs.

Voice adapters cannot call the session RPCs until the sidecar forwards those
RPCs to the ledger over the production ledger transport.

## 2. Goals

1. Add internal Ledger gRPC session RPCs that call the existing
   `services/ledger/src/session_reservations.rs` SQL wrappers.
2. Replace the sidecar adapter UDS session stubs with a bridge to those Ledger
   RPCs.
3. Preserve all D41S invariants: positive deltas, idempotency, tuple matching,
   signed audit rows, canonical ingest compatibility, and fail-closed outage
   behavior.
4. Add focused ledger and sidecar tests proving accepted, denied, replay,
   conflict, over-budget, release, and transport-failure behavior.
5. Add a local demo gate that exercises the sidecar UDS path, not the direct
   SQL demo path.

## 3. Non-goals

- No LiveKit Agents or Pipecat adapter code.
- No new session ledger semantics, tables, or SQL procedures unless a focused
  bug in the existing substrate blocks the bridge.
- No sidecar direct Postgres connection. The sidecar continues to talk to the
  ledger service over mTLS gRPC.
- No per-request reserve fallback for voice sessions.
- No observed provider-actual settlement lane.

## 4. Architecture - LOCKED

```text
Python voice adapter / SDK session client
  -> SidecarAdapter ReserveSession / CommitSessionDelta / ReleaseSession over UDS
  -> sidecar session bridge
  -> Ledger ReserveSession / CommitSessionDelta / ReleaseSession over mTLS gRPC
  -> services/ledger/src/session_reservations.rs
  -> post_session_reserve / post_session_commit_delta / post_session_release
  -> audit_outbox -> outbox_forwarder -> canonical_events
```

The sidecar must not import `sqlx` or `spendguard-ledger` to shortcut this
path. Ledger remains the only process that talks to the ledger database.

## 5. Wire surfaces - LOCKED

The adapter-facing RPCs stay the SR-V1 names and messages from
`proto/spendguard/sidecar_adapter/v1/adapter.proto`:

```text
rpc ReserveSession(ReserveSessionRequest) returns (ReserveSessionOutcome)
rpc CommitSessionDelta(CommitSessionDeltaRequest) returns (CommitSessionDeltaOutcome)
rpc ReleaseSession(ReleaseSessionRequest) returns (ReleaseSessionOutcome)
```

The bridge adds the same semantic RPCs to the internal
`spendguard.ledger.v1.Ledger` service:

```text
rpc ReserveSession(ReserveSessionLedgerRequest) returns (ReserveSessionLedgerResponse)
rpc CommitSessionDelta(CommitSessionDeltaLedgerRequest) returns (CommitSessionDeltaLedgerResponse)
rpc ReleaseSession(ReleaseSessionLedgerRequest) returns (ReleaseSessionLedgerResponse)
```

The Ledger request messages mirror the SR-V1 adapter fields plus the internal
tuple fields already required by `services/ledger/src/session_reservations.rs`.
Do not add JSON pass-through to the adapter-facing sidecar proto.

## 6. Ledger handler responsibility - LOCKED

Ledger handlers own the server-minted session audit envelope:

- `ReserveSessionLedger` mints `session_reservation_id` and
  `ttl_expires_at` before calling `post_session_reserve`.
- `ReserveSessionLedger` supplies both `audit_context.accepted` and
  `audit_context.denied`; the SQL procedure selects the correct branch after
  locking balances.
- `CommitSessionDeltaLedger` and `ReleaseSessionLedger` supply the signed
  `audit_context` required by the SQL procedures.
- Audit producer identity remains `ledger:session-reservation-ledger`.
- Audit signatures are Ed25519 signatures produced by the ledger signer
  already present on `LedgerService`.

The sidecar must not fabricate ledger audit signatures. Its bridge role is
validation, mapping, and fail-closed transport behavior.

## 7. Mapping rules - LOCKED

| Adapter request | Ledger request |
|---|---|
| `tenant_id` | UUID parsed unchanged. |
| `budget_id` | UUID parsed unchanged. |
| `window_instance_id` | UUID parsed unchanged. |
| `unit.unit_id` | maps to ledger `unit_id`; missing/empty is `INVALID_ARGUMENT`. |
| `pricing.pricing_version` | maps unchanged; missing/empty is `INVALID_ARGUMENT`. |
| `pricing.price_snapshot_hash` | hex-encodes to `price_snapshot_hash_hex`; empty is `INVALID_ARGUMENT`. |
| `pricing.fx_rate_version` | maps unchanged; missing/empty is `INVALID_ARGUMENT`. |
| `pricing.unit_conversion_version` | maps unchanged; missing/empty is `INVALID_ARGUMENT`. |
| `session_id`, `route`, `idempotency_key` | map unchanged and must be non-empty. |
| `estimated_amount_atomic`, `amount_atomic_delta` | positive decimal string only. |
| `event_time` | required for commit/release; if absent, reject rather than using wall-clock fallback. |

## 8. Outcome mapping - LOCKED

| Ledger result | Sidecar adapter result |
|---|---|
| Reserve `status = accepted` | `ReserveSessionOutcome.accepted`. |
| Reserve `status = denied` | `ReserveSessionOutcome.denied`; gRPC status remains OK so adapters can surface typed DENY metadata. |
| Reserve validation/SQL business error | `ReserveSessionOutcome.error` unless the ledger transport failed before a response. |
| Commit `status = accepted` or idempotent replay | `CommitSessionDeltaOutcome.accepted`. |
| Commit over-reserved / tuple mismatch / idempotency conflict | `CommitSessionDeltaOutcome.error` with stable code; never silently continue. |
| Release `status = released` or idempotent settled replay | `ReleaseSessionOutcome.accepted`. |
| Missing session / tuple conflict / idempotency conflict | `ReleaseSessionOutcome.error` with stable code. |
| Ledger transport unavailable, timeout, TLS failure | sidecar returns gRPC `UNAVAILABLE`; adapter fail-closed applies. |

## 9. Idempotency and replay - LOCKED

The ledger SQL substrate remains the source of truth. The sidecar bridge must
not add a replay cache for session reserve, commit, or release.

Replays must reach Ledger so the SQL request hash can distinguish
byte-identical replay from same-key/different-payload conflict. Same-key
conflict maps to an error outcome, not to a fresh mutation.

## 10. VERIFY-AT-IMPL marker register

| Marker | Question to pin during implementation | Owning slice |
|---|---|---|
| `SB-V1` | Exact `ledger.proto` field numbers and generated Rust client/server names. | `COV_D41S_06_sidecar_session_bridge` |
| `SB-V2` | Ledger handler audit-context minting shape for accepted and denied reserve branches. | `COV_D41S_06_sidecar_session_bridge` |
| `SB-V3` | Sidecar adapter UDS outcome mapping for accepted, denied, error, and transport outage. | `COV_D41S_06_sidecar_session_bridge` |
| `SB-V4` | Focused sidecar/ledger integration test harness and fake-ledger strategy. | `COV_D41S_06_sidecar_session_bridge` |
| `SB-V5` | Local demo mode proves sidecar UDS path, not direct SQL. | `COV_D41S_06_sidecar_session_bridge` |

## 11. Definition of done

The bridge is done when the sidecar UDS session RPCs no longer return
`UNIMPLEMENTED`, all `SB-V*` markers are pinned, the sidecar UDS demo proves
reserve -> commit -> replay -> denied -> release through Ledger gRPC, and the
existing direct D41S substrate demo still passes unchanged.
