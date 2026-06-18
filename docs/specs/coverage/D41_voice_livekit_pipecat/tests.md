# D41 voice adapters - Tests

## 1. Shared voice session tests

| ID | Test | Verifies |
|---|---|---|
| TP-D41-01 | Missing `unit_id`, `window_instance_id`, or pricing rejects construction. | Day-1 tuple discipline. |
| TP-D41-02 | `start()` calls substrate `reserve_session` before provider start callback. | Fail-closed start. |
| TP-D41-03 | Reserve DENY aborts and paid-provider stub is not called. | Hard gate. |
| TP-D41-04 | `commit_delta()` rejects zero and negative deltas. | Positive delta invariant. |
| TP-D41-05 | Commit deltas use monotonic idempotency keys and replay cleanly. | Reconnect safety. |
| TP-D41-06 | `release()` is idempotent and sends reason code. | Settlement. |

## 2. LiveKit tests

| ID | Test | Verifies |
|---|---|---|
| TP-D41-10 | LiveKit wrapper satisfies pinned interface from `V41-V1`. | Type/API conformance. |
| TP-D41-11 | LiveKit session start reserves before upstream LLM connection. | Fail-closed. |
| TP-D41-12 | LiveKit finalized usage emits positive deltas per `V41-V3`. | Streaming commit. |
| TP-D41-13 | LiveKit provider error releases/settles and rethrows. | Failure path. |

## 3. Pipecat tests

| ID | Test | Verifies |
|---|---|---|
| TP-D41-20 | Pipecat wrapper satisfies pinned interface from `V41-V2`. | Type/API conformance. |
| TP-D41-21 | Pipecat pipeline start reserves before upstream LLM service call. | Fail-closed. |
| TP-D41-22 | Pipecat finalized usage emits positive deltas per `V41-V4`. | Streaming commit. |
| TP-D41-23 | Pipecat provider error releases/settles and rethrows. | Failure path. |

## 4. Acceptance gates

| ID | Command | Pass condition |
|---|---|---|
| TA-D41-00 | D41 bridge gates TA-D41B-01..06 from `docs/specs/coverage/D41_sidecar_session_bridge/tests.md` | full bridge prerequisite is green before adapter runtime tests are accepted. |
| TA-D41-01 | `PYTHONPATH=sdk/python/src python3.11 -m pytest sdk/python/tests/integrations/test_voice_session.py -q` | exit 0. |
| TA-D41-02 | `PYTHONPATH=sdk/python/src python3.11 -m pytest sdk/python/tests/integrations/test_livekit_agents.py sdk/python/tests/integrations/test_pipecat_voice.py -q` | no new regressions. |
| TA-D41-03 | `make demo-down` | exits 0. |
| TA-D41-04 | `make demo-up DEMO_MODE=voice_session_guard` | prints locked success line. |
| TA-D41-05 | `make -C deploy/demo demo-verify-voice-session-guard` | SQL hard gate exits 0. |
| TA-D41-06 | `pnpm -C docs/site-v2 run build` | exits 0. |

## 5. Slice mapping

| Slice | Tests |
|---|---|
| `COV_D41_01_livekit_adapter_skeleton` | TP-D41-10 skeleton |
| `COV_D41_02_livekit_session_gate` | TP-D41-11..13 |
| `COV_D41_03_pipecat_adapter_skeleton` | TP-D41-20 skeleton |
| `COV_D41_04_pipecat_session_gate` | TP-D41-21..23 |
| `COV_D41_05_voice_shared_tests` | TP-D41-01..06 plus all adapter unit tests |
| `COV_D41_06_voice_demo_docs` | TA-D41-00..06 |
