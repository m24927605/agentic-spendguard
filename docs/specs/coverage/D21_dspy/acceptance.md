# D21 — Acceptance

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`review-standards.md`](review-standards.md).

D21 ships when **every gate** in §1 is green AND the §2 invariants are unbreakable AND the §3 ship-checklist is fully checked. Per build-plan §3 "100% feasible" rule: every gate is runnable in the repo's current state, no third-party action required.

## 1. Hard gates

### G1 — Build + import

```bash
cd sdk/python && pip install -e '.[dspy]'
python -c "from spendguard.integrations.dspy import SpendGuardDSPyCallback, BudgetBinding, RunContext; print('ok')"
```

Expected: prints `ok`. No `ImportError`.

### G2 — Unit suite (mock dspy)

```bash
cd sdk/python && pytest tests/integrations/test_dspy.py -v
```

Expected: 16 tests pass (count from `tests.md` §2; final count may rise during implementation but never fall below 14).

### G3 — Integration suite (real dspy + pytest-httpx)

```bash
cd sdk/python && pip install pytest-httpx dspy-ai
pytest tests/integrations/test_dspy_real.py -v
```

Expected: 5 tests pass. NO outbound HTTP traffic leaves the test process (all calls intercepted by `pytest-httpx`).

### G4 — Existing LangChain tests still pass (regression)

```bash
cd sdk/python && pytest tests/integrations/test_langchain_default_e2e.py -v
```

Expected: unchanged baseline. D21 does NOT touch `langchain.py`.

### G5 — Existing default-estimator tests still pass

```bash
cd sdk/python && pytest tests/integrations/test_default_estimators.py -v
```

Expected: unchanged baseline. D21 reuses `_default_estimator.py` shape but does not modify it.

### G6 — Demo mode `agent_real_dspy` passes

```bash
make demo-down
make demo-up DEMO_MODE=agent_real_dspy
```

Expected:

- All compose services reach healthy.
- Demo driver exits 0.
- stdout contains `[demo] agent_real_dspy ALL 3 steps PASS`.
- stdout contains `D21_DSPY OK: ...` from SQL verification.
- stdout contains `D21_DSPY OK: canonical_events` from the outbox closure verifier.
- The step 1 ALLOW (`dspy.ChainOfThought`) prints a non-empty `result.answer`.
- The step 2 DENY substep shows the counting-provider stub registered ZERO hits while the deny was being issued.
- The step 3 CUSTOM-LM substep demonstrates the direct (non-LiteLLM) path works.

### G7 — Demo tear-down clean

```bash
make demo-down
```

Expected: no orphaned containers, no orphaned volumes, exit 0.

### G8 — Public docs page renders

```bash
cd docs/site && npm run build
```

Expected: build succeeds. `docs/site/dist/docs/integrations/dspy/index.html` exists. Page contains:

- "1-minute setup" code snippet with `dspy.configure(callbacks=[SpendGuardDSPyCallback(...)])` call.
- 2-path decision matrix with 2 rows: D12 LiteLLM shim (transitive) / D21 BaseCallback (direct). Each row has a "when to use" cell.
- "Limitations" section explicitly listing the 4 non-goals from `design.md` §3 (token-by-token streaming / `on_tool_*` / `on_module_*` / async callbacks).
- Cross-link to D12 docs page.

### G9 — README index entry present

```bash
grep -F "spendguard-sdk[dspy]" README.md
```

Expected: exactly one row in the adapter integrations table for `DSPy`.

### G10 — PyPI extra wired

```bash
grep -F "dspy = " sdk/python/pyproject.toml
```

Expected: `dspy = ["dspy-ai>=2.6,<3.0"]` extra defined. Existing extras unchanged.

### G11 — No proto / no schema / no Rust changes

```bash
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '\.(proto|sql|rs)$' | grep -v '^deploy/demo/verify_step_agent_real_dspy\.sql$'
```

Expected: empty output. Only the new demo verify SQL is in the .sql allow-list; nothing else proto/SQL/Rust touched.

### G12 — No mutation of existing integration modules

```bash
git diff main..HEAD -- sdk/python/src/spendguard/integrations/langchain.py sdk/python/src/spendguard/integrations/pydantic_ai.py sdk/python/src/spendguard/integrations/openai_agents.py sdk/python/src/spendguard/integrations/litellm.py sdk/python/src/spendguard/integrations/agt.py sdk/python/src/spendguard/integrations/_default_estimator.py | wc -l
```

Expected: `0` changed lines. D21 is a strictly net-new module.

### G13 — Shared contextvar object identity

```bash
cd sdk/python && python -c "
from spendguard._litellm_shim import _IN_FLIGHT as A
from spendguard.integrations.dspy import _SHIM_IN_FLIGHT as B
assert A is B, 'shared contextvar is not the same object'
print('ok')
"
```

Expected: prints `ok`. The two contextvars are the same object — guarantees D12 + D21 coexistence works at runtime.

### G14 — Spec-pair forward compatibility with D12

```bash
grep -F "from spendguard._litellm_shim" sdk/python/src/spendguard/integrations/dspy.py
```

Expected: import statement present. When D12 ships, its `litellm_shim.py` will import the SAME `_IN_FLIGHT` from `spendguard._litellm_shim`; reviewer verifies this contract holds in D12 review even though D12 ships separately.

## 2. Invariants (must never regress)

| ID | Invariant | How verified |
|----|-----------|--------------|
| INV-1 | **DENY never hits the upstream provider.** Counting stub (demo) and `pytest-httpx` mock (unit/integration) MUST register zero hits across all DENY decisions. | U10 + I03 + demo deny step + SQL stub-counter delta |
| INV-2 | **Pre-call reservation precedes provider HTTP.** Sidecar `RequestDecision` RPC fires before the DSPy LM dispatches its provider call. | U09 (strict order list) + I01 (event-based wire-level order) |
| INV-3 | **Fail-closed default.** Sidecar DEGRADE → `SidecarUnavailable` raised → provider not called. Only `SPENDGUARD_DSPY_FAIL_OPEN=1` permits otherwise. | U11 |
| INV-4 | **No `_PENDING` leak.** Every `on_lm_start` is paired with `on_lm_end` OR a TTL sweep clears the entry within 5 min. | U03 + U13 + U14 + test fixture `dspy_pending_clean` |
| INV-5 | **No `_SHIM_IN_FLIGHT` stuck-True.** After `on_lm_end` (success, failure, or cancellation), the contextvar returns to `False`. | U10 + U14 + U15 + U18 |
| INV-6 | **D12 coexistence — no double reserve.** With both D21 + D12 installed, a single `dspy.LM(...)` call triggers exactly ONE reserve. | I04 |
| INV-7 | **No sync-in-async deadlock.** `on_lm_start` from inside a running loop raises `SyncInAsyncContext` loudly; never enters `asyncio.run`. | U12 |
| INV-8 | **Direct-path coverage proven.** Custom `dspy.LM` subclass that bypasses LiteLLM still triggers SpendGuard reserve via D21. | Demo step 3 (CUSTOM-LM) + U17 (subclass-no-usage fallback) |
| INV-9 | **No mutation of caller's inputs dict.** Callback does NOT inject keys, does NOT rewrite `messages` / `prompt`, does NOT add headers. | U05 (asserts inputs identity / equality) |
| INV-10 | **Outputs schema tolerance.** Outputs missing `.usage` or shaped as bare strings do NOT crash the callback; estimator fallback kicks in + WARN logged. | U17 |

## 3. Ship checklist

```
[ ] G1 build + import passes
[ ] G2 unit suite passes (≥ 14 tests; goal 16)
[ ] G3 integration suite passes (5 tests)
[ ] G4 existing langchain test baseline unchanged
[ ] G5 existing default-estimator test baseline unchanged
[ ] G6 `make demo-up DEMO_MODE=agent_real_dspy` exits 0 + success lines printed
[ ] G7 `make demo-down` clean
[ ] G8 docs site builds + new page renders with decision matrix
[ ] G9 README adapter table updated
[ ] G10 pyproject.toml extra defined
[ ] G11 no proto / SQL / Rust drift outside the demo verify SQL
[ ] G12 no mutation of existing integration modules
[ ] G13 shared contextvar object identity verified
[ ] G14 spec-pair forward compatibility check passes
[ ] INV-1 .. INV-10 all green
[ ] All 4 slices merged in order S1 → S4 with R1-R5 review loop per build-plan §1.1
[ ] Memory write-back entry `project_coverage_D21_shipped.md` drafted per build-plan §8
```

## 4. Definition of done (per build-plan §7)

- All 4 slices merged into main.
- Acceptance gates G1..G14 + invariants INV-1..INV-10 green.
- README adapter row landed: `| DSPy | Python | pip install 'spendguard-sdk[dspy]' |`.
- `docs/site/docs/integrations/dspy.md` live with 2-path decision matrix.
- `Makefile` carries `DEMO_MODE=agent_real_dspy` entry.
- Memory entry written per build-plan §8.
- Cross-link added from D12 docs page noting "for custom dspy.LM subclasses see D21 callback".

## 5. Out-of-scope explicit declarations

D21 does NOT close any of:

- Token-by-token streaming gating — end-of-call commit only (parity with `langchain.py` / `openai_agents.py` / D12).
- `on_tool_start` / `on_tool_end` callbacks — tool spend rolls into the parent LM reservation. Deferred to D21.1 if observed traffic warrants.
- `on_module_start` / `on_module_end` callbacks — higher-level than necessary; LM-boundary gating subsumes it.
- Async DSPy callbacks — DSPy ≥ 2.6 hooks are sync. Raises `SyncInAsyncContext` when invoked from inside a running loop.
- Per-attempt idempotency for DSPy retry loops — each retry IS a new reservation (parity with LangChain `_agenerate` rerun behavior).

These limitations are documented in `docs/site/docs/integrations/dspy.md` "Limitations" section so operator expectation matches shipping surface.

## 6. Post-ship implications for the build plan

D21 shipping adds the DIRECT path for DSPy gating:

- For DSPy users on LiteLLM-routed providers: D12 transitive path remains preferred (one install covers many frameworks).
- For DSPy users with custom `dspy.LM` subclasses or who don't install D12: D21 is required.
- Both paths can coexist safely (INV-6).

The strategy memo `framework-coverage-2026-06.md` "DSPy adapter" section should be updated post-ship to note "D12 transitive remains primary recommendation; D21 first-class for custom-LM subclass operators."

D21 does NOT replace D12 — they're complementary. The 2-path decision matrix in `dspy.md` is the canonical guide for operators choosing between them.

## 7. Cross-spec touchpoints

- D12 spec already mentions T03 (`test_dspy_predict_triggers_spendguard_reserve`) as transitive coverage smoke. Once D21 ships, that T03 test stays valid AND a new `test_real_dspy_with_d12_shim_no_double_reserve` (I04 here) is added to D21's integration suite — together they prove the coexistence story.
- D12's `litellm_shim.py` will, on its next ship, import `_IN_FLIGHT` from `spendguard._litellm_shim` (the module D21 creates). D21 carries the migration risk: the import contract is one-directional (D12 depends on D21's new module). D12 spec reviewer must catch this when D12 implementation lands.
