# D12 — Review Standards

**Audience:** `superpowers:code-reviewer` skill (per build-plan §1.2 the canonical reviewer for every slice). Backup: R5 panel arbitration (build-plan §1.3).
**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`acceptance.md`](acceptance.md).
**Replaces:** the codex CLI adversarial loop used in earlier hardening phases. R1-R5 here = re-invocations of `superpowers:code-reviewer` per build-plan §1.1.

## 1. Per-slice acceptance bar

A slice passes when, **and only when**:

1. The slice's diff matches the file boundary in `implementation.md` §2 (e.g. Slice 1 touches only `litellm_shim.py` skeleton + new test file partial).
2. All hard gates from `acceptance.md` §1 that are runnable at this slice's commit point pass.
3. Findings count from `superpowers:code-reviewer` is zero (Blockers and Majors). Minors may be deferred to a follow-up GitHub issue with explicit rationale captured in the slice's commit message.
4. The slice maintains backwards compatibility per `implementation.md` §3. **In particular, D11 `litellm_guardrail.py` must NEVER be touched by D12 slices.**

## 2. Slice-specific reviewer checklist

For each slice, the reviewer MUST verify each row that applies. Rows marked `Blocker` are non-negotiable; finding even one Blocker fails the slice.

### Slice 1 — Module skeleton + install/uninstall state machine + recursion guard

| # | Check | Severity |
|---|-------|----------|
| 1.1 | Module imports `litellm` at top level with `try/except ImportError` + install-hint message. | Blocker |
| 1.2 | `_INSTALL_STATE: _InstallState | None` is module-level; no class-attached state. | Blocker |
| 1.3 | `_IN_FLIGHT` is `contextvars.ContextVar`, NEVER a plain `threading.local` or module-level bool. | Blocker |
| 1.4 | `install()` checks `_INSTALL_STATE` BEFORE calling `_patch_*` helpers. Patching never starts when state is non-None. | Blocker |
| 1.5 | `_compute_config_signature` is deterministic (uses `id()` of resolver/reconciler callables + bool params); two `install()` calls with the same callables hash equal. | Major |
| 1.6 | `uninstall()` iterates `state.originals` in **reverse** so Router subclasses restore before `Router` itself. | Blocker |
| 1.7 | `is_installed()` is a 1-LOC truthiness check on `_INSTALL_STATE`. No side effects. | Major |
| 1.8 | New exception types `SpendGuardShimAlreadyInstalled` + `SpendGuardShimSyncInAsyncContext` both inherit `SpendGuardConfigError`. | Major |
| 1.9 | No mutation of module-level state at import time beyond logger setup. | Major |
| 1.10 | Tests U02-U06 + U21 + U22 present. | Major |

### Slice 2 — Patch acompletion + atext_completion

| # | Check | Severity |
|---|-------|----------|
| 2.1 | Wrapper checks `_IN_FLIGHT.get()` FIRST, before any other work. Re-entry short-circuits to the saved `original`. | Blocker |
| 2.2 | Wrapper saves the original via `state.originals.append((litellm, "acompletion", original))` BEFORE assigning the wrapper. | Blocker |
| 2.3 | Wrapper sets `_IN_FLIGHT` via `token = _IN_FLIGHT.set(True)` + `try/finally` reset. NEVER a plain assignment without reset. | Blocker |
| 2.4 | `state.core` is called with `_original_acompletion=original` kwarg so the core dispatches to the saved original (not `litellm.acompletion`). | Blocker |
| 2.5 | The 5-LOC patch to `SpendGuardDirectAcompletion.__call__` accepts `_original_acompletion: Callable | None = None` and uses it INSTEAD OF `litellm.acompletion` when set. | Blocker |
| 2.6 | When `_original_acompletion` is None (existing callers), behavior is bit-for-bit identical to today — pinned by a regression test (`test_direct_acompletion_unchanged_baseline`). | Blocker |
| 2.7 | `atext_completion` wrapper mirrors `acompletion` exactly (same recursion guard, same delegation). | Major |
| 2.8 | No mutation of the user's `**kwargs` dict (no `del`, no key injection). | Blocker |
| 2.9 | Test U17 (load-bearing INV-2 ordering proof) present. Test fails if order is `["provider", "reserve"]`. | Blocker |
| 2.10 | Test U21 (recursion guard) present. | Blocker |

### Slice 3 — Patch sync completion + text_completion

| # | Check | Severity |
|---|-------|----------|
| 3.1 | Sync wrapper checks `asyncio.get_running_loop()` via `try/except RuntimeError`. If a loop IS running, raises `SpendGuardShimSyncInAsyncContext`. | Blocker |
| 3.2 | Outside a loop, sync wrapper bridges via `asyncio.run(_async_dispatch(...))`. NEVER via `loop.run_until_complete` on a manually-created loop (would conflict with future re-installs). | Blocker |
| 3.3 | `SpendGuardShimSyncInAsyncContext` error message names the offending function AND points caller at `acompletion`. | Major |
| 3.4 | `text_completion` patch mirrors `completion` patch (same checks). | Major |
| 3.5 | Test U10 (async-context guard) present and verifies the error message contains the hint. | Major |
| 3.6 | `patch_sync=False` install option skips both `completion` and `text_completion` patches. | Major |

### Slice 4 — Patch Router

| # | Check | Severity |
|---|-------|----------|
| 4.1 | `Router.acompletion` is patched at the **class level**, not on instances. | Blocker |
| 4.2 | Router wrapper takes `self` as first arg (it is a method), forwards properly. | Blocker |
| 4.3 | Subclass walk uses `Router.__subclasses__()`; only re-patches subclasses that have `acompletion` in their `__dict__` (i.e. they overrode the parent method). Subclasses that inherit pick up the patched parent automatically via MRO. | Blocker |
| 4.4 | Each subclass patch is appended to `state.originals` so `uninstall()` restores them too. | Blocker |
| 4.5 | `patch_router=False` install option skips all Router patching. | Major |
| 4.6 | Recursion guard works inside Router too: if `Router.acompletion` internally calls `litellm.acompletion` (LiteLLM does this for some routes), the second reserve is short-circuited. | Blocker |
| 4.7 | Tests U12-U15 present. | Major |

### Slice 5 — Unit tests with mock litellm + pytest-httpx ordering

| # | Check | Severity |
|---|-------|----------|
| 5.1 | Every test that calls `install()` wraps in `try/finally` calling `uninstall()`. Fixture `shim_clean` enforces this via `addfinalizer`. | Blocker |
| 5.2 | No test relies on `litellm` real HTTP — all mocked via `pytest-httpx` or `monkeypatch.setattr`. | Blocker |
| 5.3 | U17 ordering test uses a list of recorded labels and asserts EXACTLY `["reserve", "provider"]`. Not "reserve is in the list" — strict order. | Blocker |
| 5.4 | U18 DENY test asserts `mock_acompletion.call_count == 0` AND `httpx_mock.get_requests() == []`. Both. | Blocker |
| 5.5 | U22 isolated contextvars test runs 2 `asyncio.gather` tasks concurrently and asserts each saw exactly 1 reserve and no cross-task `_IN_FLIGHT` bleed. | Blocker |
| 5.6 | U23 success commit test verifies `provider_event_id` came from `response.id`, not estimator. | Major |
| 5.7 | U24 + U25 exception/cancellation tests verify `outcome=FAILURE` / `outcome=CANCELLED` reach sidecar, AND original exception re-raises with intact traceback. | Major |
| 5.8 | All 22+ unit tests pass under `pytest -v`. | Blocker |

### Slice 6 — Integration with real litellm + pytest-httpx + CrewAI smoke

| # | Check | Severity |
|---|-------|----------|
| 6.1 | Integration tests import `litellm` for real (no `monkeypatch.setattr` on litellm). | Blocker |
| 6.2 | All upstream provider HTTP intercepted by `pytest-httpx`. NO live network calls. | Blocker |
| 6.3 | I01 strict-order check uses `asyncio.Event` set by fake-sidecar; pytest-httpx callback verifies event is set before recording. | Blocker |
| 6.4 | I02 DENY test verifies `httpx_mock.get_requests()` is empty. | Blocker |
| 6.5 | I06 baseline test verifies bit-for-bit clean uninstall — `litellm.acompletion is _ORIGINAL_REF`. | Blocker |
| 6.6 | T01-T03 (CrewAI / DSPy transitive) use `pytest.importorskip` so suite passes even when frameworks not installed. | Major |
| 6.7 | T01 asserts CrewAI agent loop triggered ≥1 reserve via the shim. This is INV-8. | Blocker |
| 6.8 | T02 DENY test for CrewAI verifies kickoff raised AND no OpenAI HTTP recorded. | Blocker |

### Slice 7 — Demo modes + docs

| # | Check | Severity |
|---|-------|----------|
| 7.1 | `DEMO_MODE=litellm_sdk_real` Makefile branch wires the new bootstrap module + counting provider; does NOT mount D11's `litellm_guardrail/proxy_config.yaml`. | Blocker |
| 7.2 | `DEMO_MODE=litellm_sdk_deny` is a separate Makefile branch — not a flag inside `litellm_sdk_real`. | Blocker |
| 7.3 | Bootstrap module calls `litellm_shim.install(...)` BEFORE issuing any litellm call. Order in the file is verifiable: install precedes first `await litellm.acompletion`. | Blocker |
| 7.4 | Demo driver `run_litellm_sdk_real_mode` step 3 (TRANSITIVE) inline-creates a CrewAI Agent + asserts reserve was triggered by the kickoff call. | Blocker |
| 7.5 | `verify_step_litellm_sdk.sql` includes the 6 assertions from `tests.md` §5. | Blocker |
| 7.6 | Stub-counter delta assertion (INV-1) is present and uses `decision_context->>'expected_allow_count'` field. | Blocker |
| 7.7 | Outbox closure check runs after the demo per existing `Makefile` pattern. | Major |
| 7.8 | No regressions in adjacent demo modes (`litellm_real`, `litellm_deny`, `litellm_direct`, `litellm_guardrail`) — those Makefile branches not edited. | Blocker |
| 7.9 | New page `docs/site/docs/integrations/litellm-sdk-shim.md` exists and renders via `cd docs/site && npm run build`. | Blocker |
| 7.10 | Decision matrix lists 3 paths (egress proxy D02 / guardrail D11 / SDK shim D12) with explicit "when to use" rows. | Major |
| 7.11 | "Limitations" section explicitly states INV-3 (fail-closed) + token-by-token streaming caveat + sync-in-async refusal + #8842 not closed upstream. | Blocker |
| 7.12 | README adapter integrations table gains exactly one row for `LiteLLM SDK shim`. | Major |
| 7.13 | Cross-link added from D11 page noting D12 covers direct SDK. | Minor |

## 3. Cross-cutting reviewer focus areas (every slice)

| Area | What to check | Severity if missed |
|------|---------------|--------------------|
| Backwards compatibility | Did the slice mutate `litellm_guardrail.py`? Did the slice change `examples/litellm-proxy-composite/`? Did the slice change an existing PyPI extra? Did the diff to `litellm.py` exceed 25 lines (G14)? | Blocker |
| Type hints | All new public functions carry full hints. `from __future__ import annotations` used. | Major |
| Logging | All `log.warning` / `log.info` callsites carry the `spendguard_litellm_shim:` prefix. No secrets in logs. | Major |
| Error messages | All `SpendGuardConfigError` strings name the offending function / env var. `SpendGuardShimSyncInAsyncContext` hint points at `acompletion`. | Major |
| Secret leakage | No logging of `api_key`, `litellm_kwargs.get("api_key")`, `user_api_key_dict`, `master_key`, env var values containing `KEY` / `SECRET` / `PASSWORD` / `TOKEN`. | Blocker |
| Test isolation | Unit tests do NOT require Docker, do NOT require a running sidecar, do NOT make outbound HTTP. Each `install()` call paired with `uninstall()` in `finally`. | Blocker |
| Async / sync mixing | No `asyncio.run()` from inside an async function. No `loop.run_until_complete` ever. `_IN_FLIGHT` is contextvar (per-task), never threadlocal. | Blocker |
| Drop handles | Any new asyncio task / fixture cleans up in `finally` or pytest fixture teardown. No bare `asyncio.create_task` without await. | Major |
| Global state | `_INSTALL_STATE` is the ONLY global. No module-level mutable state beyond it + the logger. | Blocker |
| Dependency surface | No new runtime dependency beyond `litellm>=1.50` + `pytest-httpx>=0.30` (test-only). No new compile-time deps. | Major |
| Monkey-patch hygiene | EVERY patched attribute has a corresponding entry in `state.originals` for `uninstall()`. EVERY restore uses `setattr`. EVERY patch verifies the attribute exists before replacing. | Blocker |
| Recursion guard correctness | `_IN_FLIGHT` set BEFORE calling `state.core`. `_IN_FLIGHT` reset in `finally`. Token-based reset (never plain `set(False)` which would clobber an outer frame's True). | Blocker |

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

If a slice is likely to need panel arbitration, surface it in the slice's commit message early. Likely D12 triggers:

- **Slice 2 `_original_acompletion` kwarg on `SpendGuardDirectAcompletion`:** if the kwarg conflicts with a future LiteLLM-passthrough kwarg of the same name, panel decides whether to rename to a `_spendguard_` prefixed kwarg or move the dispatch to a constructor injection.
- **Slice 4 Router subclass walk:** if `Router.__subclasses__()` returns weakly-referenced subclasses that get GC'd mid-install, panel decides whether to skip GC'd refs or hold strong refs in `state.patched_subclasses` (current spec says hold).
- **Slice 5 U17 ordering assertion brittleness:** if `monkeypatch.setattr` + ordering list approach is flaky under `pytest-xdist`, panel decides whether to fall back to `asyncio.Event` wire-level proof from the integration suite.
- **Slice 6 T01 CrewAI version drift:** if CrewAI's internal LiteLLM path changes (e.g. moves to a Pydantic-AI re-export), panel decides whether to pin CrewAI version OR widen the assertion to "any litellm-routed call".
- **Slice 7 INV-1 stub-counter delta:** if `decision_context->>'expected_allow_count'` field is hard to thread through the demo, panel decides whether to use a side-channel file marker or accept a relaxed assertion ("≥0 stub hits on DENY-only sub-step").

## 6. Slice-merge order is fixed

Per dependency in `implementation.md` §2: **Slice 1 → 2 → 3 → 4 → 5 → 6 → 7**, never reorder. Slices 3+4 may potentially merge in parallel with slice 2 after slice 1, but the linear sequence is the safe default given the file-level overlap on `_patch_*` helpers. Slice 6 depends on slice 5's fixture infrastructure. Slice 7 depends on slices 5+6 for accurate docs claims.

## 7. Final reviewer override

If the reviewer believes the spec itself is wrong (e.g. recursion-guard design, idempotent-install semantics, sync-in-async raise vs bridge), flag it as a Blocker on the relevant slice with rationale referencing `design.md` §5 "Key decisions" — do not silently deviate. Spec changes route through Staff+ panel per build-plan §1.3.

## 8. Spec-pair consistency check (D11 + D12)

D12 ships AFTER D11. The reviewer MUST verify on every D12 slice:

- D11 source files (`litellm_guardrail.py`, `examples/litellm-proxy-composite/`, `deploy/demo/litellm_guardrail/`) are NOT modified.
- D11 PyPI extra `litellm-guardrail` is NOT modified.
- D11 demo modes (`litellm_guardrail`) NOT modified.
- The 3-path decision matrix in D12's docs page lists D11 as a peer path (not deprecated).

If D11 has not yet been merged at the time of D12 slice review, the reviewer flags the dependency violation as a Blocker and the implementer waits for D11 to land.
