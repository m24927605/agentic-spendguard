# D26 — Review Standards

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`acceptance.md`](acceptance.md).
**Reviewer:** `superpowers:code-reviewer` skill per `framework-coverage-build-plan-2026-06.md` §1.2. R5 panel escalation per §1.3.

This document is the per-slice checklist the reviewer reads alongside repo coding standards.

## 1. Architectural invariants

### 1.1 Single wrapper covers all providers

- **MUST:** One file `sdk/python/src/spendguard/integrations/letta.py` covers all Letta providers (OpenAI / Anthropic / Google / DeepSeek / etc.) by wrapping `LLMClientBase` polymorphically. The reviewer rejects any PR that creates per-provider subclasses (`SpendGuardOpenAILettaClient`, etc.).
- **MUST:** `letta>=0.8,<1.0` is the only declared dependency in the `[letta]` extra. Pinning specific provider SDKs (e.g. `openai`, `anthropic`) inside the extra is a Blocker — the operator already pulls those transitively via Letta itself.

### 1.2 Composition over inheritance for inner client

- **MUST:** Constructor takes `inner: LLMClientBase`. Wrapper MUST NOT instantiate `OpenAIClient`, `AnthropicClient`, `GoogleAIClient`, or any concrete Letta provider client directly.
- **MUST:** Wrapper does not call `super().__init__()` (ABC takes provider config the wrapper doesn't own). A `super().__init__()` call is a Blocker — silently changes inner-client behavior under upstream refactors.
- **MUST NOT:** Subclass a concrete Letta provider client (`OpenAIClient`, etc.). Subclassing one breaks polymorphism for all others.

### 1.3 `__getattr__` delegation

- **MUST:** `__getattr__` delegates unknown attribute lookups to `self._inner`. This is how `llm_config`, `provider`, `build_request_data`, `convert_response_to_chat_completion`, and any future LLMClientBase additions remain accessible to Letta's `Agent`.
- **MUST:** `__getattr__` only fires on miss (so `_inner`, `_client`, `_budget_id`, etc. shadow correctly). The reviewer reads the impl and verifies no `__getattribute__` override exists.
- **MUST NOT:** Add side effects (logging, metric emission, caching) inside `__getattr__`. The framework calls these methods often; side effects there are a Blocker.

### 1.4 Shared run context

- **MUST:** Import `current_run_context` from `spendguard.integrations.openai_agents`. Duplicating the `RunContext` / `run_context()` definition is a Blocker — polyglot trace sharing breaks if the contextvar identities diverge.
- **MUST NOT:** Modify `openai_agents.py` to support D26 — that file is locked at D-shipped state.

## 2. Gate semantics

### 2.1 Reserve-before-HTTP fail-closed

- **MUST:** `RequestDecision` is awaited BEFORE `self._inner.send_llm_request(...)`. The reviewer reads bytecode order and rejects any reordering.
- **MUST:** DENY decision raises a `SpendGuardDenied` (or upstream sidecar's denial exception) BEFORE any inner-client method is awaited.
- **MUST:** The integration test `test_real_letta_deny_path_zero_provider_http` asserts ZERO HTTP requests reached the inner transport on DENY. Missing this assertion is a Blocker.

### 2.2 POST emission semantics

- **MUST:** `emit_llm_call_post` fires for SUCCESS / FAILURE / CANCELLED branches when `outcome.reservation_ids` is non-empty.
- **MUST:** `CancelledError` detection uses `type(exc).__name__ == "CancelledError"` (matches D12 / D24 pattern) — avoids cross-loop `isinstance` mismatches across `asyncio` / `trio` / `anyio`.
- **MUST:** When no reservation exists (DENY at PRE), POST MUST NOT fire. The test `test_send_llm_request_skips_post_when_no_reservation` enforces this.

### 2.3 Usage extraction precedence

- **MUST:** `_extract_total_tokens` prefers `usage.total_tokens` over `prompt + completion`. Letta's normalization layer populates `total_tokens` for every provider; the prompt+completion fallback handles older Letta versions where the field is absent.
- **MUST:** When `usage is None`, return `0`. Raising or defaulting to a non-zero estimator is a Blocker.

## 3. Sync path safety

### 3.1 No silent `asyncio.run()` inside a loop

- **MUST:** `send_llm_request_sync()` MUST detect an active asyncio loop via `asyncio.get_running_loop()` and raise `RuntimeError` with a message pointing at the async variant.
- **MUST:** The test `test_send_llm_request_sync_inside_running_loop_raises` exists and asserts both `send_llm_request_sync` AND the async-path pointer appear in the message.
- **MUST NOT:** Use `nest_asyncio` or any trick that papers over re-entrancy. Silent re-entry is a Blocker.

## 4. Pass-through completeness

- **MUST:** Every `LLMClientBase` abstract or concrete method except `send_llm_request` / `send_llm_request_sync` is reachable via `__getattr__`. Specifically: `build_request_data`, `convert_response_to_chat_completion`, `llm_config`, `provider`, `model_endpoint_type`, plus any provider-specific helpers.
- **MUST:** Pass-through methods carry no side effects. A counter or timer inside `__getattr__` is a Blocker.

## 5. Demo + docs

### 5.1 Server-mode redirect is load-bearing

- **MUST:** `docs/site/docs/integrations/letta.md` decision table lists the `letta server` row FIRST and points at D02 + D03 as the canonical path. Listing D26 first is a Blocker — it actively misleads ~70% of Letta operators per Trend Researcher 2026-06.
- **MUST:** The page links to D02 closed-CLI install AND D03 base-URL drop-in landing page. Linking only one is a finding.

### 5.2 Demo gate

- **MUST:** `agent_real_letta` demo mode exercises both ALLOW and DENY paths in a single `make demo` invocation. Splitting them across two modes is a finding (not Blocker).
- **MUST:** `verify_step_letta.sql` asserts `LLM_CALL_PRE` + paired `LLM_CALL_POST` for the ALLOW call AND `LLM_CALL_PRE` with `decision='DENY'` + NO paired POST for the DENY call. Missing either assertion is a Blocker.

### 5.3 README adapter table

- **MUST:** Row uses extras label `spendguard-sdk[letta]` exactly and references D26.

## 6. Security review

- **MUST:** No environment variable named `SPENDGUARD_LETTA_FAIL_OPEN` (or similar) is introduced. Fail-closed is the only mode. Any fail-open env knob is a Blocker (parity with D-shipped POC adapters; only D12 inherits fail-open from `litellm.py`).
- **MUST:** Signature derivation uses `repr(request_data) + repr(llm_config) + repr(tools) + repr(force_tool_use)`. The reviewer verifies `llm_config` is included — leaving it out lets a tenant flip model under the same reservation. Missing `llm_config` is a Blocker.
- **MUST:** Signature is computed deterministically (no dict iteration without sorting where dicts appear inside `request_data`). Non-deterministic signatures cause idempotency-key collisions.
- **MUST:** `request_data` is treated as opaque — wrapper MUST NOT log or persist it outside the signature hash. Request payloads can contain user PII (Letta passes full message history into requests).

## 7. Test rigor

### 7.1 Real-Letta integration tests

- **MUST:** Integration tests use `pytest.importorskip("letta", minversion="0.8")`. Hard-failing without Letta installed is a Blocker.
- **MUST:** At least one integration test exercises a real `Agent.step(...)` round trip, not just `wrapped.send_llm_request(...)` directly. The `Agent`-level test catches Letta-side surface changes (e.g. signature drift in 0.9.x).

### 7.2 Coverage matrix completeness

The matrix in `tests.md` §4 MUST be 100% green. Any cell marked as "deferred" or "follow-on" is a Blocker for R1.

### 7.3 Mock layer

- **MUST:** `FakeLLMClient` in `conftest_letta.py` subclasses the real `letta.llm_api.llm_client_base.LLMClientBase` ABC, not a `MagicMock(spec=LLMClientBase)`. Mock-spec is a finding — silently swallows method signature changes upstream.

## 8. Cross-cutting

### 8.1 No regression on shipped adapters

- `tests/integrations/test_openai_agents.py` MUST remain green after D26 lands. Any test failure there is a Blocker — D26 reuses `current_run_context` and MUST NOT modify `openai_agents.py`.

### 8.2 No `letta server` SDK coverage drift

- The docs page MUST point `letta server` operators at D02/D03. The implementation MUST NOT add a `letta-server`-mode code path inside `letta.py`. That's egress-proxy territory.

## 9. R5 panel arbitration triggers

If any of the following stand at R5, escalate to Staff+ panel per `framework-coverage-build-plan-2026-06.md` §1.3:

- Wrapper subclasses a concrete provider client instead of `LLMClientBase`.
- `__getattr__` adds side effects.
- Deny-path test does not assert zero inner HTTP.
- Sync wrapper silently falls back to `asyncio.run()` inside an active loop.
- Docs page leads with D26 over D02/D03 for server-mode users.
- `llm_config` is omitted from the signature.

Panel composition per the build plan: Software Architect (summarizer) + Backend Architect + AI Engineer + Security Engineer + Senior Developer. Materials: this `review-standards.md`, the failing slice diff, the R1-R4 transcripts, and the `tests.md` coverage matrix.
