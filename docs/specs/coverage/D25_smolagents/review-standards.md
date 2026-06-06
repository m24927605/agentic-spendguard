# D25 — Review Standards

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`acceptance.md`](acceptance.md).
**Reviewer:** `superpowers:code-reviewer` skill per `framework-coverage-build-plan-2026-06.md` §1.2. R5 panel escalation per §1.3.

This document is the per-slice checklist the reviewer reads alongside repo coding standards.

## 1. Architectural invariants

### 1.1 Single-module wrap surface

- **MUST:** One file `sdk/python/src/spendguard/integrations/smolagents.py` covers ALL three direct-wrap inner Model classes (`InferenceClientModel`, `OpenAIServerModel`, `TransformersModel`). The reviewer rejects any PR that creates per-vendor wrapper subclasses (e.g. `SpendGuardInferenceClientModel`).
- **MUST:** `smolagents>=1.5` is the only declared dependency in the `[smolagents]` extra. Pinning a vendor backend (e.g. `huggingface_hub`, `openai`, `transformers`) is a Blocker — `smolagents` itself declares those as appropriate extras and the operator chooses.
- **MUST NOT:** Wrap `LiteLLMModel`. D12 SDK shim is the canonical path. A `SpendGuardSmolModel` wrapping `LiteLLMModel` would double-gate every call (D12 fires PRE first, then D25 fires PRE again on the SmolAgents call boundary, producing two reservations per call). Documented as a "do not do" in the docs page; reviewer rejects examples that wrap `LiteLLMModel`.

### 1.2 Composition over inheritance for inner Model

- **MUST:** Constructor takes `inner: smolagents.Model`. Wrapper MUST NOT instantiate `InferenceClient`, `openai.OpenAI`, or `transformers.AutoModelForCausalLM` directly.
- **MUST:** Wrapper does not call `super().__init__()`. `smolagents.Model.__init__` sets attributes used only by vendor subclasses; calling super would force a synthetic `model_id` and break inner introspection. A `super().__init__()` call is a Blocker.
- **MUST NOT:** Subclass a concrete vendor Model (`InferenceClientModel`, `OpenAIServerModel`, `TransformersModel`). Subclassing locks the wrapper to one vendor's lifecycle quirks and creates polymorphism breakage.

### 1.3 Shared run context

- **MUST:** Import `current_run_context` from `spendguard.integrations.openai_agents`. Duplicating the `RunContext` / `run_context()` definition is a Blocker — polyglot trace sharing breaks if the contextvar identities diverge.
- **MUST NOT:** Modify `openai_agents.py` to support D25 — that file is locked at D-shipped state. Any change there is out of scope and rejected at R1.

## 2. Gate semantics

### 2.1 Reserve-before-HTTP fail-closed

- **MUST:** `RequestDecision` is awaited BEFORE `self._inner.generate(...)`. The reviewer reads the bytecode order and rejects any reordering.
- **MUST:** DENY decision raises a `SpendGuardDenied` (or upstream sidecar's denial exception) BEFORE any inner-model method is awaited.
- **MUST:** The integration test `test_code_agent_deny_path` asserts ZERO HTTP requests reached the inner transport on DENY. Missing this assertion is a Blocker.

### 2.2 POST emission semantics

- **MUST:** `emit_llm_call_post` fires for SUCCESS / FAILURE / CANCELLED branches when `outcome.reservation_ids` is non-empty.
- **MUST:** `CancelledError` detection uses `type(exc).__name__ == "CancelledError"` (matches D12 / D24 patterns) — avoids cross-loop `isinstance` mismatches across `asyncio` / `trio` / `anyio`.
- **MUST:** When no reservation exists (DENY at PRE), POST MUST NOT fire. The test `test_generate_skips_post_when_no_reservation` enforces this.

### 2.3 Usage extraction

- **MUST:** `_extract_total_tokens` reads `usage.input_tokens + usage.output_tokens` from `smolagents.models.TokenUsage`. Reading any other field (e.g. `total_tokens` which does NOT exist on `TokenUsage` as of `smolagents 1.5`) is a Blocker.
- **MUST:** When `token_usage is None`, return `0`. Raising or defaulting to a non-zero estimator is a Blocker.

## 3. `__call__` alias

### 3.1 Version-drift bypass guard

- **MUST:** `SpendGuardSmolModel.__call__` exists and delegates to `self.generate(...)`. `smolagents<1.5` agents invoke `model(messages, ...)`; without the alias they would silently bypass the gate while passing static-type checks.
- **MUST:** The two alias tests (`test_call_alias_routes_through_generate`, `test_call_alias_propagates_kwargs`) are present and not marked `skip`/`xfail`.
- **MUST NOT:** Implement `__call__` as a separate code path duplicating the PRE/POST logic. Duplication is a finding — the alias must funnel into `generate`.

## 4. `step_callbacks` helper safety

### 4.1 Informational only, never gates

- **MUST:** `spendguard_step_callback()` MUST NOT call `client.request_decision`. The callable's only sidecar interaction is `client.emit_agent_step_telemetry` (or the fallback `emit_custom_audit`). Any decision RPC inside the callback is a Blocker — it would fire AFTER the step's LLM call already completed, which is a wrong-time gate.
- **MUST:** Docstring states explicitly: "NOT a gating surface — step_callbacks fire AFTER each step completes."

### 4.2 Exception swallow is load-bearing

- **MUST:** The callable's top-level `try`/`except Exception` MUST catch every exception, log via `logging.warning`, and return `None`. A raise would abort the host agent run — sidecar outage during telemetry would corrupt the user's agent state.
- **MUST:** Catch `Exception`, NOT `BaseException`. `KeyboardInterrupt` / `SystemExit` MUST propagate. Catching `BaseException` is a Blocker.
- **MUST:** Test `test_step_callback_swallows_exceptions` asserts the callable returns `None` when the sidecar telemetry call raises.

## 5. `__getattr__` forward

### 5.1 Resilience without leaking private state

- **MUST:** `__getattr__` returns `self._inner` attributes for non-private names; `name.startswith("_")` MUST raise `AttributeError`. Leaking `_inner` access via `wrapper._inner` (no AttributeError on `_`) is a finding.
- **MUST:** `__getattr__` MUST NOT shadow `generate` / `__call__` / `_extract_total_tokens`. The reviewer checks the resolution order: explicit methods on `SpendGuardSmolModel` always win; `__getattr__` only fires when the attribute is genuinely missing.

## 6. Demo + docs

- **MUST:** `agent_real_smolagents` demo mode uses `OpenAIServerModel` against the existing `deploy/demo/wiremock/openai/` mappings — no new mock infrastructure. New mock files are a finding.
- **MUST:** The docs page shows the SAME `SpendGuardSmolModel(...)` construction for all three direct-wrap inner Model classes. The only difference between code samples is the inner constructor.
- **MUST:** The docs page contains an explicit "do not wrap `LiteLLMModel`" warning + link to D12.
- **MUST:** README adapter table row uses extras label `spendguard-sdk[smolagents]` exactly.

## 7. Security review

- **MUST:** No environment variable named `SPENDGUARD_SMOLAGENTS_FAIL_OPEN` is introduced. Fail-closed is the only mode. Any fail-open env knob is a Blocker (parity with D24 and D-shipped POC adapters; only D12 inherits fail-open from `litellm.py`).
- **MUST:** `kwargs` is included in the signature via `repr(sorted(kwargs.items()))`. Sorting is required for determinism — unsorted dict items is a finding.
- **MUST:** The signature is derived from `repr(messages) + repr(stop_sequences) + repr(response_format) + repr(tools_to_call_from) + repr(sorted(kwargs.items()))`. Omitting any of the five components is a Blocker — operator-chosen routing flags (e.g. `temperature`, `max_tokens`) MUST affect the reservation key.

## 8. Test rigor

### 8.1 Parametrization

- **MUST:** Integration tests use `pytest.mark.parametrize` or fixture `params=[...]` keyed on an `INNERS` list (`inference_client`, `openai_server`). Hard-coding to one inner is a Blocker.
- **MUST:** `pytest.mark.skipif(not INNERS, ...)` guards CI runs without `smolagents` installed.

### 8.2 Coverage matrix completeness

The matrix in `tests.md` §4 MUST be 100% green. Any cell marked as "deferred" or "follow-on" is a Blocker for R1.

### 8.3 Mock layer

- **MUST:** `FakeSmolModel` in `conftest_smolagents.py` subclasses the real `smolagents.Model` ABC, not a mock spec. Using `MagicMock(spec=Model)` is a finding — silently swallows method signature changes upstream.

## 9. Cross-cutting

### 9.1 No regression on shipped adapters

- `tests/integrations/test_openai_agents.py` MUST remain green after D25 lands. Any test failure there is a Blocker — D25 reuses `current_run_context` and MUST NOT modify `openai_agents.py`.

### 9.2 D12 documented as transitive path for `LiteLLMModel`

- The integration docs page MUST link to D12's docs page for the LiteLLM-routed transitive coverage case AND explicitly recommend NOT wrapping `LiteLLMModel` with `SpendGuardSmolModel`. Omitting either is a finding.

### 9.3 Polyglot trace integration

- One integration test MUST demonstrate a polyglot run (SmolAgents `CodeAgent` + `openai_agents.Agent` step) sharing one `run_context()`, with both producing audit rows under the same `run_id`. Absence is a finding (we ship this guarantee in `openai_agents.py`'s docstring).

## 10. R5 panel arbitration triggers

If any of the following stand at R5, escalate to Staff+ panel per `framework-coverage-build-plan-2026-06.md` §1.3:

- Wrapper instantiates a vendor SDK directly (`InferenceClient`, `openai.OpenAI`, `transformers.*`).
- Deny-path test does not assert zero inner HTTP.
- `__call__` alias missing or duplicates PRE/POST logic.
- `step_callbacks` helper calls `request_decision`, OR raises out of the callable.
- `LiteLLMModel` wrap example present in docs / tests / demo.
- Per-vendor subclass module split (`smolagents_inference_client.py`, etc.).

Panel composition per the build plan: Software Architect (summarizer) + Backend Architect + AI Engineer + Security Engineer + Senior Developer. Materials: this `review-standards.md`, the failing slice diff, the R1-R4 transcripts, and the `tests.md` coverage matrix.
