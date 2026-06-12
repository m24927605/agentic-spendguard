# COV_D41_02 - LiveKit session reservation gate

> **Deliverable:** D41 LiveKit Agents + Pipecat voice adapters
> **Slice:** 2 of 6
> **Spec set:** [`docs/specs/coverage/D41_voice_livekit_pipecat/`](../specs/coverage/D41_voice_livekit_pipecat/)
> **Precedence:** substrate and adapter designs are LOCKED.

## Scope

Implement LiveKit reserve-before-provider-start, usage delta commits, and release/error behavior through `SpendGuardVoiceSession`.

## LOCKED design quotes

From adapter `design.md` §5:

> Adapters must not open a paid provider connection before `start()` succeeds.

From substrate `design.md` §8:

> Sidecar unavailable before session starts - Fail closed; voice session must not connect to paid model provider.

## Files touched

| File | Why |
|---|---|
| `sdk/python/src/spendguard/integrations/livekit_agents/_llm.py` | LiveKit behavior. |
| `sdk/python/src/spendguard/integrations/voice/_usage.py` | LiveKit usage extraction if shared. |
| `sdk/python/tests/integrations/test_livekit_agents.py` | Reserve/delta/release tests. |

## VERIFY-AT-IMPL pins

Pin `V41-V3`.

## Test/verification plan

- TP-D41-11..13.

## Anti-scope

- No Pipecat code.
- No demo overlay.
