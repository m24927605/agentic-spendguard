# D19 — Review standards

Backlinks: [`design.md`](./design.md), [`implementation.md`](./implementation.md), [`tests.md`](./tests.md), [`acceptance.md`](./acceptance.md).

This checklist is what `superpowers:code-reviewer` runs per slice (R1-R5). Each line is a **must** unless explicitly marked optional. Slice-level reviewers must run the corresponding gates from `acceptance.md` and cite results.

## §1. Cross-cutting standards (every slice)

- [ ] **No proto / DB / migration changes.** Run `git diff --stat origin/main -- proto/ schema/ migrations/` — must be empty.
- [ ] **No edits to the 6 already-shipped adapters** (`langchain.py`, `openai_agents.py`, `litellm.py`, `pydantic_ai.py`, `agt.py`, and the LangGraph helper). The only allowed shared-file touch is `_default_estimator.py` and it must be **additive** (no rename / signature change of existing symbols).
- [ ] **No proto types in public `__all__`.** Reviewer greps `__all__` against the proto module path — none must appear.
- [ ] **No `print` statements in `adk.py`.** Logging via `logging.getLogger(__name__)` only.
- [ ] **No bare `except`.** Every `except` clause names a specific exception type.
- [ ] **All async work `await`-ed.** No fire-and-forget `asyncio.create_task` without retention.
- [ ] **Type hints on every function signature.** `mypy --strict` clean on the new file.
- [ ] **Docstrings on every public symbol** + module-level docstring matching the `langchain.py` / `openai_agents.py` style (integration shape example included).

## §2. Slice S1 — Module skeleton + extras + import guard

- [ ] `pyproject.toml` `[adk]` extra includes `google-adk>=1.0` exactly (no upper bound).
- [ ] Module-level `try / except ImportError` guard mirrors `langchain.py` line-by-line (raise-from, install-hint substring).
- [ ] Install-hint string `pip install 'spendguard-sdk[adk]'` matches the LangChain prior verbatim.
- [ ] No imports from `google.adk` outside the guard.
- [ ] No `from google.adk import *`.
- [ ] U01 passes.
- [ ] Gates G01, G02, G03 from `acceptance.md` pass.

## §3. Slice S2 — `SpendGuardAdkCallback` class + dispatch + state handoff

- [ ] `__call__` is `async def` (not `def`); ADK accepts both, but `request_decision` is async.
- [ ] Dispatch is by `isinstance(payload, LlmRequest)` first, then `LlmResponse` — not by arity.
- [ ] State keys are namespaced with the `spendguard.` prefix and pulled from class constants (no inline strings).
- [ ] No instance-level mutable state shared across runs (the only allowed instance fields are init-time config).
- [ ] U05, U06, U07, U08 pass.
- [ ] Reviewer confirms the four state keys (`reservation_id`, `decision_id`, `step_id`, `llm_call_id`) are all stashed on ALLOW and **none** stashed on DENY (replaced by the single `denied` flag).
- [ ] Default `claim_estimator` wiring matches the `openai_agents.py` pattern (loads from `_default_estimator`, dispatched off model name).

## §4. Slice S3 — PRE / POST wiring

### §4.1 PRE (`_before`)

- [ ] Signature derivation uses `hashlib.blake2b(..., digest_size=16)` (32 hex chars) for symmetry with LangChain prior.
- [ ] `derive_uuid_from_signature` is called twice — once for `llm_call_id` scope and once for `decision_id` scope.
- [ ] `idempotency_key` is derived through `derive_idempotency_key` (no hand-rolled hashing).
- [ ] `trigger="LLM_CALL_PRE"` and `route="llm.call"` are literal strings (not enums constructed inline).
- [ ] `tool_call_id=""` is passed (not `None`).
- [ ] `DecisionDenied` exception caught and **only** `DecisionDenied`-family (no broader `except SpendGuardError`).
- [ ] Deny response built via `_build_deny_response` helper, not inline.
- [ ] `error_code="SPENDGUARD_DENY"` literal — reviewer must grep the source for this exact string and find one occurrence.
- [ ] Deny `error_message` includes reason codes comma-joined; defaults to `BUDGET_EXHAUSTED` when reason_codes is empty.

### §4.2 POST (`_after`)

- [ ] `denied` flag is checked **before** any RPC.
- [ ] All four state keys are required (and-checked) before `emit_llm_call_post`. Missing any → silent return (no exception).
- [ ] `outcome="SUCCESS"` literal.
- [ ] `provider_reported_amount_atomic=""` (the SpendGuard convention for unknown).
- [ ] `estimated_amount_atomic` is the `str(int)` form of `_extract_total_tokens`, even when 0.
- [ ] `provider_event_id` flows through `_extract_provider_event_id`, falls back to `""` (never `None`).

### §4.3 Usage extraction

- [ ] Extraction order is exactly: (1) Gemini `total_token_count` → (2) Gemini `prompt + candidates` → (3) OpenAI `total_tokens` → (4) `0`.
- [ ] Each branch is a positive-only check (`isinstance(x, int) and x > 0` for #1, `or 0` for sums) — no implicit truthy on potentially-zero ints.
- [ ] No try/except in `_extract_total_tokens` — pure attribute access via `getattr(..., default=None)`.

- [ ] U02, U03, U04, U09, U10, U11, U12, U13, U14, U15, U16, U17, U18, U19, U20 pass.

## §5. Slice S4 — Tests

- [ ] All 20 unit tests (U01-U20) exist with the exact names in `tests.md` §1.
- [ ] All 5 integration tests (I01-I05) exist with the exact names in `tests.md` §1.
- [ ] Unit suite runs **without** `google-adk` installed (via `SimpleNamespace` stub fallback).
- [ ] Integration suite uses `pytest.importorskip("google.adk")` (skip, not fail, when extra absent).
- [ ] Recorded fixtures exist at the documented paths and pass G06.
- [ ] No live API call in any test (grep for `genai.GenerativeModel`, `generativelanguage.googleapis.com` — must find zero in test code paths).
- [ ] Sidecar fake is the **existing** `FakeSpendGuardServer` (no new fake implementation).
- [ ] Gates G04, G05, G06, G07, G12 pass.

## §6. Slice S5 — Demo + docs

### §6.1 Demo

- [ ] `run_demo.py` `agent_real_adk` branch follows the same shape as the existing `agent_real_langchain` / `agent_real_openai_agents` branches.
- [ ] `GOOGLE_API_KEY` check fails fast with `sys.exit(2)` + FATAL log line (mirror LangChain branch).
- [ ] Demo runs **two** turns: one ALLOW + one DENY, with the DENY produced by setting `BUDGET = 0` before the second turn.
- [ ] Demo log emits both:
  - `[demo] agent_real_adk run completed: ALLOW path`
  - `[demo] agent_real_adk run completed: DENY path (model not called)`
- [ ] `Makefile` `demo-up-agent-real-adk` alias exists.
- [ ] A no-API-key variant `agent_real_adk_stub` exists for CI (G08).
- [ ] Gate G08 passes.

### §6.2 Docs

- [ ] `docs/site/docs/integrations/adk.md` exists.
- [ ] Page documents: install, basic registration (both slots), advanced (`run_id_fn` override, custom `claim_estimator`), DENY behavior (synthetic `LlmResponse`).
- [ ] Page links back to `docs/site/docs/integrations/index.md` and is linked from there in turn.
- [ ] `README.md` adapter table row added per `implementation.md` §8.
- [ ] Gate G10, G11 pass.

## §7. Security review

- [ ] No secret material logged. Reason codes, reservation IDs, decision IDs are OK; raw prompts / responses must not be logged.
- [ ] `error_message` on the deny `LlmResponse` contains only reason codes — never raw user prompt or sidecar internal IDs that aren't already in the public decision-id format.
- [ ] No `eval` / `exec` / `pickle.loads` on anything reaching the adapter.
- [ ] No dependency on a non-pinned `google-adk` floor — `>= 1.0` exactly.
- [ ] `_signature_for` does not include API keys or auth headers from `llm_request` (verify by reading the field list).

## §8. Performance review

- [ ] PRE adds ≤ 1 sidecar round-trip; POST adds ≤ 1.
- [ ] No synchronous blocking call from `__call__` or its delegates (no `time.sleep`, no `requests.post`, no `urllib`).
- [ ] No per-call object allocations beyond what the LangChain prior does — reviewer cross-checks allocation count by inspecting the source.
- [ ] Default claim estimator caches its dispatched factory (one `_default_estimator` lookup per `SpendGuardAdkCallback` instance, not per call).

## §9. Slice-exit checklist (every slice)

- [ ] All slice-specific gates above pass.
- [ ] `git status` clean modulo the slice's own files.
- [ ] Slice doc under `docs/internal/slices/COV_D19_S<N>_*.md` exists with the required sections (scope, files touched, test plan, backlinks, anti-scope).
- [ ] Commit message follows project convention (`feat(adk): <one-line summary>` for impl slices; `test(adk): ...`; `docs(adk): ...`).

## §10. Deliverable-exit (after S5)

- [ ] Gates G01-G08 + G10-G15 from `acceptance.md` all green.
- [ ] G09 confirmed locally with evidence attached to the memory write-back per build-plan §8.
- [ ] No open Codex / `superpowers:code-reviewer` findings against any slice.
- [ ] Memory entry written under `~/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/project_coverage_D19_shipped.md` per build-plan §2.4 + §8.
- [ ] README adapter table row visible at HEAD.
- [ ] `docs/site/docs/integrations/adk.md` linked from the integrations index.
