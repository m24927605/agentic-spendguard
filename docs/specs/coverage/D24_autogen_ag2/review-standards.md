# D24 — Review Standards

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`acceptance.md`](acceptance.md).
**Reviewer:** `superpowers:code-reviewer` skill per `framework-coverage-build-plan-2026-06.md` §1.2. R5 panel escalation per §1.3.

This document is the per-slice checklist the reviewer reads alongside repo coding standards.

## 1. Architectural invariants

### 1.1 Single-module dual-lineage coverage

- **MUST:** One file `sdk/python/src/spendguard/integrations/autogen.py` covers BOTH AutoGen 0.4+ and AG2. The reviewer rejects any PR that creates a separate `ag2.py` module or duplicates the wrapper class.
- **MUST:** `LINEAGE` probe is telemetry-only. Business logic in `create()` / `create_stream()` MUST NOT branch on `LINEAGE`. Any conditional on `LINEAGE` other than logging / metric labels is a Blocker finding.
- **MUST:** `autogen-core>=0.4` is the only declared dependency in the `[autogen]` extra. Pinning `autogen-agentchat` or `ag2` is a Blocker — the operator picks the lineage.

### 1.2 Composition over inheritance for inner client

- **MUST:** Constructor takes `inner: ChatCompletionClient`. Wrapper MUST NOT instantiate `OpenAIChatCompletionClient`, `AnthropicChatCompletionClient`, or any vendor SDK directly.
- **MUST:** Wrapper does not call `super().__init__()` (ABC has no shared state). A `super().__init__()` call is a Blocker — silently changes inner-client behavior under upstream refactors.
- **MUST NOT:** Subclass `OpenAIChatCompletionClient` or any concrete vendor client. Subclassing breaks AG2 polymorphism guarantees and creates duplicate-effort maintenance.

### 1.3 Shared run context

- **MUST:** Import `current_run_context` from `spendguard.integrations.openai_agents`. Duplicating the `RunContext` / `run_context()` definition is a Blocker — polyglot trace sharing breaks if the contextvar identities diverge.
- **MUST NOT:** Modify `openai_agents.py` to support D24 — that file is locked at D-shipped state. Any change there is out of scope and rejected at R1.

## 2. Gate semantics

### 2.1 Reserve-before-HTTP fail-closed

- **MUST:** `RequestDecision` is awaited BEFORE `self._inner.create(...)`. The reviewer reads the bytecode order and rejects any reordering.
- **MUST:** DENY decision raises a `SpendGuardDenied` (or upstream sidecar's denial exception) BEFORE any inner-client method is awaited.
- **MUST:** The integration test `test_assistant_agent_deny_path` asserts ZERO HTTP requests reached the inner transport on DENY. Missing this assertion is a Blocker.

### 2.2 POST emission semantics

- **MUST:** `emit_llm_call_post` fires for SUCCESS / FAILURE / CANCELLED branches when `outcome.reservation_ids` is non-empty.
- **MUST:** `CancelledError` detection uses `type(exc).__name__ == "CancelledError"` (matches D12 shim pattern) — avoids cross-loop `isinstance` mismatches across `asyncio` / `trio` / `anyio`.
- **MUST:** When no reservation exists (DENY at PRE), POST MUST NOT fire. The test `test_create_skips_post_when_no_reservation` enforces this.

### 2.3 Usage extraction

- **MUST:** `_extract_total_tokens` reads `usage.prompt_tokens + usage.completion_tokens` from `autogen_core.models.RequestUsage`. Reading any other field (e.g. `total_tokens` which doesn't exist on `RequestUsage` in either lineage) is a Blocker.
- **MUST:** When `usage is None`, return `0`. Raising or defaulting to a non-zero estimator is a Blocker.

## 3. Stream behavior

### 3.1 POC scope is explicit

- **MUST:** `create_stream()` is documented as pass-through with PRE/POST firing at the next `create()` boundary. Inline comment matches the OpenAI Agents adapter's docstring on `stream_response`.
- **MUST:** Test `test_create_stream_does_not_call_request_decision` exists and is NOT marked `skip` or `xfail`. The behavior is intentional; the test asserts it.
- **MUST NOT:** Add per-chunk gating in this deliverable. That's tracked as a follow-on; any half-implementation is a Blocker.

## 4. Pass-through completeness

- **MUST:** Every `ChatCompletionClient` abstract or concrete method except `create` is pass-through. Specifically: `create_stream`, `actual_usage`, `total_usage`, `count_tokens`, `remaining_tokens`, `capabilities`, `model_info`.
- **MUST:** Pass-through methods carry no side effects (no sidecar calls, no caching, no metric emission). A counter or timer is a Blocker — they confuse `AssistantAgent`'s token-budget caps.

## 5. Demo + docs

- **MUST:** Both `agent_real_autogen` and `agent_real_ag2` demo modes share a single `verify_step_autogen.sql`. Two parallel verifiers is a finding (not Blocker).
- **MUST:** The docs page shows the SAME `SpendGuardChatCompletionClient(...)` construction for both lineages. The only difference between the AutoGen and AG2 code samples is the `AssistantAgent` import path.
- **MUST:** README adapter table row uses extras label `spendguard-sdk[autogen]` exactly (not `[ag2]`, not `[autogen-ag2]`).

## 6. Security review

- **MUST:** No environment variable named `SPENDGUARD_AUTOGEN_FAIL_OPEN` is introduced. Fail-closed is the only mode. Any fail-open env knob is a Blocker (parity with D-shipped POC adapters; only D12 inherits fail-open from `litellm.py`).
- **MUST:** `extra_create_args` is shallow-copied (`dict(extra_create_args or {})`) before being included in the signature. Including the operator's mutable dict in the signature without copying creates a TOCTOU between PRE and inner call. Missing copy is a Blocker.
- **MUST:** The signature is derived from `repr(messages) + repr(tools) + repr(sorted(extra.items()))`. Sorting is required for determinism. Unsorted dict items is a finding.

## 7. Test rigor

### 7.1 Parametrization

- **MUST:** Integration tests use `pytest.mark.parametrize` or fixture `params=[...]` keyed on a `LINEAGES` list. Hard-coding to `autogen` only with no AG2 coverage is a Blocker.
- **MUST:** `pytest.mark.skipif(not LINEAGES, ...)` guards CI runs without either package installed.

### 7.2 Coverage matrix completeness

The matrix in `tests.md` §4 MUST be 100% green. Any cell marked as "deferred" or "follow-on" is a Blocker for R1.

### 7.3 Mock layer

- **MUST:** `FakeChatCompletionClient` in `conftest_autogen.py` subclasses the real `autogen_core.models.ChatCompletionClient` ABC, not a mock spec. Using `MagicMock(spec=ChatCompletionClient)` is a finding — silently swallows method signature changes upstream.

## 8. Cross-cutting

### 8.1 No regression on shipped adapters

- `tests/integrations/test_openai_agents.py` MUST remain green after D24 lands. Any test failure there is a Blocker — D24 reuses `current_run_context` and MUST NOT modify `openai_agents.py`.

### 8.2 D12 documented as transitive path

- The integration docs page MUST link to D12's docs page for the LiteLLM-routed transitive coverage case. Omitting this link is a finding.

## 9. R5 panel arbitration triggers

If any of the following stand at R5, escalate to Staff+ panel per `framework-coverage-build-plan-2026-06.md` §1.3:

- LINEAGE probe branches business logic anywhere.
- Wrapper instantiates a vendor SDK directly.
- Deny-path test does not assert zero inner HTTP.
- Stream behavior is partially gated (per-chunk attempted but incomplete).
- AG2 lineage tests are skipped without an installed-package justification.

Panel composition per the build plan: Software Architect (summarizer) + Backend Architect + AI Engineer + Security Engineer + Senior Developer. Materials: this `review-standards.md`, the failing slice diff, the R1-R4 transcripts, and the `tests.md` coverage matrix.
