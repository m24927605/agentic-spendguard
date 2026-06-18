# COV_D41_03 - Pipecat adapter skeleton

> **Deliverable:** D41 LiveKit Agents + Pipecat voice adapters
> **Slice:** 3 of 6
> **Spec set:** [`docs/specs/coverage/D41_voice_livekit_pipecat/`](../../specs/coverage/D41_voice_livekit_pipecat/)
> **Precedence:** substrate and adapter designs are LOCKED.

## Scope

Create Pipecat adapter module skeleton, extras/import guard, interface pin, and minimal tests. No reserve behavior lands here.

## LOCKED design quotes

From adapter `design.md` §4:

> `class SpendGuardPipecatLLMService:`
>
> `    def __init__(self, *, upstream_service: object, guard: SpendGuardVoiceSession) -> None: ...`

From adapter `design.md` §7:

> `V41-V2` - Pipecat exact `FrameProcessor`/`LLMService` interception interface.

## Files touched

| File | Why |
|---|---|
| `sdk/python/src/spendguard/integrations/pipecat/__init__.py` | Public module. |
| `sdk/python/src/spendguard/integrations/pipecat/_llm_service.py` | Skeleton wrapper. |
| package extra metadata | `[pipecat]` extra/import guard. |
| `sdk/python/tests/integrations/test_pipecat_voice.py` | Skeleton tests. |

## VERIFY-AT-IMPL pins

Pin `V41-V2`.

## Test/verification plan

- TP-D41-20 skeleton.
- A1.4 import smoke for extra.

## Anti-scope

- No reserve/delta/release behavior.
- No LiveKit changes except shared test helpers if unavoidable.
