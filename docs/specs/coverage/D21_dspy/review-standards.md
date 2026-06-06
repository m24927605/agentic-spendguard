# D21 — Review Standards

**Audience:** `superpowers:code-reviewer` skill (per build-plan §1.2 the canonical reviewer for every slice). Backup: R5 panel arbitration (build-plan §1.3).
**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`acceptance.md`](acceptance.md).

## 1. Per-slice acceptance bar

A slice passes when, **and only when**:

1. The slice's diff matches the file boundary in `implementation.md` §2 (e.g. Slice 1 touches only `dspy.py` skeleton + `_litellm_shim.py` + pyproject extra + partial test file).
2. All hard gates from `acceptance.md` §1 that are runnable at this slice's commit point pass.
3. Findings count from `superpowers:code-reviewer` is zero (Blockers and Majors). Minors may be deferred to a follow-up GitHub issue with explicit rationale captured in the slice's commit message.
4. The slice maintains backwards compatibility per `implementation.md` §3. **In particular, no existing integration module (`langchain.py`, `pydantic_ai.py`, `openai_agents.py`, `litellm.py`, `agt.py`, `_default_estimator.py`) may be modified by D21 slices.**

## 2. Slice-specific reviewer checklist

### Slice 1 — Module skeleton + extras + `_PENDING` registry + run-context

| # | Check | Severity |
|---|-------|----------|
| 1.1 | Module imports `dspy.utils.callback.BaseCallback` at top level with `try/except ImportError` + install-hint message. | Blocker |
| 1.2 | `_PENDING: dict[str, _CallState]` is module-level; no class-attached state. | Blocker |
| 1.3 | `_PENDING_TTL_SECONDS = 300` constant defined; used in TTL sweep. | Major |
| 1.4 | `_SHIM_IN_FLIGHT` imported from `spendguard._litellm_shim` (NOT redefined locally). Object identity must match D12's eventual import target. | Blocker |
| 1.5 | `spendguard._litellm_shim` module defines `_IN_FLIGHT` as `contextvars.ContextVar[bool]` with default False. No other exports. | Blocker |
| 1.6 | `[dspy]` extra in `pyproject.toml` floors `dspy-ai>=2.6`, ceiling `<3.0`. | Major |
| 1.7 | `RunContext` is `@dataclass(frozen=True, slots=True)`. Default factory emits UUIDv7 via `new_uuid7()`. | Major |
| 1.8 | `_CallState` dataclass carries `started_at` for TTL sweep; `shim_token: contextvars.Token[bool] | None`. | Blocker |
| 1.9 | No mutation of module-level state at import time beyond logger + empty `_PENDING` dict. | Major |
| 1.10 | Tests U01-U04 present. | Major |

### Slice 2 — Callback class: `on_lm_start` + `on_lm_end` wiring

| # | Check | Severity |
|---|-------|----------|
| 2.1 | `SpendGuardDSPyCallback` inherits `BaseCallback`. Calls `super().__init__()`. | Blocker |
| 2.2 | `__init__` signature uses keyword-only args (`*,`). `claim_reconciler` is required; `claim_estimator` is Optional. | Blocker |
| 2.3 | `on_lm_start` calls `_sweep_pending()` FIRST, then `_guard_async_context()`, before any other work. | Blocker |
| 2.4 | `_SHIM_IN_FLIGHT.set(True)` happens via `token = _SHIM_IN_FLIGHT.set(True)` + token stored in `_CallState.shim_token`. NEVER a plain set without token capture. | Blocker |
| 2.5 | On DENY or DEGRADE, `on_lm_start` calls `_SHIM_IN_FLIGHT.reset(token)` BEFORE re-raising. `_PENDING` is NOT populated. | Blocker |
| 2.6 | `on_lm_end` pops `_PENDING[call_id]` exactly once. Returns gracefully (WARN log) if entry is missing. | Blocker |
| 2.7 | `on_lm_end` outcome classification: `CancelledError → CANCELLED`, other exception → `FAILURE`, None → `SUCCESS`. Order of isinstance checks matters (CancelledError is subclass of BaseException). | Blocker |
| 2.8 | `on_lm_end` resets `_SHIM_IN_FLIGHT` via `try/finally` so the contextvar is restored even if `emit_llm_call_post` raises. | Blocker |
| 2.9 | `_extract_total_tokens` handles all of: `None`, bare string list, `LMResponse` with `.usage` dict, missing `.usage` attr. Never raises. | Blocker |
| 2.10 | No mutation of caller's `inputs` dict (no `del`, no key injection). Test U05 / U06 assert identity. | Blocker |
| 2.11 | Tests U05-U18 present (16 unit tests). | Blocker |
| 2.12 | `_guard_async_context` uses `asyncio.get_running_loop()` via `try/except RuntimeError`. Raises `SyncInAsyncContext` ONLY if a loop is running. | Blocker |
| 2.13 | Error message of `SyncInAsyncContext` names `SpendGuardDSPyCallback.on_lm_start` AND points caller at "sync entrypoint". | Major |
| 2.14 | `asyncio.run` used for sync→async dispatch. NEVER `loop.run_until_complete` on a manually-created loop. | Blocker |
| 2.15 | `_signature_from_inputs` uses `json.dumps(..., sort_keys=True, default=str)` + `blake2b(digest_size=16)`. Deterministic. | Major |

### Slice 3 — Tests + demo `agent_real_dspy`

| # | Check | Severity |
|---|-------|----------|
| 3.1 | Every test that calls `on_lm_start` wraps in `try/finally` calling `on_lm_end` OR asserts the exception path triggered cleanup. Fixture `dspy_pending_clean` enforces `_PENDING == {}` at teardown. | Blocker |
| 3.2 | No unit test relies on real `dspy` — all uses minimal mock subclass. Real-dspy verification is in `test_dspy_real.py`. | Blocker |
| 3.3 | U09 ordering test uses a list of recorded labels and asserts EXACTLY `["reserve", "provider"]`. Strict order. | Blocker |
| 3.4 | U10 DENY test asserts BOTH `mock_provider_http.call_count == 0` AND `_PENDING == {}` AND `_SHIM_IN_FLIGHT.get() == False`. | Blocker |
| 3.5 | I01 strict-order check uses `asyncio.Event` set by fake-sidecar; pytest-httpx callback verifies event is set before recording. | Blocker |
| 3.6 | I04 D12 coexistence test installs BOTH adapters and asserts exactly 1 reserve per `dspy.Predict(...)` call (not 2). | Blocker |
| 3.7 | `DEMO_MODE=agent_real_dspy` Makefile branch wires the new bootstrap; does NOT mount other demo configs. | Blocker |
| 3.8 | Demo driver `run_dspy_real_mode` step 1 ALLOW prints a non-empty `result.answer`. | Blocker |
| 3.9 | Demo driver step 3 CUSTOM-LM inline-defines a `dspy.LM` subclass that hits OpenAI HTTP directly (NOT via LiteLLM) — proves direct-path coverage. | Blocker |
| 3.10 | `verify_step_agent_real_dspy.sql` includes the 5 assertions from `tests.md` §4. | Blocker |
| 3.11 | Stub-counter delta assertion (INV-1) is present and uses `decision_context->>'expected_allow_count'` or equivalent. | Major |
| 3.12 | Outbox closure check runs after the demo per existing `Makefile` pattern. | Major |
| 3.13 | No regressions in adjacent demo modes (`agent`, `agent_real_openai_agents`, `litellm_real`, etc.) — those Makefile branches not edited. | Blocker |
| 3.14 | All 16 unit tests + 5 integration tests pass under `pytest -v`. | Blocker |

### Slice 4 — Docs + README

| # | Check | Severity |
|---|-------|----------|
| 4.1 | New page `docs/site/docs/integrations/dspy.md` exists and renders via `cd docs/site && npm run build`. | Blocker |
| 4.2 | Decision matrix lists 2 paths (D12 transitive / D21 direct) with explicit "when to use" rows. | Blocker |
| 4.3 | "Limitations" section explicitly states the 4 non-goals from `design.md` §3. | Blocker |
| 4.4 | "1-minute install" snippet uses `dspy.configure(callbacks=[SpendGuardDSPyCallback(...)])` AND notes the callback MUST be FIRST. | Blocker |
| 4.5 | README adapter integrations table gains exactly one row for `DSPy`. | Major |
| 4.6 | Cross-link added from existing D12 page (when D12 lands) noting D21 covers custom dspy.LM subclasses. | Minor |
| 4.7 | `deploy/demo/dspy/README.md` exists with demo-mode notes (env vars, expected output, troubleshooting). | Major |

## 3. Cross-cutting reviewer focus areas (every slice)

| Area | What to check | Severity if missed |
|------|---------------|--------------------|
| Backwards compatibility | Did the slice mutate any existing `integrations/*.py` file? G12 must produce 0-line diff. | Blocker |
| Type hints | All new public functions carry full hints. `from __future__ import annotations` used. | Major |
| Logging | All `log.warning` / `log.info` callsites carry the `spendguard.integrations.dspy:` prefix. No secrets in logs. | Major |
| Error messages | All `SpendGuardConfigError` strings name the offending function. `SyncInAsyncContext` hint points at sync entrypoint. | Major |
| Secret leakage | No logging of `api_key`, `inputs.get("messages")` content (may contain user PII), `instance.kwargs`. | Blocker |
| Test isolation | Unit tests do NOT require Docker, do NOT require a running sidecar, do NOT make outbound HTTP. Each test's `_PENDING` and `_SHIM_IN_FLIGHT` cleaned via fixture. | Blocker |
| Async / sync mixing | No `asyncio.run()` from inside an async function. No `loop.run_until_complete` ever. `_SHIM_IN_FLIGHT` is contextvar (per-task), never threadlocal. | Blocker |
| Pending dict hygiene | Every `on_lm_start` that successfully populates `_PENDING` MUST have a matching `on_lm_end` path that pops it OR a TTL sweep that catches it. No leaks. | Blocker |
| Global state | `_PENDING` is the only mutable module-level state. No other module-level mutability beyond it + logger + `_SHIM_IN_FLIGHT` (which is immutable reference to a ContextVar). | Blocker |
| Dependency surface | No new runtime dependency beyond `dspy-ai>=2.6` (extra) + `pytest-httpx>=0.30` (test-only). No new compile-time deps. | Major |
| Callback ordering | `on_lm_start` MUST not call any user callback. Doc + test asserts SpendGuardDSPyCallback comes FIRST in the callbacks list (U08). | Major |
| Outputs schema tolerance | `_extract_total_tokens` and `_extract_provider_event_id` must NEVER raise. They handle None, list, missing attr, wrong type. | Blocker |
| Shared contextvar contract | `_SHIM_IN_FLIGHT` import path is `from spendguard._litellm_shim import _IN_FLIGHT` (NOT a redefinition). G13 verifies object identity. | Blocker |

## 4. R1-R5 review loop reminders (per build-plan §1.1)

| Round | Reviewer action | Implementer action on findings |
|-------|----------------|--------------------------------|
| R1 | Run `superpowers:code-reviewer` on slice diff + this checklist. | Address every Blocker + Major. Defer Minors with rationale in commit message. |
| R2 | Re-run reviewer on the post-fix diff. | Same as R1. |
| R3 | Re-run. By R3, Blockers should be at zero. | If R3 still has Blockers, escalate to R4 with structural changes — do not patch around. |
| R4 | Last "self-contained" round. | Significant structural changes may invalidate earlier review findings; reviewer must re-evaluate the whole slice diff, not just deltas. |
| R5 | Final round before panel. | If R5 has any Blocker, escalate to Staff+ panel arbitration per build-plan §1.3. |
| Panel | 5 panelists per build-plan §1.3. Summarizer Software Architect. | Implementer follows panel ruling (merge-with-residuals / block / rework). |

## 5. Panel-arbitration likely triggers (so the implementer knows)

If a slice is likely to need panel arbitration, surface it in the slice's commit message early. Likely D21 triggers:

- **Slice 1 shared contextvar contract:** if D12 has already shipped (different design) by the time D21 lands, the `_litellm_shim.py` placement may conflict. Panel decides whether to (a) refactor D12's existing `_IN_FLIGHT` into `spendguard._litellm_shim`, or (b) hold D21 to its own contextvar with an explicit `D12_compatibility` integration layer. Current spec assumes (a).
- **Slice 2 `on_lm_start` sync→async dispatch:** if DSPy 2.7+ introduces async-native hooks, the `asyncio.run` bridge becomes unnecessary AND incompatible. Panel decides whether to (a) pin DSPy floor to 2.6 explicitly with ceiling <2.7, or (b) feature-detect and use the async hook when present. Current spec assumes (a).
- **Slice 2 `_extract_total_tokens` provider drift:** if a future DSPy version renames `LMResponse.usage` to `LMResponse.tokens` or similar, the extractor breaks. Panel decides whether to (a) widen the heuristic to multiple keys, or (b) accept the breakage and bump the DSPy floor. Current spec leans (a).
- **Slice 3 demo CUSTOM-LM subclass:** if DSPy 2.6 + tightens the `dspy.LM` ABI such that subclasses can't bypass LiteLLM easily, the demo step 3 becomes unrealistic. Panel decides whether to replace step 3 with a different direct-path proof (e.g. a `dspy.LM` subclass wired to Ollama local model). Current spec assumes the OpenAI direct-HTTP bypass works.
- **Slice 4 decision matrix completeness:** reviewer may push for adding a 3rd row (egress proxy D02). Panel decides whether D02 belongs in the DSPy matrix or just D12 + D21. Current spec is 2 rows; D02 cross-link in "See also" suffices.

## 6. Slice-merge order is fixed

Per dependency in `implementation.md` §2: **Slice 1 → 2 → 3 → 4**, never reorder. Slices 3 + 4 may parallelize after slice 2 lands. Slice 4 docs may reference slice 3 demo evidence so completing slice 3 first reduces churn.

## 7. Final reviewer override

If the reviewer believes the spec itself is wrong (e.g. shared contextvar design, sync-in-async raise semantics, `_PENDING` registry vs threadlocal), flag it as a Blocker on the relevant slice with rationale referencing `design.md` §5 "Key decisions" — do not silently deviate. Spec changes route through Staff+ panel per build-plan §1.3.

## 8. Spec-pair consistency check (D12 + D21)

D21 ships independently of D12 but coordinates via the shared `_litellm_shim.py` contextvar. The reviewer MUST verify on every D21 slice:

- D21 does NOT modify `litellm_shim.py` (D12's module, may or may not exist at D21 ship time).
- D21's `_litellm_shim.py` placeholder defines ONLY `_IN_FLIGHT`. No `install` / `uninstall` / `is_installed` functions (those belong to D12's `litellm_shim.py`).
- D21's docs page lists D12 as a peer path (not deprecated, not preferred over D21).
- When D12 ships AFTER D21: D12's review must catch the requirement to import `_IN_FLIGHT` FROM `spendguard._litellm_shim` (not redefine locally).
- When D12 ships BEFORE D21: D21's slice 1 review must verify D12's `litellm_shim.py` already pulls `_IN_FLIGHT` from `_litellm_shim.py` and that D21's import resolves to the same object (G13 verification).

If neither has shipped yet at the time of D21 slice review, the reviewer treats the contract as forward-looking and verifies via G13 at slice 1 review time.

## 9. Acceptance test residuals — known deferred surface

The following are known limitations explicitly out of scope per `design.md` §3 and `acceptance.md` §5. The reviewer accepts these as deferred residuals (not Blockers):

- Token-by-token streaming gating
- `on_tool_*` / `on_module_*` callback gating
- Async DSPy callback support (when DSPy 2.7+ ships them)
- Per-attempt idempotency for DSPy retry loops
- Multi-callback ordering enforcement beyond docs (operator responsibility)

If a future spec (e.g. D21.1) targets any of these, it ships as a separate deliverable per build-plan §2.1 cadence.
