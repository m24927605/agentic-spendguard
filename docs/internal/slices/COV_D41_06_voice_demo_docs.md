# COV_D41_06 - Voice demo, docs, and closeout

> **Deliverable:** D41 LiveKit Agents + Pipecat voice adapters
> **Slice:** 6 of 6
> **Spec set:** [`docs/specs/coverage/D41_voice_livekit_pipecat/`](../../specs/coverage/D41_voice_livekit_pipecat/)
> **Precedence:** substrate and adapter designs are LOCKED.

## Scope

Add deterministic local demo mode `voice_session_guard`, hard verify SQL, docs site page, README/CHANGELOG updates, and memory closeout.

## LOCKED design quotes

From adapter `implementation.md` §5:

> Locked success line:
>
> `[demo] voice_session_guard ALL 4 steps PASS (LIVEKIT + PIPECAT + DENY + ERROR)`

From adapter `acceptance.md` §5:

> D41S substrate closed first.
>
> D41 sidecar-to-ledger bridge closed before adapter runtime behavior.
>
> No per-request fallback invented for voice sessions.

## Files touched

| File | Why |
|---|---|
| `deploy/demo/voice_session_guard/*` | Demo overlay, fixtures, driver. |
| `deploy/demo/verify_step_voice_session_guard.sql` | Hard SQL gate. |
| `deploy/demo/Makefile` | Demo mode and verify target. |
| `docs/site-v2/src/content/docs/docs/integrations/voice-livekit-pipecat.mdx` | Docs page. |
| `README.md` | Adapter rows. |
| `CHANGELOG.md` | D41 entry. |
| memory files | Closeout. |

## VERIFY-AT-IMPL pins

Pin `V41-V5`; confirm `V41-V1`..`V41-V5` all closed.

## Test/verification plan

- `make demo-down`
- `make demo-up DEMO_MODE=voice_session_guard`
- `make -C deploy/demo demo-verify-voice-session-guard`
- TA-D41-01..06.

## Anti-scope

- No new substrate semantics.
- No sidecar-to-ledger bridge work; that belongs to `COV_D41S_06`.
- No real microphone/browser/live provider requirement for hard gate.
