# COV_D41_04 - Pipecat session reservation gate

> **Deliverable:** D41 LiveKit Agents + Pipecat voice adapters
> **Slice:** 4 of 6
> **Spec set:** [`docs/specs/coverage/D41_voice_livekit_pipecat/`](../../specs/coverage/D41_voice_livekit_pipecat/)
> **Precedence:** substrate and adapter designs are LOCKED.

## Scope

Implement Pipecat reserve-before-provider-start, usage delta commits, and release/error behavior through `SpendGuardVoiceSession`.

This slice is blocked until `COV_D41S_06_sidecar_session_bridge` is shipped.

## LOCKED design quotes

From adapter `design.md` §5:

> each finalized LLM/STT/TTS usage point
>   -> commitSessionDelta(amount > 0, streaming_commit_id)

From adapter `review-standards.md` §2:

> Adapter uses `reserve_session`, `commit_session_delta`, and `release_session`; no local per-request approximation.

From bridge `design.md` §11:

> The bridge is done when the sidecar UDS session RPCs no longer return
> `UNIMPLEMENTED`, all `SB-V*` markers are pinned, the sidecar UDS demo proves
> reserve -> commit -> replay -> denied -> release through Ledger gRPC, and the
> existing direct D41S substrate demo still passes unchanged.

## Files touched

| File | Why |
|---|---|
| `sdk/python/src/spendguard/integrations/pipecat/_llm_service.py` | Pipecat behavior. |
| `sdk/python/src/spendguard/integrations/voice/_usage.py` | Pipecat usage extraction if shared. |
| `sdk/python/tests/integrations/test_pipecat_voice.py` | Reserve/delta/release tests. |

## VERIFY-AT-IMPL pins

Pin `V41-V4`.

## Test/verification plan

- TP-D41-21..23.

## Anti-scope

- No demo overlay.
- No per-request fallback.
