# COV_D41_05 - Voice shared helper and full tests

> **Deliverable:** D41 LiveKit Agents + Pipecat voice adapters
> **Slice:** 5 of 6
> **Spec set:** [`docs/specs/coverage/D41_voice_livekit_pipecat/`](../../specs/coverage/D41_voice_livekit_pipecat/)
> **Precedence:** substrate and adapter designs are LOCKED.

## Scope

Complete `SpendGuardVoiceSession`, shared options/usage helpers, fail-closed tests, tuple-threading tests, and idempotency/reconnect tests used by both adapters.

Runtime tests in this slice require `COV_D41S_06_sidecar_session_bridge` to be
on main. Pure construction/tuple validation tests may be authored before the
bridge, but no runtime claim is accepted against `UNIMPLEMENTED` stubs.

## LOCKED design quotes

From adapter `design.md` §6:

> Missing `unit_id`, `window_instance_id`, or pricing fields are construction errors for v1. This is stricter than older Python adapters because D41 is new substrate work after HARDEN_D05_UR/WI.

From substrate `design.md` §6:

> Commit delta - `(session_reservation_id, streaming_commit_id)`.

## Files touched

| File | Why |
|---|---|
| `sdk/python/src/spendguard/integrations/voice/_session.py` | Shared session helper. |
| `sdk/python/src/spendguard/integrations/voice/_options.py` | Required tuple validation. |
| `sdk/python/src/spendguard/integrations/voice/_usage.py` | Shared usage helpers. |
| `sdk/python/tests/integrations/test_voice_session.py` | Full shared tests. |
| adapter tests | Regression coverage for both frameworks. |

## Test/verification plan

- TP-D41-01..06.
- All LiveKit/Pipecat unit tests.
- TA-D41-01..02.

## Anti-scope

- No demo overlay or docs publish.
- No substrate API redesign.
