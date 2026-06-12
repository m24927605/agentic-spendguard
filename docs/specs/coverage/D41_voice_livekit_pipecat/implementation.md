# D41 voice adapters - Implementation

## 1. File layout

```text
sdk/python/src/spendguard/integrations/voice/
  __init__.py
  _session.py
  _options.py
  _usage.py
sdk/python/src/spendguard/integrations/livekit_agents/
  __init__.py
  _llm.py
sdk/python/src/spendguard/integrations/pipecat/
  __init__.py
  _llm_service.py
sdk/python/tests/integrations/
  test_voice_session.py
  test_livekit_agents.py
  test_pipecat_voice.py
deploy/demo/voice_session_guard/
  docker-compose.yaml
  driver.py
  fixtures/
deploy/demo/verify_step_voice_session_guard.sql
docs/site-v2/src/content/docs/docs/integrations/voice-livekit-pipecat.mdx
```

## 2. Shared helper

`SpendGuardVoiceSession` wraps the D41 substrate SDK:

- validates construction tuple
- reserves session before paid provider connection
- assigns monotonic `streaming_commit_id`
- rejects zero/negative deltas
- retries bounded transient commit failures
- releases on normal end/cancel/error

## 3. LiveKit adapter

The first slice pins the exact LiveKit interface. The intended control flow:

```python
guard = SpendGuardVoiceSession(...)
llm = SpendGuardLiveKitLLM(upstream_llm=real_llm, guard=guard)
session = AgentSession(llm=llm, ...)
```

If LiveKit requires subclassing a plugin type, `SpendGuardLiveKitLLM` becomes that subclass by dated amendment.

## 4. Pipecat adapter

The first Pipecat slice pins whether the correct seam is an `LLMService` wrapper, a `FrameProcessor`, or both. The intended control flow:

```python
guard = SpendGuardVoiceSession(...)
llm_service = SpendGuardPipecatLLMService(upstream_service=real_service, guard=guard)
pipeline = Pipeline([... , llm_service, ...])
```

## 5. Demo

`DEMO_MODE=voice_session_guard` runs local deterministic fixtures:

1. LiveKit-style session starts, commits two deltas, releases.
2. Pipecat-style session starts, commits two deltas, releases.
3. Denied session never calls local paid-provider stub.
4. Provider error commits/release path is visible.

Locked success line:

```text
[demo] voice_session_guard ALL 4 steps PASS (LIVEKIT + PIPECAT + DENY + ERROR)
```

## 6. Slice to file map

| Slice | Files |
|---|---|
| `COV_D41_01_livekit_adapter_skeleton` | LiveKit module skeleton, extras, V41-V1 pin. |
| `COV_D41_02_livekit_session_gate` | LiveKit reserve/delta/release implementation. |
| `COV_D41_03_pipecat_adapter_skeleton` | Pipecat module skeleton, extras, V41-V2 pin. |
| `COV_D41_04_pipecat_session_gate` | Pipecat reserve/delta/release implementation. |
| `COV_D41_05_voice_shared_tests` | Shared helper and full unit tests. |
| `COV_D41_06_voice_demo_docs` | Demo overlay, verify SQL, docs, memory. |
