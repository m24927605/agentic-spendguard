# COV_D41_01 - LiveKit adapter skeleton

> **Deliverable:** D41 LiveKit Agents + Pipecat voice adapters
> **Slice:** 1 of 6
> **Spec set:** [`docs/specs/coverage/D41_voice_livekit_pipecat/`](../specs/coverage/D41_voice_livekit_pipecat/)
> **Substrate spec:** [`D41_session_reservation_substrate`](../specs/coverage/D41_session_reservation_substrate/)
> **Precedence:** substrate `design.md` and adapter `design.md` are LOCKED and trump this doc.

## Scope

Create LiveKit adapter module skeleton, extras/import guard, interface pin, and minimal type/import tests. No reserve behavior lands here.

## LOCKED design quotes

From `design.md` §4:

> `class SpendGuardLiveKitLLM:`
>
> `    def __init__(self, *, upstream_llm: object, guard: SpendGuardVoiceSession) -> None: ...`

From `design.md` §7:

> `V41-V1` - LiveKit Agents exact LLM plugin/wrapper interface and session start hook.

## Files touched

| File | Why |
|---|---|
| `sdk/python/src/spendguard/integrations/livekit_agents/__init__.py` | Public module. |
| `sdk/python/src/spendguard/integrations/livekit_agents/_llm.py` | Skeleton wrapper. |
| package extra metadata | `[livekit]` extra/import guard. |
| `sdk/python/tests/integrations/test_livekit_agents.py` | Skeleton tests. |

## VERIFY-AT-IMPL pins

Pin `V41-V1`.

## Test/verification plan

- TP-D41-10 skeleton.
- A1.4 import smoke for extra.

## Anti-scope

- No reserve/delta/release behavior.
- No Pipecat code.
