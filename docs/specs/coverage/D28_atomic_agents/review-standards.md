# D28 — Review Standards

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`acceptance.md`](acceptance.md).
**Reviewer:** `superpowers:code-reviewer` skill per `framework-coverage-build-plan-2026-06.md` §1.2. R5 panel escalation per §1.3.

Per-slice checklist read alongside repo coding standards.

## 1. Architectural invariants

### 1.1 Wrap the Instructor object, not the raw provider SDK

- **MUST:** `wrap_instructor_client(...)` accepts only `instructor.Instructor` or `instructor.AsyncInstructor`. Accepting a bare `openai.OpenAI` / `anthropic.Anthropic` is a **Blocker** — that path silently undercounts Instructor validation retries (rejected alternative per `design.md` §1).
- **MUST:** The unit test `test_wrap_instructor_client_rejects_raw_openai_client` AND the integration test `test_raw_openai_wrap_rejected_by_factory` both exist. Either missing is a Blocker.
- **MUST:** `TypeError` message points the operator at `instructor.from_openai(...)`. A bare `TypeError("invalid client")` is a finding.

### 1.2 Composition + `__getattr__`, no subclass of `Instructor`

- **MUST:** `SpendGuardInstructorProxy` and `SpendGuardAsyncInstructorProxy` inherit from `_ProxyBase` (or plain `object`). Inheriting from `instructor.Instructor` / `instructor.AsyncInstructor` is a Blocker — those classes use `__init_subclass__` magic and accept private kwargs that will break under upstream churn.
- **MUST:** Constructor does NOT call `instructor.Instructor.__init__` or `instructor.AsyncInstructor.__init__`.
- **MUST:** `__getattr__` delegates unknown attribute lookups to `self._inner`. Required so `proxy.mode`, `proxy.create_kwargs`, and any future Instructor attrs remain reachable.
- **MUST NOT:** Add side effects (logging, metrics, caching) inside `__getattr__`. Blocker.

### 1.3 Both `create` and `create_with_completion` covered

- **MUST:** Sync proxy's `chat.completions.create` AND `chat.completions.create_with_completion` route through `_sync_gated_call`. Same for the async proxy.
- **MUST:** `create_with_completion` path unpacks `(parsed, raw_completion)` and reads usage from `raw_completion`.
- **MUST:** `create` path reads usage from `parsed._raw_response` (Instructor's documented private attr). Falling through to a zero-cost POST without consulting `_raw_response` is a Blocker — undercount path.

### 1.4 Sync vs async dispatch

- **MUST:** Factory dispatches on `isinstance(client, AsyncInstructor)` first (because `AsyncInstructor` is the more specific type), then `Instructor`. Reversing the order silently routes async clients to the sync proxy and is a Blocker.
- **MUST NOT:** Use `inspect.iscoroutinefunction(...)` heuristics on `client.chat.completions.create` — the Instructor base method is defined the same way for both sync and async; the reliable signal is the class identity.

### 1.5 Shared run context

- **MUST:** Import `current_run_context` from `spendguard.integrations.openai_agents`. Duplicating `RunContext` / `run_context()` is a Blocker — polyglot trace sharing breaks if the contextvar identities diverge.
- **MUST NOT:** Modify `openai_agents.py` to support D28 — that file is locked at D-shipped state.

## 2. Gate semantics

### 2.1 Reserve-before-HTTP fail-closed

- **MUST:** `RequestDecision` is awaited (async) / called (sync) BEFORE `self._inner.chat.completions.create*(...)`. Reordering is a Blocker.
- **MUST:** DENY decision raises `SpendGuardDenied` BEFORE any inner-client method is invoked.
- **MUST:** The integration test `test_real_atomic_agents_deny_path_zero_provider_http` asserts ZERO HTTP requests reached the inner OpenAI transport on DENY. Missing this assertion is a Blocker.

### 2.2 Instructor validation-retry → per-retry reservation

- **MUST:** The integration test `test_real_instructor_validation_retry_creates_per_retry_reservation` exists and passes. This is the load-bearing test that justifies wrapping the Instructor object. Missing or skipping this test is a Blocker.
- **MUST:** The unit test `test_signature_diverges_across_instructor_validation_retries` synthesizes two successive kwargs dicts where `messages` differs by an injected validation-error message and asserts distinct `llm_call_id`s.
- **MUST NOT:** Add an explicit retry counter to the signature. The natural `messages`-divergence path is the documented, tested behavior. An explicit counter introduces a fragile coupling to Instructor's internal retry state.

### 2.3 POST emission semantics

- **MUST:** `emit_llm_call_post*` fires for SUCCESS / FAILURE / CANCELLED branches when `outcome.reservation_ids` is non-empty.
- **MUST:** `CancelledError` detection uses `type(exc).__name__ == "CancelledError"` (matches D12 / D24 / D26 pattern) — avoids cross-loop `isinstance` mismatches across `asyncio` / `trio` / `anyio`.
- **MUST:** When no reservation exists (DENY at PRE), POST MUST NOT fire. The test `test_sync_create_with_completion_skips_post_when_no_reservation` enforces this.

### 2.4 Usage extraction precedence

- **MUST:** `_extract_total_tokens` prefers `usage.total_tokens` over `prompt + completion`. When `usage is None`, return `0`. Raising or defaulting to a non-zero estimate is a Blocker.
- **MUST:** `_extract_provider_event_id` reads `raw_completion.id` (OpenAI-shaped `ChatCompletion`). Returning `""` when absent is correct; raising is a Blocker.

## 3. Signature security

### 3.1 `response_model` identity is in the signature

- **MUST:** `_signature` includes a stable identifier for `response_model` (e.g. `f"{cls.__module__}.{cls.__qualname__}"`). Omitting `response_model` lets a tenant flip schema mid-reservation. Blocker.

### 3.2 `model` is in the signature

- **MUST:** `model` is included so a `gpt-4o` → `gpt-4o-mini` (or vice-versa) swap yields a fresh `llm_call_id`. Otherwise a tenant could change cost class under one reservation. Blocker.

### 3.3 Deterministic signature

- **MUST:** Signature uses repeatable `repr()` on inputs. If any input is a dict, sort keys before hashing — Python's `repr({...})` is insertion-ordered, which is deterministic per process but flaky across recreation paths.
- **MUST:** `request_data` / `messages` are treated as opaque — wrapper MUST NOT log or persist them outside the signature hash. Atomic Agents' Pydantic schemas commonly carry user PII.

## 4. Pass-through completeness

- **MUST:** Every Instructor attribute except the two overridden `chat.completions` methods is reachable via `__getattr__`. Specifically: `mode`, `create_kwargs`, `default_model`, and any provider-specific attribute Instructor exposes per provider.
- **MUST:** Pass-through methods carry no side effects.

## 5. Demo + docs

### 5.1 Demo gate

- **MUST:** `agent_real_atomic_agents` demo mode exercises both ALLOW and DENY paths in a single `make demo` invocation. Splitting them across two modes is a finding (not Blocker).
- **MUST:** `verify_step_atomic_agents.sql` asserts `LLM_CALL_PRE` + paired `LLM_CALL_POST` for the ALLOW call AND `LLM_CALL_PRE` with `decision='DENY'` + NO paired POST for the DENY call. Missing either assertion is a Blocker.

### 5.2 Docs page leads with the wrap rationale

- **MUST:** `docs/site/docs/integrations/atomic-agents.md` opens with the Instructor-wrap-vs-raw-SDK rationale table. The rejected raw-SDK row MUST be present (not omitted as "obviously wrong") — operators reach for the raw-SDK wrap because it looks simpler, and the docs page is where we head that off.
- **MUST:** Page links to D27 (LlamaIndex) and D26 (Letta) "see also" because all three are Python adapters that wrap a client object via composition. Missing the cross-link is a finding.

### 5.3 README adapter table

- **MUST:** Row uses extras label `spendguard-sdk[atomic-agents]` exactly and references D28.

## 6. Security review

- **MUST:** No environment variable named `SPENDGUARD_ATOMIC_AGENTS_FAIL_OPEN` (or similar). Fail-closed is the only mode. Parity with D-shipped POC adapters.
- **MUST:** Signature derivation uses `model`, `messages`, `response_model` identity, `tools`, `tool_choice`. Missing any of those is a Blocker per §3.1-3.2.
- **MUST:** No silent type coercion that could let a malformed `response_model` (e.g. a string) bypass the schema-identity contribution to the signature.

## 7. Test rigor

### 7.1 Real-Atomic-Agents integration tests

- **MUST:** Integration tests use `pytest.importorskip("instructor", minversion="1.5")` AND `pytest.importorskip("atomic_agents", minversion="1.0")`. Hard-failing without either installed is a Blocker.
- **MUST:** At least one integration test exercises a real `BaseAgent.run(...)` round trip — not just `proxy.chat.completions.create_with_completion(...)` directly. The `BaseAgent`-level test catches Atomic Agents-side surface changes (e.g. a `BaseAgentConfig` field rename in 1.1.x).

### 7.2 Coverage matrix completeness

- The matrix in `tests.md` §4 MUST be 100% green. Any cell marked as "deferred" or "follow-on" is a Blocker for R1.

### 7.3 Mock layer

- **MUST:** `FakeInstructor` / `FakeAsyncInstructor` in `conftest_atomic_agents.py` mirror the real `chat.completions.create` / `create_with_completion` interface, NOT `MagicMock(spec=Instructor)`. Mock-spec is a finding — silently swallows signature changes upstream.
- **MUST:** `mocked_openai_transport` is an `httpx.MockTransport` (or `pytest-httpx` shim), not a method-level patch on `OpenAI.chat.completions.create`. Lower-level transport mocking is the only way to assert zero HTTP on the DENY path.

## 8. Cross-cutting

### 8.1 No regression on shipped adapters

- `tests/integrations/test_openai_agents.py` MUST remain green. D28 reuses `current_run_context` and MUST NOT modify `openai_agents.py`.
- `tests/integrations/test_letta.py` and `tests/integrations/test_langchain.py` MUST remain green.

### 8.2 Async dispatch covers Atomic Agents' documented surface

- `BaseAgent` accepts either an `Instructor` or `AsyncInstructor`. D28 MUST cover both. Async-only or sync-only coverage is a Blocker.

## 9. R5 panel arbitration triggers

If any of the following stand at R5, escalate to Staff+ panel per `framework-coverage-build-plan-2026-06.md` §1.3:

- Adapter accepts a raw provider SDK and silently wraps Instructor's retries with undercount.
- Wrapper subclasses `instructor.Instructor` / `instructor.AsyncInstructor` and inherits private init kwargs.
- Deny-path test does not assert zero inner HTTP.
- Validation-retry reservation-per-attempt test is missing or asserts only one reservation.
- `response_model` identity is omitted from the signature.
- `model` is omitted from the signature.
- Factory dispatch reverses sync-vs-async order and routes async clients to the sync proxy.

Panel composition per the build plan: Software Architect (summarizer) + Backend Architect + AI Engineer + Security Engineer + Senior Developer. Materials: this `review-standards.md`, the failing slice diff, the R1-R4 transcripts, and the `tests.md` coverage matrix.
