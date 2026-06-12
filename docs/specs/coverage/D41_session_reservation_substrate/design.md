# D41 session reservation substrate

**Status:** Staff+ substrate design - LOCKED 2026-06-12.
**Parent strategy:** [`framework-coverage-addendum-2026-06-10.md`](../../../strategy/framework-coverage-addendum-2026-06-10.md) §3.
**Consumes:** Existing request-scoped Reserve -> CommitEstimated/Release substrate.
**Unblocks:** [`D41_voice_livekit_pipecat`](../D41_voice_livekit_pipecat/design.md).
**Owner sub-agent:** Backend Architect, with Staff+ review.

> D41 adapters MUST NOT implement a voice workaround before this substrate lands. LiveKit and Pipecat are long-lived realtime sessions; per-request reservation alone does not model continuous STT -> LLM -> TTS burn.

## 1. Problem

Existing SpendGuard adapters reserve budget immediately before one LLM call, then commit or release when that call finishes. Realtime voice agents keep a session open while token burn happens across turns, partial transcripts, tool calls, streaming model responses, and TTS output. A single pre-dispatch reservation is either too small to be safe or too large to be useful.

D41 needs a new substrate: a session-scoped reservation that holds an upper bound for a live voice session, supports incremental streaming commits, and releases the remainder at session end or timeout. This design is substrate-first because both LiveKit Agents and Pipecat need the same lifecycle.

## 2. Goals

1. Add a session reservation lifecycle that can reserve a budget envelope for a live session.
2. Support positive incremental streaming commits against that session reservation.
3. Preserve SpendGuard invariants: fail-closed reserve, idempotent commit, signed audit chain, tenant isolation, unit/window/pricing tuple match.
4. Add TS and Python SDK surfaces for session reservation clients.
5. Add a local demo gate proving reserve, incremental commits, release, reconnect idempotency, and over-budget deny.
6. Keep existing request-scoped adapters backward compatible.

## 3. Non-goals

- No provider-specific voice adapter code. That is `D41_voice_livekit_pipecat`.
- No observed-amount provider-report lane. D41 uses positive estimated streaming commits unless provider actuals are available.
- No global strongly consistent budget beyond the existing single-writer-per-budget ledger constraint.
- No UI dashboard work.
- No changes to frozen cross-language corpora except a new explicitly versioned fixture file if SDK ID derivation needs it.

## 4. Lifecycle - LOCKED

```text
SessionReserve
  -> holds amount_atomic_reserved under (tenant, budget, window, unit, pricing)
  -> returns session_reservation_id and ttl_expires_at

StreamingCommit
  -> positive delta against session_reservation_id
  -> idempotent by streaming_commit_id
  -> cumulative committed amount cannot exceed reserved amount

SessionRelease
  -> releases remaining uncommitted amount
  -> idempotent after settled

SessionExpire
  -> sidecar TTL sweep releases remaining amount
  -> emits expiration audit event
```

The session reservation is a ledger hold, not a credit line. Every commit reduces the held remainder and increases committed spend. Release settles only the uncommitted remainder.

## 5. API surface - LOCKED

Names are conceptual until proto tags are pinned by `SR-V1`. Implementations must use these semantic names unless a dated amendment changes them.

```text
rpc ReserveSession(ReserveSessionRequest) returns (ReserveSessionOutcome)
rpc CommitSessionDelta(CommitSessionDeltaRequest) returns (CommitSessionDeltaOutcome)
rpc ReleaseSession(ReleaseSessionRequest) returns (ReleaseSessionOutcome)
```

Required request fields:

| Request | Required fields |
|---|---|
| `ReserveSessionRequest` | `tenant_id`, `budget_id`, `window_instance_id`, `unit`, `pricing`, `session_id`, `route`, `estimated_amount_atomic`, `ttl_seconds`, `idempotency_key` |
| `CommitSessionDeltaRequest` | `session_reservation_id`, `streaming_commit_id`, `amount_atomic_delta`, `outcome`, `event_time`, `idempotency_key` |
| `ReleaseSessionRequest` | `session_reservation_id`, `reason_code`, `event_time`, `idempotency_key` |

Every `amount_atomic_delta` must be a positive decimal string. Zero commits are rejected.

## 6. Idempotency - LOCKED

| Operation | Idempotency tuple |
|---|---|
| Reserve session | `(tenant_id, session_id, route, idempotency_key)` |
| Commit delta | `(session_reservation_id, streaming_commit_id)` |
| Release session | `(session_reservation_id, release_idempotency_key)` |

Replay with byte-identical payload returns the original outcome. Replay with same key and different payload returns idempotency conflict. This mirrors the existing request reservation rules.

## 7. Audit vocabulary - LOCKED

New audit event family:

| Event | Emitted when |
|---|---|
| `spendguard.audit.session.reserve` | session hold created |
| `spendguard.audit.session.commit_delta` | streaming delta committed |
| `spendguard.audit.session.release` | remainder released |
| `spendguard.audit.session.expired` | TTL sweep settled remainder |
| `spendguard.audit.session.denied` | reserve denied before session starts |

Events must carry `session_reservation_id`, `tenant_id`, `budget_id`, `window_instance_id`, `unit`, `unit_id`, `pricing_version`, `price_snapshot_hash_hex`, and `event_time` where applicable. Signed CloudEvent envelope rules are unchanged.

## 8. Failure semantics - LOCKED

| Failure | Behavior |
|---|---|
| Sidecar unavailable before session starts | Fail closed; voice session must not connect to paid model provider. |
| Session reserve DENY | Fail closed; adapter surfaces typed SpendGuard denial. |
| Commit delta RPC fails mid-session | Adapter records local pending delta and retries within bounded deadline; if still failing, fail closed for further provider turns and release/TTL handles remainder. |
| Reconnect after network drop | Reuse same `session_reservation_id`; replay already-sent deltas by `streaming_commit_id`. |
| Process crash | TTL sweep releases uncommitted remainder. |

## 9. VERIFY-AT-IMPL marker register

| Marker | Question to pin during implementation | Owning slice |
|---|---|---|
| `SR-V1` | Proto message field numbers and service placement. | `COV_D41S_01_session_contract_spec_and_proto` |
| `SR-V2` | Ledger table/index shape and exact transaction boundary for reserve/commit/release. | `COV_D41S_02_ledger_session_reservation` |
| `SR-V3` | SDK TS/Python method names and generated proto paths. | `COV_D41S_03_sdk_session_client` |
| `SR-V4` | Reconnect replay algorithm and maximum local pending-delta buffer. | `COV_D41S_04_streaming_commit_and_reconnect` |
| `SR-V5` | Audit CloudEvent field list and canonical ingest mapping. | `COV_D41S_05_substrate_demo_gate` |

## 10. Slice plan

| Slice | Title |
|---|---|
| `COV_D41S_01_session_contract_spec_and_proto` | Proto/API contract and migration skeleton. |
| `COV_D41S_02_ledger_session_reservation` | Ledger stored procedures/tables and transaction tests. |
| `COV_D41S_03_sdk_session_client` | TS/Python SDK surfaces and unit tests. |
| `COV_D41S_04_streaming_commit_and_reconnect` | Retry, idempotency, reconnect, TTL sweep. |
| `COV_D41S_05_substrate_demo_gate` | Local substrate demo and docs handoff to D41 adapters. |

## 11. Definition of done

The substrate is done when all `SR-V*` markers are pinned, request-scoped adapter tests still pass, the local session demo proves reserve -> multiple positive commits -> release, and D41 voice adapter specs can reference this design without inventing their own lifecycle.

## 12. Dated implementation amendments

### 2026-06-12 - `COV_D41S_01_session_contract_spec_and_proto` SR-V1 proto pin

`SR-V1` is pinned to `proto/spendguard/sidecar_adapter/v1/adapter.proto`
inside the existing `spendguard.sidecar_adapter.v1.SidecarAdapter` service.
This placement keeps session reservation lifecycle on the same adapter UDS
contract as request-scoped `RequestDecision`, `EmitTraceEvents`, and
`ReleaseReservation`; D41 does not add a separate voice-only service.

The RPC names are the locked semantic names from §5:

```text
rpc ReserveSession(ReserveSessionRequest) returns (ReserveSessionOutcome)
rpc CommitSessionDelta(CommitSessionDeltaRequest) returns (CommitSessionDeltaOutcome)
rpc ReleaseSession(ReleaseSessionRequest) returns (ReleaseSessionOutcome)
```

Field tags are pinned in design-order for each request:

| Request | Tag layout |
|---|---|
| `ReserveSessionRequest` | `1 tenant_id`, `2 budget_id`, `3 window_instance_id`, `4 unit`, `5 pricing`, `6 session_id`, `7 route`, `8 estimated_amount_atomic`, `9 ttl_seconds`, `10 idempotency_key` |
| `CommitSessionDeltaRequest` | `1 session_reservation_id`, `2 streaming_commit_id`, `3 amount_atomic_delta`, `4 outcome`, `5 event_time`, `6 idempotency_key` |
| `ReleaseSessionRequest` | `1 session_reservation_id`, `2 reason_code`, `3 event_time`, `4 idempotency_key` |

`ReserveSessionRequest.unit` uses existing `spendguard.common.v1.UnitRef`,
and `ReserveSessionRequest.pricing` uses existing
`spendguard.common.v1.PricingFreeze`. This deliberately reuses the D05 /
HARDEN_D05_WI tuple substrate instead of adding parallel `unit_id`,
`pricing_version`, or `price_snapshot_hash` fields.

### 2026-06-12 - `COV_D41S_02_ledger_session_reservation` SR-V2 ledger transaction pin

`SR-V2` is pinned to `services/ledger/migrations/0062_session_reservations.sql`,
`services/ledger/src/session_reservations.rs`, and
`services/ledger/tests/session_reservations.rs`.

Ledger storage uses three tables:

| Table | Pinned purpose |
|---|---|
| `session_reservations` | One row per session hold with tenant, budget, window, unit, pricing tuple, reserved amount, cumulative committed amount, released amount, status, TTL, reserve idempotency hash, and original reserve outcome. |
| `session_commit_deltas` | One idempotent row per `(session_reservation_id, streaming_commit_id)` with amount, applied/denied state, request hash, and original commit outcome. |
| `session_reservation_events` | Narrow audit-visible event projection for reserve, commit delta, release, expiry, and denial events. It carries `operation_kind` so denied expiry attempts replay independently from commit denials. D41S_05 signs and canonicalizes the CloudEvent envelope. |

Accepted session operations also write existing ledger tables:

| Operation | `ledger_transactions.operation_kind` | Double-entry movement |
|---|---|---|
| Reserve | `reserve` | `available_budget` debit -> `reserved_hold` credit. |
| Streaming commit | `commit_estimated` | `reserved_hold` debit -> `committed_spend` credit. |
| Release | `release` | `reserved_hold` debit -> `available_budget` credit for the uncommitted remainder. |
| Expiry | `release` | `reserved_hold` debit -> `available_budget` credit for the uncommitted remainder. |

Transaction boundaries are pinned to Postgres stored procedures:

```text
post_session_reserve(p_request JSONB) -> JSONB
post_session_commit_delta(p_request JSONB) -> JSONB
post_session_release(p_request JSONB) -> JSONB
post_session_expire(p_request JSONB) -> JSONB
```

`post_session_reserve` locks the relevant `ledger_accounts` rows and checks
available balance before creating an accepted hold. `post_session_commit_delta`,
`post_session_release`, and `post_session_expire` lock the target
`session_reservations` row `FOR UPDATE` before mutating balances. Release and
expiry re-check their operation-level idempotency event after that lock to
close the concurrent replay race. `SESSION_TTL_NOT_EXPIRED` expiry denials are
also persisted under `operation_kind = 'expire'`, so the same key replays or
conflicts after time passes instead of mutating later. Replay uses the §6
idempotency tuples and returns the original JSONB outcome when the request hash
is identical. Same tuple with a different request hash raises SQLSTATE `40P03`.

Successful commits update only `committed_amount_atomic`; release or expiry
settles `reserved_amount_atomic - committed_amount_atomic` into
`released_amount_atomic`. Over-reserve commit attempts insert an unapplied
`session_commit_deltas` row and a
`spendguard.audit.session.denied` projection event without changing committed
or released balances.

The server-side commit procedure accepts optional internal tuple fields
(`tenant_id`, `budget_id`, `window_instance_id`, `unit_id`, and pricing freeze
fields) for defense in depth. If present, they must match the reservation row;
the sidecar wire contract remains the `SR-V1` `CommitSessionDeltaRequest`
shape from §12.

All D41S_02 stored procedures are installed as `SECURITY DEFINER` and the four
entry procedures grant execute to `ledger_application_role`; helper procedures
are not exposed for direct public execution.

Accepted reserve, commit, release, and expiry operations insert matching
`audit_outbox` and `audit_outbox_global_keys` rows in the same transaction as
the `ledger_transactions` row. The outbox row uses the existing
`spendguard.audit.outcome` envelope type and carries the concrete
`spendguard.audit.session.*` event name inside the CloudEvent payload so the
existing deferred `ledger_transactions_must_have_audit` invariant remains
true. D41S_02 tests install that deferred trigger and assert there are no
ledger transactions without an audit row.
