# D22 — Review standards (R1-R5 reviewer checklist)

Used verbatim by `superpowers:code-reviewer` for every D22 slice. Findings are categorized as **Blocker** (R1 fail) / **Major** (R1 fail) / **Minor** (advisory, can ship with residual issue).

## 1. Architectural fidelity (Blocker if violated)

- [ ] **Callable factory shape preserved.** `SpendGuardAgnoPreHook(...)` and `SpendGuardAgnoPostHook(...)` are factory objects whose `__call__()` returns the async hook callable. They are NOT subclasses of any Agno `Model`. PR diffs that introduce model-subclassing fail this gate.
- [ ] **Hook callable parameter names are literally `(agent, run_input)` for pre and `(agent, run_response)` for post.** Agno injects via `inspect.signature` parameter NAMES. Closure tricks (`functools.wraps`, lambda renaming, `*args, **kwargs` pass-through) that change visible parameter names are blockers. Test #16 / #17 enforce this.
- [ ] **Shared `_RUN_CONTEXT` contextvar** uses the same module variable name (`spendguard_run_context`) as `langchain.py:86`, `pydantic_ai.py`, `openai_agents.py:86`. A new context-var name is a Blocker — it breaks multi-framework `run_id` sharing.
- [ ] **No new sidecar surface area.** The adapter calls only `client.request_decision(...)` and `client.emit_llm_call_post(...)` — same two methods used by `langchain.py`. New client methods or proto changes are Blockers (would force a sidecar release).

## 2. Idempotency + identifier derivation (Blocker)

- [ ] `llm_call_id` is derived via `derive_uuid_from_signature(signature, scope="llm_call_id")` — never `new_uuid7()` in the hot path (would break retry determinism).
- [ ] `decision_id` is derived from the same signature with `scope="decision_id"`.
- [ ] `idempotency_key` is built via `derive_idempotency_key(...)` with **all six** kwargs (tenant_id / session_id / run_id / step_id / llm_call_id / trigger="LLM_CALL_PRE"). Missing any kwarg is a Blocker.
- [ ] Default `_default_call_signature` hashes `agent.model.id` PLUS `run_input`. Hashing run_input alone is a Major (collisions across models in the same run).

## 3. Lifecycle correctness (Blocker)

- [ ] **PRE before vendor SDK.** The pre-hook MUST be `await`ed before Agno's `agent.model.<provider>_client.chat.completions.create(...)` is reached. Verified by integration test `test_real_agent_deny_short_circuits` — if the stub OpenAI is invoked on a STOP, this is a Blocker.
- [ ] **POST after vendor response** with real usage. The post-hook reads `run_response.metrics.total_tokens` and emits `outcome="SUCCESS"`.
- [ ] **`outcome="PROVIDER_ERROR"` on RunError or missing metrics.** Forgetting this path (i.e. silently no-op on error) is a Blocker — the reservation would leak in the ledger.
- [ ] **Inflight slot lifecycle**: pre populates, post pops. Post finding no slot logs and no-ops (never commits without a reserve). Pre evicting on bound is FIFO.
- [ ] Streaming path commits POST exactly ONCE on the completion event, not per chunk.

## 4. Memory + concurrency safety (Blocker)

- [ ] Inflight map is bounded (`_INFLIGHT_MAX = 10_000`). Unbounded growth is a Blocker.
- [ ] Inflight map is `collections.OrderedDict` so FIFO eviction is deterministic. `dict` + iteration order assumption is a Major.
- [ ] Module-shared inflight is acceptable because pre/post pairs are constructed within one process; cross-process sharing is explicitly out of scope. Reviewer accepts the comment but flags any attempt to back the map with a global mutex / threading lock (asyncio single-loop guarantee covers this) as Minor.

## 5. Default estimator integration (Major)

- [ ] `agno_default_claim_estimator` lives in `_default_estimator.py` alongside the other four factories. New file is Major (regression — the helper must be discoverable from one file).
- [ ] Estimator resolves `agent.model.id` at CALL time (closure), not construction time. Otherwise `Team` agents with multi-model dispatch get wrong tokenizer.
- [ ] Estimator handles `run_input` of three shapes: `str`, `list`, other (str-coerce). Missing any branch is Major.
- [ ] Estimator respects `_resolve_max_tokens` style fall-through OR explicitly documents `None` cap (family default). Hard-coding a default integer is Major.

## 6. Public surface stability (Blocker for changes after slice 2)

- [ ] `__all__` lists: `ClaimEstimator`, `CallSignatureFn`, `RunContext`, `SpendGuardAgnoPreHook`, `SpendGuardAgnoPostHook`, `current_run_context`, `run_context`. Removing or renaming after slice 2 ships is a Blocker.
- [ ] Constructor kwargs (Major if changed after slice 2): `client`, `budget_id`, `window_instance_id`, `unit`, `pricing`, `claim_estimator`, `call_signature_fn`, `route`, `inflight`.
- [ ] No required positional arg is added — every parameter is keyword-only via `*,`.

## 7. Demo + docs (Blocker for slice 4)

- [ ] `agent_real_agno` is registered in `deploy/demo/demo/run_demo.py` mirroring the existing `agent_real_langchain` branch structure (no novel control flow).
- [ ] Demo invokes the real `agno.agent.Agent` against a real `OpenAIChat` model (in CI: real OpenAI key OR recorded fixture from `examples/agno-prehooks/fixtures/`). MockLLM-only demo is a Blocker (per project memory `feedback_demo_quality_gate`).
- [ ] Deny path (`DENY=1`) is wired through the same script; exit code 13 is non-negotiable.
- [ ] Integration page `docs/site/docs/integrations/agno.md` exists and is referenced from the sidebar config.
- [ ] README adapter table row pins `agno >= 1.0` matching `pyproject.toml`.

## 8. Anti-patterns (auto-flag as findings)

- [ ] Any `print(...)` inside `agno.py` (use `logger.warning`).
- [ ] Any `asyncio.create_task(...)` for the reserve call (must be `await`ed).
- [ ] Any catch-all `except Exception:` around `client.request_decision` that swallows `DecisionDenied`.
- [ ] Any commit that adds `import openai` / `import anthropic` to `agno.py` (provider SDKs are user-supplied; the adapter is provider-agnostic).
- [ ] Any tests that mock Agno's `Agent.run` directly (must mock the inner Model's vendor client, then run the real `Agent.arun` — otherwise the test does not exercise hook injection).

## 9. Regressions that must NOT happen

- [ ] LangChain / Pydantic-AI / OpenAI Agents / AGT integration test suites all still PASS.
- [ ] `_default_estimator.py` existing factories are byte-identical (audit: `git diff --stat src/spendguard/integrations/_default_estimator.py` shows additions only).
- [ ] `pyproject.toml` other extras unchanged (audit: only the `agno = [...]` block added).
- [ ] Wheel size delta ≤ +5 KB.

## 10. Memory citations

Reviewer checks code comments for memory-bank citations. Acceptable: free-form references to LangChain's `claim_estimator` cascade, `derive_*` helper docstrings. Unacceptable: fabricated commit SHAs / "per spec §X.Y" pointers to non-existent sections. The `project_slice_05_shipped` lesson — fabricated memory citations are a Blocker on R1 — applies to D22.

## 11. R5 panel escalation matrix

If R5 still has open findings, panel composition follows `framework-coverage-build-plan-2026-06.md` §1.3 (Software Architect / Backend Architect / AI Engineer / Security Engineer / Senior Developer). Summarizer defaults to AI Engineer for D22 (per the build-plan owner column). Panel materials MUST include: the slice diff, this `review-standards.md`, the corresponding section of `acceptance.md`, the failing R1-R5 review records, and the integration test logs.
