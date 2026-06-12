# D41 voice adapters - Acceptance Gates

## 1. Prerequisite

| Gate | Command | Pass condition |
|---|---|---|
| A0.1 | `test -f docs/specs/coverage/D41_session_reservation_substrate/design.md` | substrate design exists. |
| A0.2 | substrate closeout evidence from D41S final slice | session reservation demo green before adapter demo is accepted. |

## 2. Unit tests

| Gate | Command | Pass condition |
|---|---|---|
| A1.1 | Python test command for voice helper | exits 0. |
| A1.2 | Python test command for LiveKit adapter | exits 0. |
| A1.3 | Python test command for Pipecat adapter | exits 0. |
| A1.4 | import smoke for `[livekit]` and `[pipecat]` extras | missing extra produces actionable install message. |

## 3. Demo

| Gate | Command | Pass condition |
|---|---|---|
| A2.1 | `make demo-down` | exit 0. |
| A2.2 | `make demo-up DEMO_MODE=voice_session_guard` | prints `[demo] voice_session_guard ALL 4 steps PASS (LIVEKIT + PIPECAT + DENY + ERROR)`. |
| A2.3 | `make -C deploy/demo demo-verify-voice-session-guard` | SQL hard gate exits 0. |
| A2.4 | DENY step provider-stub counter | unchanged across denied session. |

## 4. Docs and closeout

| Gate | Command | Pass condition |
|---|---|---|
| A3.1 | docs-site build command | exits 0. |
| A3.2 | `rg -n "session-scoped reservation|streaming commit" docs/site-v2/src/content/docs/docs/integrations/voice-livekit-pipecat.mdx` | voice docs state the substrate model. |
| A3.3 | `rg -n "LiveKit|Pipecat" README.md CHANGELOG.md` | rows/entries present. |
| A3.4 | memory file exists | `project_coverage_d41_shipped.md` exists after closeout. |

## 5. Ship checklist

- [ ] D41S substrate closed first.
- [ ] `V41-V1`..`V41-V5` pinned.
- [ ] Live demo physically run after `make demo-down`.
- [ ] No per-request fallback invented for voice sessions.
- [ ] Docs distinguish LiveKit hosted inference billing from SpendGuard self-hosted hard cap.
