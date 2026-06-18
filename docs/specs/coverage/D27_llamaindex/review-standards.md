# D27 — Review standards

Backlinks: [`design.md`](./design.md), [`implementation.md`](./implementation.md), [`tests.md`](./tests.md), [`acceptance.md`](./acceptance.md).

This checklist is what `superpowers:code-reviewer` runs per slice (R1-R5). Each line is a **must** unless explicitly marked optional. Slice-level reviewers must run the corresponding gates from `acceptance.md` and cite results.

## §1. Cross-cutting standards (every slice)

- [ ] **No proto / DB / migration changes.** Run `git diff --stat origin/main -- proto/ schema/ migrations/` — must be empty.
- [ ] **No edits to the 6 already-shipped adapters** (`langchain.py`, `openai_agents.py`, `litellm.py`, `pydantic_ai.py`, `agt.py`). The only allowed shared-file touch is `_default_estimator.py` and it must be **additive** (no rename / signature change of existing symbols).
- [ ] **No proto types in public `__all__`.** Reviewer greps `__all__` against the proto module path — none must appear.
- [ ] **No `print` statements in `llamaindex.py`.** Logging via `logging.getLogger(__name__)` only.
- [ ] **No bare `except`.** Every `except` clause names a specific exception type.
- [ ] **No async overload.** Handler is sync. `client.request_decision_sync` + `emit_llm_call_post_sync` are the documented entry points (per AGT precedent).
- [ ] **Type hints on every function signature.** `mypy --strict` clean on the new file.
- [ ] **Docstrings on every public symbol** + module-level docstring matching the `langchain.py` / `openai_agents.py` style (integration shape example included).
- [ ] **No provider sub-packages in `[llamaindex]` extra.** Reviewer asserts that `llama-index-llms-openai` / `-anthropic` / `-gemini` / `-bedrock` do NOT appear in the `[llamaindex]` extra of `pyproject.toml` (operators install whichever vendor they use).

## §2. Slice S1 — Module skeleton + extras + import guard + denied exception

- [ ] `pyproject.toml` `[llamaindex]` extra includes `llama-index-core>=0.12` exactly (no upper bound).
- [ ] Module-level `try / except ImportError` guard mirrors `langchain.py` line-by-line (raise-from, install-hint substring).
- [ ] Install-hint string `pip install 'spendguard-sdk[llamaindex]'` matches the LangChain prior verbatim.
- [ ] Imports from `llama_index.core` ONLY. No `llama_index.llms.*`. No `from llama_index import *`.
- [ ] `SpendGuardLlamaIndexDenied` is a `SpendGuardError` subclass.
- [ ] `SpendGuardLlamaIndexDenied.reason_codes` is a `list[str]` (never `None`); default falls back to `["BUDGET_EXHAUSTED"]` in the message string.
- [ ] U01 passes.
- [ ] Gates G01, G02, G03 from `acceptance.md` pass.

## §3. Slice S2 — `SpendGuardLlamaIndexHandler` class + event filter + state map + trace hooks

- [ ] Class inherits `BaseCallbackHandler` directly (not via composition).
- [ ] `super().__init__(event_starts_to_ignore=[], event_ends_to_ignore=[])` is called explicitly (LlamaIndex's base requires it).
- [ ] `on_event_start` and `on_event_end` both early-return on `event_type != CBEventType.LLM` BEFORE any sidecar interaction.
- [ ] `self._state: dict[str, _PendingCall]` is the ONLY mutable instance field used during request handling (config fields are immutable).
- [ ] `_PendingCall` is a `@dataclass(slots=True)` (memory hygiene + immutability).
- [ ] `start_trace` stores `trace_id` on the instance; `end_trace` clears it ONLY when the passed `trace_id` matches the stored one (mismatched id is a no-op).
- [ ] No instance-level cross-request mutable state beyond `self._state` and `self._trace_id`.
- [ ] Default `claim_estimator` wiring matches the `openai_agents.py` pattern (loads from `_default_estimator`).
- [ ] U07, U08, U10, U25 pass.

## §4. Slice S3 — PRE / POST wiring

### §4.1 PRE (`_on_llm_start`)

- [ ] Signature derivation uses `hashlib.blake2b(..., digest_size=16)` (32 hex chars) for symmetry with LangChain prior.
- [ ] `derive_uuid_from_signature` is called twice — once for `llm_call_id` scope and once for `decision_id` scope.
- [ ] `idempotency_key` is derived through `derive_idempotency_key` (no hand-rolled hashing).
- [ ] `trigger="LLM_CALL_PRE"` and `route="llm.call"` are literal strings (not enums constructed inline).
- [ ] `tool_call_id=""` is passed (not `None`).
- [ ] `DecisionDenied` is the ONLY caught exception class (no broader `except SpendGuardError`).
- [ ] Deny path raises `SpendGuardLlamaIndexDenied(reason_codes=list(...))` and does NOT stash any `_PendingCall` in `self._state`.
- [ ] Reviewer greps for `raise SpendGuardLlamaIndexDenied` and finds exactly ONE occurrence in `_on_llm_start`.
- [ ] `from exc` chaining on the raise (preserves traceback per Python convention).

### §4.2 POST (`_on_llm_end`)

- [ ] `self._state.pop(event_id, None)` is the FIRST operation (cleanup + lookup in one shot).
- [ ] If `pending is None` → silent return; NO RPCs, no warning, no exception (this is the documented DENY / non-LLM path).
- [ ] `outcome="SUCCESS"` literal.
- [ ] `provider_reported_amount_atomic=""` (the SpendGuard convention for unknown).
- [ ] `estimated_amount_atomic` is the `str(int)` form of `_extract_total_tokens`, even when 0.
- [ ] `provider_event_id` flows through `_extract_provider_event_id`, falls back to `""` (never `None`).

### §4.3 Run-ID resolution

- [ ] Order: `run_id_fn` (if set) → `self._trace_id` (if set) → `parent_id` (if non-empty) → derived UUID from signature.
- [ ] Derived UUID branch uses `derive_uuid_from_signature(scope="run_id")` (not a fresh `new_uuid7`) to keep retries deterministic.

### §4.4 Usage extraction

- [ ] Extraction order is exactly: (1) `raw["usage"]["total_tokens"]` (OpenAI) → (2) `raw["usage"]["input_tokens"] + ["output_tokens"]` (Anthropic) → (3) `raw["usage_metadata"]["total_token_count"]` (Gemini) → (4) `raw["usage"]["inputTokens"] + ["outputTokens"]` (Bedrock Converse) → (5) `0`.
- [ ] Each numeric branch is a positive-only check (`isinstance(x, int) and x > 0` for canonical totals; `int(x or 0) + int(y or 0)` for split totals).
- [ ] No `try / except` in `_extract_total_tokens` — pure `Mapping`-aware attribute access.
- [ ] U09, U11, U12, U13, U14, U15, U16, U17, U18, U19, U20, U21, U22, U23, U24 pass.

## §5. Slice S4 — Tests

- [ ] All 25 unit tests (U01-U25) exist with the exact names in `tests.md` §1.
- [ ] All 8 integration tests (I01-I08) exist with the exact names in `tests.md` §1.
- [ ] Unit suite runs **without** `llama-index-core` installed (via `SimpleNamespace`/`_StubBase` stub fallback).
- [ ] Integration suite uses `pytest.importorskip("llama_index.core")` (skip, not fail, when extra absent).
- [ ] Recorded fixtures exist at the documented paths and pass G06.
- [ ] No live API call in any test (grep for `openai.com`, `anthropic.com`, `generativelanguage.googleapis.com`, `bedrock-runtime` — must find zero in test code paths).
- [ ] Sidecar fake is the **existing** `FakeSpendGuardServer` (no new fake implementation).
- [ ] Mock provider LLM classes are used in I01-I05 (not real `OpenAI` / `Anthropic` clients).
- [ ] `MockLLM` from `llama_index.core.llms` is used in I06-I08 (proves end-to-end without provider HTTP).
- [ ] U07 (filter test) explicitly enumerates the 5 non-LLM event types — reviewer verifies the list matches the LlamaIndex `CBEventType` enum.
- [ ] Gates G04, G05, G06, G07, G08, G13 pass.

## §6. Slice S5 — Demo + docs

### §6.1 Demo

- [ ] `run_demo.py` `agent_real_llamaindex` branch follows the same shape as the existing `agent_real_langchain` / `agent_real_openai_agents` branches.
- [ ] `OPENAI_API_KEY` check fails fast with `sys.exit(2)` + FATAL log line (mirror LangChain branch).
- [ ] Demo runs **two** queries: one ALLOW + one DENY, with the DENY produced by setting `BUDGET = 0` (via `_exhaust_budget` helper) before the second.
- [ ] Demo log emits both:
  - `[demo] agent_real_llamaindex run completed: ALLOW path`
  - `[demo] agent_real_llamaindex run completed: DENY path (model not called)`
- [ ] Demo wires `try: qe.query(...) except SpendGuardLlamaIndexDenied:` — does NOT swallow other exception types.
- [ ] `Makefile` `demo-up-agent-real-llamaindex` alias exists.
- [ ] A no-API-key variant `agent_real_llamaindex_stub` exists for CI (G09) and uses `llama_index.core.llms.MockLLM`.
- [ ] Gate G09 passes.

### §6.2 Docs

- [ ] `docs/site/docs/integrations/llamaindex.md` exists.
- [ ] Page documents: install, basic registration (`Settings.callback_manager = ...`), advanced (`run_id_fn` override, custom `claim_estimator`), DENY behavior (raised `SpendGuardLlamaIndexDenied`).
- [ ] Page ships the **2-path coverage matrix** table per `implementation.md` §9, with explicit cross-link to the D12 LiteLLM SDK shim docs page for the `-litellm` path.
- [ ] Page documents the non-goals (no streaming intra-chunk; embedding/retrieve filtered).
- [ ] Page links back to `docs/site/docs/integrations/index.md` and is linked from there in turn.
- [ ] `README.md` adapter table row added per `implementation.md` §8.
- [ ] Gate G11, G12 pass.

## §7. Security review

- [ ] No secret material logged. Reason codes, reservation IDs, decision IDs are OK; raw prompts / responses must NOT be logged.
- [ ] `SpendGuardLlamaIndexDenied.__str__` contains only reason codes — never raw user prompt content or sidecar-internal IDs that aren't already in the public decision-id format.
- [ ] No `eval` / `exec` / `pickle.loads` on anything reaching the adapter (including fixture loads — use `json.loads` only).
- [ ] No dependency on a non-pinned `llama-index-core` floor — `>= 0.12` exactly.
- [ ] `_signature_for` does not include API keys or auth headers from `payload[EventPayload.SERIALIZED]` (verify by inspecting the read fields — should be `model` + `messages`/`prompt` only).
- [ ] The `_extract_*` helpers do NOT log `response.raw` content (it may contain user PII echoed back).

## §8. Performance review

- [ ] Non-LLM events incur ≤ 1 `enum` compare + early return (reviewer reads the dispatcher and confirms).
- [ ] LLM events add ≤ 1 sidecar round-trip on PRE; ≤ 1 on POST.
- [ ] No synchronous blocking call from event handlers beyond the documented `client.*_sync` calls (no `time.sleep`, no `requests.post`, no `urllib`).
- [ ] No per-call object allocations beyond what the LangChain prior does — reviewer cross-checks allocation count by inspecting the source.
- [ ] Default claim estimator caches its dispatched factory (one `_default_estimator` lookup per handler instance, not per call).
- [ ] `self._state` cleanup happens in EVERY successful `on_event_end` invocation (verified by U21).

## §9. Slice-exit checklist (every slice)

- [ ] All slice-specific gates above pass.
- [ ] `git status` clean modulo the slice's own files.
- [ ] Slice doc under `docs/internal/slices/COV_D27_S<N>_*.md` exists with the required sections (scope, files touched, test plan, backlinks, anti-scope).
- [ ] Commit message follows project convention (`feat(llamaindex): <one-line summary>` for impl slices; `test(llamaindex): ...`; `docs(llamaindex): ...`).

## §10. Deliverable-exit (after S5)

- [ ] Gates G01-G09 + G11-G18 from `acceptance.md` all green.
- [ ] G10 confirmed locally with evidence (log lines + SQL output) attached to the memory write-back per build-plan §8.
- [ ] No open `superpowers:code-reviewer` findings against any slice.
- [ ] Memory entry written under `~/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/project_coverage_D27_shipped.md` per build-plan §2.4 + §8.
- [ ] README adapter table row visible at HEAD.
- [ ] `docs/site/docs/integrations/llamaindex.md` linked from the integrations index, and the 2-path matrix is visible on the rendered page.
- [ ] D12 docs page cross-link verified (round-trip: D27 page → D12 page → mentions LlamaIndex transitive coverage).
