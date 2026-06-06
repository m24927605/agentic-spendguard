# D11 — Review Standards

**Audience:** `superpowers:code-reviewer` skill (per build-plan §1.2 the canonical reviewer for every slice). Backup: R5 panel arbitration (build-plan §1.3).
**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`acceptance.md`](acceptance.md).
**Replaces:** the codex CLI adversarial loop used in earlier hardening phases. R1-R5 here = re-invocations of `superpowers:code-reviewer` per build-plan §1.1.

## 1. Per-slice acceptance bar

A slice passes when, **and only when**:

1. The slice's diff matches the file boundary in `implementation.md` §2 (e.g. Slice 1 touches only `litellm_guardrail.py` skeleton + new test file).
2. All hard gates from `acceptance.md` §1 that are runnable at this slice's commit point pass.
3. Findings count from `superpowers:code-reviewer` is zero (Blockers and Majors). Minors may be deferred to a follow-up GitHub issue with explicit rationale captured in the slice's commit message.
4. The slice maintains backwards compatibility per `implementation.md` §3.

## 2. Slice-specific reviewer checklist

For each slice, the reviewer MUST verify each row that applies. Rows marked `Blocker` are non-negotiable; finding even one Blocker fails the slice.

### Slice 1 — `SpendGuardGuardrail` class skeleton

| # | Check | Severity |
|---|-------|----------|
| 1.1 | Module imports `CustomGuardrail` lazily inside a `try/except ImportError` block with the install hint message. | Blocker |
| 1.2 | `SpendGuardGuardrail.__init__` calls `super().__init__(guardrail_name=...)`. | Blocker |
| 1.3 | `_delegate` is a `_LoopBoundCallback`, never inherits from `CustomGuardrail`. Composition, not multiple inheritance. | Blocker |
| 1.4 | No mutation of module-level state at import time beyond logger setup. | Major |
| 1.5 | Test U01 covers the ImportError shape. | Major |

### Slice 2 — `async_pre_call_hook`

| # | Check | Severity |
|---|-------|----------|
| 2.1 | Hook implementation is **pure delegation** — fewer than 5 LOC of body excluding signature. No new error handling. | Blocker |
| 2.2 | Raises (`DecisionDenied`, `SidecarUnavailable`, `SpendGuardConfigError`) propagate; no `except` swallowing. | Blocker |
| 2.3 | Return value forwarded verbatim from delegate. No `data` mutation. | Blocker |
| 2.4 | Tests U12-U14 present and cover delegate-call, deny-propagation, degrade-propagation. | Major |

### Slice 3 — Post-call hook signature translation

| # | Check | Severity |
|---|-------|----------|
| 3.1 | `async_post_call_success_hook` constructs kwargs dict with `litellm_call_id` populated from `data`. If `data["litellm_call_id"]` is missing, the delegate's existing `_get_stash` returns None → no-op (no exception). | Blocker |
| 3.2 | `async_post_call_failure_hook` populates `kwargs["exception"] = original_exception` before forwarding. The delegate's `_classify_failure` reads it. | Blocker |
| 3.3 | `start_time` / `end_time` passed as `None` is safe — pinned by a regression test (U17 path or equivalent). | Major |
| 3.4 | INV-5 covered: when `response.usage` is None, the delegate's streaming-fallback fires + WARN log + estimator-snapshot commit. | Blocker |
| 3.5 | No new exception types introduced. | Minor |

### Slice 4 — Env-driven default factory

> **SLICE-PHASING (added 2026-06-07 in SLICE 4 R2)**: SLICE 4 ships the `from_env` / `from_kwargs` / `from_config` factory layer for the 5-var subset (`SPENDGUARD_TENANT_ID`, `SPENDGUARD_SIDECAR_ADDRESS`, `SPENDGUARD_API_KEY`, `SPENDGUARD_DISABLED`, `SPENDGUARD_PROXY_TIMEOUT_MS`) which exercises the factory machinery without requiring resolver wiring. SLICE 4 R1 reviewer carved off the resolver-module wiring (`SPENDGUARD_RESOLVER_MODULE`, `SPENDGUARD_BUDGET_ID`, `SPENDGUARD_WINDOW_INSTANCE_ID`, `SPENDGUARD_UNIT_ID` + pricing-version vars + `_load_resolver_from_env`/`_load_reconciler_from_env` + `BudgetBinding` validation) into a SLICE 4b that lands before SLICE 5's `proxy_config.yaml`, since the SLICE 5 yaml loader dispatches through the same loader functions. Gates 4.2 / 4.3 / 4.5 / 4.6 apply only after SLICE 4b lands; SLICE 4b R1 will re-apply them. SLICE 4 gates that remain in-scope: 4.1 (missing env var raises with var name), 4.4 (`_default_estimator` reuse), and the 5-var subset of 4.7 (tests covering the SLICE 4 vars).

| # | Check | Severity | Phase |
|---|-------|----------|-------|
| 4.1 | Missing required env var raises `SpendGuardConfigError` at construction time, message names the var. | Blocker | SLICE 4 |
| 4.2 | `SPENDGUARD_RESOLVER_MODULE=pkg.mod:fn` path: bad module / missing attr both raise `SpendGuardConfigError`. **SLICE-PHASING (added 2026-06-07 in SLICE 4 R2)**: this gate applies only after SLICE 4b lands. SLICE 4 R1 reviewer carved off the resolver-module wiring (`SPENDGUARD_RESOLVER_MODULE`, `SPENDGUARD_BUDGET_ID`, `SPENDGUARD_WINDOW_INSTANCE_ID`, `SPENDGUARD_UNIT_ID` + pricing-version vars + `_load_resolver_from_env`/`_load_reconciler_from_env` + `BudgetBinding` validation) into SLICE 4b. SLICE 4b R1 will re-apply this gate. | Blocker | SLICE 4b |
| 4.3 | When `SPENDGUARD_RESOLVER_MODULE` is set, single-tenant env vars are NOT consulted (verified by U08 leaving them unset). **SLICE-PHASING (added 2026-06-07 in SLICE 4 R2)**: this gate applies only after SLICE 4b lands. The single-tenant env vars (`SPENDGUARD_BUDGET_ID` / `SPENDGUARD_WINDOW_INSTANCE_ID` / `SPENDGUARD_UNIT_ID`) and the resolver-module dispatch both live in the SLICE 4b carve-off; SLICE 4 itself does not consult them. SLICE 4b R1 will re-apply this gate. | Blocker | SLICE 4b |
| 4.4 | `_default_estimator` is reused — no duplicate estimator logic. | Major | SLICE 4 |
| 4.5 | `BudgetBinding` validation: empty `budget_id` / `window_instance_id` / `unit_id` rejected at factory time (mirror `litellm.py` lines 306-315). **SLICE-PHASING (added 2026-06-07 in SLICE 4 R2)**: this gate applies only after SLICE 4b lands. `BudgetBinding` field validation depends on the budget-binding env vars (`SPENDGUARD_BUDGET_ID` / `SPENDGUARD_WINDOW_INSTANCE_ID` / `SPENDGUARD_UNIT_ID`) that SLICE 4b owns; SLICE 4 ships only the factory plumbing scaffolding. SLICE 4b R1 will re-apply this gate. | Blocker | SLICE 4b |
| 4.6 | Pricing version env vars parsed into a `PricingFreeze` consistent with `examples/litellm-proxy-composite/spendguard_litellm_proxy_callback.py` field-by-field. **SLICE-PHASING (added 2026-06-07 in SLICE 4 R2)**: this gate applies only after SLICE 4b lands. The 4 pricing-version env vars are part of the SLICE 4b carve-off (alongside the resolver-module wiring) since `PricingFreeze` construction is part of the same `_load_resolver_from_env` helper. SLICE 4b R1 will re-apply this gate. | Major | SLICE 4b |
| 4.7 | Tests U04-U11 present. (U04-U05 + the 5-var-subset of U11 land in SLICE 4; U06-U10 + the remaining U11 paths land in SLICE 4b — see `tests.md` §2.2 SLICE-PHASING notes.) | Major | Split |

### Slice 5 — `proxy_config.yaml` registry entry + PyPI extra

| # | Check | Severity |
|---|-------|----------|
| 5.1 | `mode: pre_call` literal — not `during_call`, not `logging_only`. | Blocker |
| 5.2 | New PyPI extra `litellm-guardrail = ["litellm[proxy]>=1.55.0"]` ONLY. Existing `litellm` extra unchanged (floor stays at 1.50). | Blocker |
| 5.3 | `default_on: true` set in registry entry. | Major |
| 5.4 | Bootstrap module validates required env vars at import time so misconfig surfaces before first request. | Major |
| 5.5 | No changes to `examples/litellm-proxy-composite/` files. | Major |

### Slice 6 — Demo mode

| # | Check | Severity |
|---|-------|----------|
| 6.1 | `DEMO_MODE=litellm_guardrail` branch wires the **new** `litellm_guardrail/proxy_config.yaml`, not the legacy `litellm_proxy/` config. | Blocker |
| 6.2 | New compose service `litellm-guardrail-proxy` is independent (no port / volume collision with `litellm-proxy`). | Blocker |
| 6.3 | Demo driver step 2 (DENY) asserts **provider stub counter unchanged**. INV-1. | Blocker |
| 6.4 | Demo driver step 1 (ALLOW) asserts the SpendGuard reservation row exists with `decision_context.mode = 'proxy'` AND was created **before** the stub hit timestamp. INV-2. | Blocker |
| 6.5 | `verify_step_litellm_guardrail.sql` includes the 5 assertions from `tests.md` §4. | Blocker |
| 6.6 | Outbox-closure check runs after the demo per existing `Makefile` pattern. | Major |
| 6.7 | Driver writes the success line `litellm_guardrail ALL 3 steps PASS (ALLOW + DENY + STREAM)` exactly. | Major |
| 6.8 | No regressions in adjacent demo modes (`litellm_real`, `litellm_deny`, `litellm_direct`) — Makefile branches not edited. | Blocker |

### Slice 7 — Public docs

> **SLICE-PHASING (added 2026-06-07)**: SLICE 7 docs page ✓ shipped 2026-06-07. The site now lives under `docs/site-v2/` (Starlight); the canonical page path is `docs/site-v2/src/content/docs/docs/integrations/litellm-proxy.mdx` rendered at `/docs/integrations/litellm-proxy/`. Gate 7.1 reads against the new path. Gates 7.2-7.5 unchanged.

| # | Check | Severity |
|---|-------|----------|
| 7.1 | New page `docs/site-v2/src/content/docs/docs/integrations/litellm-proxy.mdx` exists and renders via `cd docs/site-v2 && npm run build`. | Blocker |
| 7.2 | Decision matrix lists 3 paths (forked callback / guardrail / egress proxy) with explicit "when to use" rows. | Major |
| 7.3 | "Limitations" section explicitly states INV-5 (end-of-stream commit), no token-by-token cap, no #8842 closure. | Blocker |
| 7.4 | README adapter integrations table gains exactly one row. | Major |
| 7.5 | Cross-link to D12 (when shipped) noted as a future row, not yet present. | Minor |

## 3. Cross-cutting reviewer focus areas (every slice)

| Area | What to check | Severity if missed |
|------|---------------|--------------------|
| Backwards compatibility | Did the slice mutate `litellm.py`? Did the slice change `examples/litellm-proxy-composite/`? Did the slice change an existing PyPI extra? | Blocker |
| Type hints | All new public functions carry full hints. `from __future__ import annotations` used. | Major |
| Logging | All `log.warning` / `log.info` callsites carry the `spendguard:` prefix matching `litellm.py`. | Minor |
| Error messages | All `SpendGuardConfigError` strings name the offending env var or call site. | Major |
| Secret leakage | No logging of `user_api_key_dict`, `master_key`, env var values containing `KEY` / `SECRET` / `PASSWORD`. | Blocker |
| Test isolation | Unit tests do NOT require Docker, do NOT require a running sidecar, do NOT make outbound HTTP. | Blocker |
| Async / sync mixing | No `asyncio.run()` from inside an async hook. No blocking IO on the hook hot path. | Blocker |
| Drop handles | Any new asyncio task / fixture cleans up in `finally` or `pytest` fixture teardown. | Major |
| Dependency surface | No new runtime dependency added beyond `litellm[proxy]>=1.55`. | Major |

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

If a slice is likely to need panel arbitration, surface it in the slice's commit message early. Likely D11 triggers:

- **Slice 3 INV-5 streaming-fallback path:** post-call hook signature drift between LiteLLM versions might require reaching into `_async_log_success_streaming` internals; if so, panel decides whether to lift it to a public delegate API instead.
- **Slice 4 `SPENDGUARD_RESOLVER_MODULE` import semantics:** if operator-supplied module raises at import time, panel decides whether to fail-closed at proxy boot (current spec) or fail-deferred to first request.
- **Slice 6 INV-2 strict-order proof:** if `asyncio.Event` approach is brittle, panel decides whether to fall back to comparing wall-clock timestamps with a tolerance window.

## 6. Slice-merge order is fixed

Per dependency in `implementation.md` §2: **Slice 1 → 2 → 3 → 4 → 5 → 6 → 7**, never reorder. Slice 6 depends on Slice 5's `proxy_config.yaml`; Slice 7 depends on Slice 5+6 for accurate docs.

## 7. Final reviewer override

If the reviewer believes the spec itself is wrong (e.g. composition vs inheritance, mode literal, default_on choice), flag it as a Blocker on the relevant slice with rationale referencing `design.md` §5 "Key decisions" — do not silently deviate. Spec changes route through Staff+ panel per build-plan §1.3.

