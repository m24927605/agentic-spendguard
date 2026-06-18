# D41 - LiveKit Agents + Pipecat voice adapters

**Status:** Spec - LOCKED 2026-06-12.
**Parent strategy:** [`framework-coverage-addendum-2026-06-10.md`](../../../strategy/framework-coverage-addendum-2026-06-10.md) §3.
**Substrate prerequisite:** [`D41_session_reservation_substrate`](../D41_session_reservation_substrate/design.md).
**Owner sub-agent:** AI Engineer.

> D41 voice adapters consume the session reservation substrate. They do not invent a per-framework budget lifecycle.

## 1. Problem

Voice agents burn tokens continuously across STT, LLM, tool, and TTS loops. LiveKit Agents and Pipecat are the two candidate frameworks selected in the addendum because both have strong adoption and clean interception surfaces, but neither ships open-source hard budget primitives for self-hosters.

The SpendGuard adapter must fail closed before the voice session connects to paid model providers, stream positive commit deltas during the session, and release unused budget at session end.

## 2. Goals

1. Python adapter for LiveKit Agents session LLM plugin path.
2. Python adapter for Pipecat pipeline/LLM service path.
3. Shared voice budget helper that wraps D41 session reservation substrate.
4. Demo mode `voice_session_guard` proving one LiveKit-style run and one Pipecat-style run against local stubs.
5. Docs page explaining session-scoped reservation, streaming commit, and failure posture.

## 3. Non-goals

- No Node LiveKit adapter in v1.
- No real microphone/browser audio in hard gate; demo uses deterministic text/audio fixtures and local stubs.
- No STT/TTS provider-specific hard billing claims unless usage is available in the framework event.
- No dashboard UI.
- No per-request reserve fallback for voice sessions. If substrate is unavailable, D41 blocks.

## 4. Public Python surfaces - LOCKED

```python
class SpendGuardVoiceSession:
    async def start(self, *, session_id: str, route: str, estimated_amount_atomic: int) -> None: ...
    async def commit_delta(self, *, amount_atomic_delta: int, reason: str) -> None: ...
    async def release(self, *, reason_code: str) -> None: ...

class SpendGuardLiveKitLLM:
    def __init__(self, *, upstream_llm: object, guard: SpendGuardVoiceSession) -> None: ...

class SpendGuardPipecatLLMService:
    def __init__(self, *, upstream_service: object, guard: SpendGuardVoiceSession) -> None: ...
```

Exact base classes and method signatures are pinned by `V41-V1` and `V41-V2`. If the framework API requires subclassing instead of wrapping, a dated amendment must update this section.

## 5. Lifecycle - LOCKED

```text
voice session start
  -> SpendGuardVoiceSession.start()
     -> reserveSession()
     -> DENY/outage: abort before paid model provider starts

each finalized LLM/STT/TTS usage point
  -> commitSessionDelta(amount > 0, streaming_commit_id)

voice session end/cancel/error
  -> releaseSession(reason_code)

process crash or missing end event
  -> substrate TTL sweep releases remainder
```

Adapters must not open a paid provider connection before `start()` succeeds.

## 6. Unit/pricing threading - LOCKED

Every adapter constructor must accept or derive:

- `tenant_id`
- `budget_id`
- `window_instance_id`
- `unit_id`
- `pricing`
- `route`

Python options use snake_case. Missing `unit_id`, `window_instance_id`, or pricing fields are construction errors for v1. This is stricter than older Python adapters because D41 is new substrate work after HARDEN_D05_UR/WI.

## 7. VERIFY-AT-IMPL marker register

| Marker | Question to pin during implementation | Owning slice |
|---|---|---|
| `V41-V1` | LiveKit Agents exact LLM plugin/wrapper interface and session start hook. | `COV_D41_01_livekit_adapter_skeleton` |
| `V41-V2` | Pipecat exact `FrameProcessor`/`LLMService` interception interface. | `COV_D41_03_pipecat_adapter_skeleton` |
| `V41-V3` | Usage signal shape for LiveKit finalized LLM turns. | `COV_D41_02_livekit_session_gate` |
| `V41-V4` | Usage signal shape for Pipecat finalized LLM turns. | `COV_D41_04_pipecat_session_gate` |
| `V41-V5` | Deterministic local voice demo fixture shape without live audio. | `COV_D41_06_voice_demo_docs` |

## 8. Slice plan

| Slice | Title |
|---|---|
| `COV_D41_01_livekit_adapter_skeleton` | LiveKit package/module skeleton and API pins. |
| `COV_D41_02_livekit_session_gate` | LiveKit start/reserve, deltas, release. |
| `COV_D41_03_pipecat_adapter_skeleton` | Pipecat module skeleton and API pins. |
| `COV_D41_04_pipecat_session_gate` | Pipecat start/reserve, deltas, release. |
| `COV_D41_05_voice_shared_tests` | Shared helper, fail-closed and idempotency tests. |
| `COV_D41_06_voice_demo_docs` | `voice_session_guard` demo, docs, README/CHANGELOG/memory. |

## 9. Definition of done

D41 is shipped when D41 session substrate is already on main, both adapters use it, `voice_session_guard` demo passes, and docs state clearly that voice coverage is session-scoped reservation with streaming commits.

## 10. Dated implementation amendments

### 2026-06-13 - Sidecar bridge prerequisite

The phrase "D41 session substrate is already on main" in §9 now means both:

1. `D41_session_reservation_substrate` direct-ledger substrate is shipped.
2. `D41_sidecar_session_bridge` is shipped, replacing the sidecar UDS
   `UNIMPLEMENTED` session stubs with a real Ledger gRPC bridge.

LiveKit/Pipecat adapters must not start against the fail-closed stubs in
`services/sidecar/src/server/adapter_uds.rs`. Adapter slices may read this
spec and pin framework APIs, but any reserve/delta/release runtime behavior is
blocked until `COV_D41S_06_sidecar_session_bridge` is on main and the
`session_bridge` demo gate passes.
