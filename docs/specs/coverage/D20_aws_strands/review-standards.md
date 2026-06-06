# D20 — Review Standards

**Audience:** `superpowers:code-reviewer` skill (per build-plan §1.2 the canonical reviewer for every slice). Backup: R5 panel arbitration (build-plan §1.3).
**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`acceptance.md`](acceptance.md).
**Replaces:** the codex CLI adversarial loop used in earlier hardening phases. R1-R5 here = re-invocations of `superpowers:code-reviewer` per build-plan §1.1.

## 1. Per-slice acceptance bar

A slice passes when, **and only when**:

1. The slice's diff matches the file boundary in `implementation.md` §2 (e.g. Slice 1 touches only `strands.py` skeleton + pyproject extra + new test file partial).
2. All hard gates from `acceptance.md` §1 that are runnable at this slice's commit point pass.
3. Findings count from `superpowers:code-reviewer` is zero (Blockers and Majors). Minors may be deferred to a follow-up GitHub issue with explicit rationale captured in the slice's commit message.
4. The slice maintains backwards compatibility per `implementation.md` §3. **In particular, no other integration module under `sdk/python/src/spendguard/integrations/` may be touched (only `strands.py` and the additive `strands_default_claim_estimator` function in `_default_estimator.py`).**

## 2. Slice-specific reviewer checklist

For each slice, the reviewer MUST verify each row that applies. Rows marked `Blocker` are non-negotiable; finding even one Blocker fails the slice.

### Slice 1 — Module skeleton + extra + dataclasses

| # | Check | Severity |
|---|-------|----------|
| 1.1 | Module imports `strands.hooks` at top level with `try/except ImportError` + install-hint message naming `pip install 'spendguard-sdk[strands]'`. | Blocker |
| 1.2 | `_RUN_CONTEXT` is `contextvars.ContextVar`, NEVER a plain `threading.local` or module-level dict. | Blocker |
| 1.3 | `StrandsRunContext` is a `frozen=True, slots=True` dataclass. Optional `step_id` defaults to `None`. | Major |
| 1.4 | `run_context()` is `@asynccontextmanager` with `try/finally` reset of the contextvar token. | Blocker |
| 1.5 | `_PendingInvocation` carries the FROZEN PRIMITIVE snapshot of estimator claim — never a live reference to the operator's claim object (mirrors litellm.py:373-386 pattern). | Blocker |
| 1.6 | New `[strands]` extra in pyproject.toml pins `aws-strands-agents>=1.0,<2`. No floor lift on `boto3` / `openai` / `litellm`. | Blocker |
| 1.7 | No mutation of module-level state at import time beyond logger setup. | Major |
| 1.8 | `ClaimEstimator` + `ClaimReconciler` type aliases documented with single-element v1 contract reminder. | Major |
| 1.9 | Tests U01-U05 present. | Major |

### Slice 2 — `before_invocation` reserve + DENY/DEGRADE fail-closed + stash

| # | Check | Severity |
|---|-------|----------|
| 2.1 | `register_hooks(registry)` binds BOTH `BeforeInvocationEvent` AND `AfterInvocationEvent` (slice 3 also relies on this — slice 2 cannot register only one). | Blocker |
| 2.2 | `before_invocation` reads `event.invocation.invocation_id` FIRST. Missing → `SpendGuardConfigError` with version-pin guidance. | Blocker |
| 2.3 | Estimator call is BEFORE the sidecar await; result validated for cardinality (single claim) + identity (budget_id + window_instance_id + unit_id) match. | Blocker |
| 2.4 | `request_decision` call carries `trigger="LLM_CALL_PRE"`, `route="llm.call"`, `decision_context_json` with `integration="strands"` + `model_backend=type(inv.model).__name__` + `model_id=...`. | Blocker |
| 2.5 | DENY raises `DecisionDenied` directly (do NOT wrap in another exception type). Strands runtime catches and surfaces as `HookExecutionError`; caller catches `DecisionDenied` via `__cause__`. | Blocker |
| 2.6 | DEGRADE raises `SidecarUnavailable` unless `SPENDGUARD_STRANDS_FAIL_OPEN=1`. Fail-open path returns SILENTLY (does NOT populate stash) and logs WARN. | Blocker |
| 2.7 | `outcome.reservation_ids` length asserted == 1 BEFORE stash population. >1 reservations → fail-closed with `SpendGuardConfigError`. | Blocker |
| 2.8 | Stash entry written under `self._stash[invocation_id]` with FROZEN PRIMITIVE snapshot of estimator claim (no mutable reference to operator's claim object). | Blocker |
| 2.9 | No mutation of `event` object (Strands' bus contract). | Blocker |
| 2.10 | Test U07 (load-bearing reserve test) present. | Blocker |
| 2.11 | Test U09 (DENY does not populate stash) present. | Blocker |
| 2.12 | Test U10 (DEGRADE fail-closed) present + U11 (fail-open) present. | Blocker |

### Slice 3 — `after_invocation` commit/release + exception classification

| # | Check | Severity |
|---|-------|----------|
| 3.1 | `after_invocation` reads `event.invocation_id` (Strands stamps it on AfterInvocationEvent too — pinned contract). | Blocker |
| 3.2 | Stash POP happens BEFORE the sidecar await — if RPC fails, stash is gone (TTL sweep backstop). NOT a "pop on success only" pattern. | Major |
| 3.3 | When `event.exception is not None`, `_classify_exception` distinguishes `CancelledError` → `CANCELLED` vs everything else → `FAILURE`. | Blocker |
| 3.4 | Failure-path release does NOT mask the original `event.exception`. The hook returns; Strands runtime continues to propagate the exception. | Blocker |
| 3.5 | Success path calls `claim_reconciler(inv, result)` — receives BOTH the original Invocation AND the InvocationResult. | Blocker |
| 3.6 | Reconciler exceptions are CAUGHT, logged as WARN, and fall back to `pending.estimator_claim_snapshot`. Never propagate reconciler errors to the agent. | Blocker |
| 3.7 | Reconciler claim validated against binding (mirrors slice 2 estimator validation). | Blocker |
| 3.8 | `provider_event_id` extracted via `_extract_provider_event_id(result)` — checks `result.id` then `result.model_response.id` fallback. | Major |
| 3.9 | `emit_llm_call_post` called with `outcome=SUCCESS` (success) / `FAILURE` (provider error) / `CANCELLED` (asyncio.CancelledError). | Blocker |
| 3.10 | When `_stash.pop` returns None (before_invocation never fired — fail-open path), return silently. NO error. | Blocker |
| 3.11 | Tests U15-U20 present. | Major |

### Slice 4 — Multi-backend tests (Bedrock + OpenAI + LiteLLM)

| # | Check | Severity |
|---|-------|----------|
| 4.1 | Three recorded fixtures present under `sdk/python/tests/integrations/fixtures/strands/`: `bedrock_anthropic_3_5_sonnet.json`, `openai_gpt_4o_mini.json`, `litellm_gemini_1_5_pro.json`. Each has `request` + `response` shape valid for `pytest-httpx`. | Blocker |
| 4.2 | Each backend test imports the real Strands model class (`BedrockModel`, `OpenAIModel`, `LiteLLMModel`); no monkeypatching of Strands itself. | Blocker |
| 4.3 | All upstream provider HTTP intercepted by `pytest-httpx`. NO live network calls. | Blocker |
| 4.4 | Strict-order check uses `asyncio.Event` set by fake-sidecar on `RequestDecision`; pytest-httpx callback verifies event is set before recording. | Blocker |
| 4.5 | Each backend test asserts `decision_context.model_backend == "BedrockModel" / "OpenAIModel" / "LiteLLMModel"`. | Blocker |
| 4.6 | DENY test for each backend verifies `httpx_mock.get_requests() == []`. INV-1 cross-backend. | Blocker |
| 4.7 | I06 (LiteLLM backend) additionally asserts `fake_sidecar.reserve_call_count == 1` — proves the hook layer wins over D12 shim's contextvar (single reserve, not double). | Blocker |
| 4.8 | I07 concurrent test uses `asyncio.gather` of 5 distinct invocations with distinct fake invocation_ids; verifies 5 reserves + 5 commits + no orphan stash. | Blocker |
| 4.9 | I09 model-swap test verifies stash entries do not leak across model changes mid-run. | Major |

### Slice 5 — Demo modes + docs

| # | Check | Severity |
|---|-------|----------|
| 5.1 | `DEMO_MODE=agent_real_strands` Makefile branch wires the new bootstrap module + counting provider mocks for all 3 backends; does NOT mount D12 LiteLLM SDK shim bootstrap. | Blocker |
| 5.2 | `DEMO_MODE=agent_real_strands_deny` is a SEPARATE Makefile branch — not a flag inside `agent_real_strands`. | Blocker |
| 5.3 | Bootstrap module calls `Agent(model=..., hooks=[SpendGuardHookProvider(...)])` BEFORE issuing any agent invocation. Order in file is verifiable: hook wired before first `invoke_async`. | Blocker |
| 5.4 | Demo driver `run_strands_real_mode` exercises ALL 3 backends in 3 separate steps (Bedrock + OpenAI + LiteLLM). Failure to exercise all 3 fails the SQL `model_backend` variety gate. | Blocker |
| 5.5 | `verify_step_strands.sql` includes the 6 assertions from `tests.md` §5. | Blocker |
| 5.6 | `model_backend` variety SQL assertion present and requires `COUNT(DISTINCT decision_context->>'model_backend') >= 2` in real mode (matrix proof). | Blocker |
| 5.7 | Outbox closure check runs after the demo per existing `Makefile` pattern. | Major |
| 5.8 | No regressions in adjacent demo modes (`agent_real_openai_agents_proxy`, `litellm_real`, `litellm_sdk_real`, `litellm_guardrail`, `cost_advisor`, `approval`) — those Makefile branches NOT edited. | Blocker |
| 5.9 | New page `docs/site/docs/integrations/aws-strands.md` exists and renders via `cd docs/site && npm run build`. | Blocker |
| 5.10 | Model-backend coverage matrix lists 6 backends (Bedrock / OpenAI / Anthropic / Gemini / Ollama / LiteLLM) with explicit "v1 verified" / "covered but untested" / "deferred" status per row. | Major |
| 5.11 | "Limitations" section explicitly states the 4 non-goals: per-tool budgets, streaming tokens, TS SDK, Ollama native. | Blocker |
| 5.12 | README adapter integrations table gains exactly one row for `AWS Strands`. | Major |
| 5.13 | Cross-link added from D12 docs page noting "Strands LiteLLM backend is double-covered; hook layer wins." | Minor |

## 3. Cross-cutting reviewer focus areas (every slice)

| Area | What to check | Severity if missed |
|------|---------------|--------------------|
| Backwards compatibility | Did the slice mutate `langchain.py`, `litellm.py`, `openai_agents.py`, `agt.py`, `pydantic_ai.py`? Did the slice change `_default_estimator.py` non-additively? Did the slice change an existing PyPI extra? | Blocker |
| Type hints | All new public functions carry full hints. `from __future__ import annotations` used. | Major |
| Logging | All `log.warning` / `log.info` callsites carry the `spendguard:` prefix and reference the integration module. No secrets in logs. | Major |
| Error messages | All `SpendGuardConfigError` strings name the offending function / env var / Strands version pin. | Major |
| Secret leakage | No logging of `api_key`, AWS credentials, OpenAI key, LiteLLM master_key, env var values containing `KEY` / `SECRET` / `PASSWORD` / `TOKEN`. NO logging of `event.invocation.messages` content (may contain user PII). | Blocker |
| Test isolation | Unit tests do NOT require Docker, do NOT require a running sidecar, do NOT make outbound HTTP. Each test constructs a fresh fake sidecar (no shared mutable state). | Blocker |
| Async / sync mixing | No `asyncio.run()` from inside an async function. `_RUN_CONTEXT` is contextvar (per-task), never threadlocal. | Blocker |
| Drop handles | Any new asyncio task / fixture cleans up in `finally` or pytest fixture teardown. No bare `asyncio.create_task` without await. | Major |
| Global state | Only `_RUN_CONTEXT` contextvar + module logger at module scope. NO module-level mutable dicts. `_stash` is instance-scoped on `SpendGuardHookProvider`. | Blocker |
| Dependency surface | No new runtime dependency beyond `aws-strands-agents>=1.0,<2`. Test-only deps (boto3/openai/litellm) come transitively through Strands' own extras. | Major |
| Stash hygiene | Every `_stash` write paired with a `_stash.pop()` in `after_invocation`. Stash pop happens UNCONDITIONALLY at entry, before any await. | Blocker |
| Event read-only contract | The hook reads `event.invocation` + `event.result` + `event.exception` but NEVER writes to them. Strands' bus contract enforced. | Blocker |
| Invocation-id contract | `invocation_id` is treated as REQUIRED; missing → fail-closed with version-pin error. Never silently substitute `id()` or a derived UUID. | Blocker |
| Cross-backend parity | Each backend (Bedrock/OpenAI/LiteLLM) MUST be tested with the SAME assertions (single reserve + single commit + DENY zero hits). Asymmetry = test bug. | Blocker |

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

If a slice is likely to need panel arbitration, surface it in the slice's commit message early. Likely D20 triggers:

- **Slice 2 invocation_id contract pin:** if Strands 1.x changes the `Invocation` attribute name (e.g. `invocation_id` → `id` or `invocation_uuid`), panel decides whether to defensively check both names OR hard-pin and force operator upgrade.
- **Slice 3 reconciler-fallback semantics:** the spec says reconciler exceptions fall back to estimator snapshot + WARN. Panel may decide this should fail-closed instead (lossy commit on bug). Default = fall back (matches LangChain integration).
- **Slice 4 LiteLLMModel double-reserve concern:** if I06 finds the D12 contextvar guard doesn't fire (e.g. Strands' LiteLLMModel runs in a different async task than the hook), panel decides whether to (a) accept the double-reserve as acceptable cost or (b) add a `_strands_in_flight` guard in `strands.py`.
- **Slice 5 demo backend mocking burden:** if mocking all 3 backends (Bedrock + OpenAI + LiteLLM) in-proc proves brittle (Bedrock SDK uses SigV4 signing), panel decides whether to relax the variety gate to 2 backends OR add a recorded-cassette playback shim.
- **Slice 5 model_backend variety SQL:** if the SQL gate proves too tight in CI (e.g. one backend's fixture loads slower and times out), panel decides whether to relax to `>= 1` OR add a longer demo timeout.

## 6. Slice-merge order is fixed

Per dependency in `implementation.md` §2: **Slice 1 → 2 → 3 → 4 → 5**, never reorder. Slice 2 + 3 cannot merge in parallel because `after_invocation` (S3) depends on stash structures defined in `before_invocation` (S2). Slice 4 depends on S2+S3 for full provider implementation. Slice 5 depends on S4 for accurate docs/coverage-matrix claims.

## 7. Final reviewer override

If the reviewer believes the spec itself is wrong (e.g. invocation_id contract assumption, hook layer vs model wrap choice, fail-closed semantics on DEGRADE), flag it as a Blocker on the relevant slice with rationale referencing `design.md` §5 "Key decisions" — do not silently deviate. Spec changes route through Staff+ panel per build-plan §1.3.

## 8. Spec-pair consistency check (D12 + D20)

D20 ships AFTER D12. The reviewer MUST verify on every D20 slice:

- D12 source files (`litellm.py`, `litellm_shim.py`, `examples/litellm-proxy-composite/`) are NOT modified.
- D12 PyPI extras (`litellm`, `litellm-shim`) NOT modified.
- D12 demo modes (`litellm_real`, `litellm_deny`, `litellm_direct`, `litellm_sdk_real`, `litellm_sdk_deny`) NOT modified.
- The model-backend coverage matrix in D20's docs page acknowledges D12 as the "LiteLLM SDK shim" peer path for non-Strands callers.

If D12 has not yet been merged at the time of D20 slice review, the LiteLLM-backend tests (I03 + I06) MAY skip with `pytest.importorskip("spendguard.integrations.litellm_shim")` — but slice 4 cannot fully ship until D12 lands. Track as a slice-blocker if relevant.

## 9. Spec-pair consistency check (D19 + D20)

D19 (Google ADK) is a sibling Python adapter. The reviewer MUST verify:

- D20 does NOT depend on D19; the two adapters ship independently.
- If D19 ships first, D20's docs page may cross-link D19 as a "sibling Python hook-pattern adapter" for operators evaluating frameworks.
- If D20 ships first, the reverse cross-link is added retroactively when D19 ships.
