# D20 — Acceptance

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`review-standards.md`](review-standards.md).

D20 ships when **every gate** in §1 is green AND the §2 invariants are unbreakable AND the §3 ship-checklist is fully checked. Per build-plan §3 "100% feasible" rule: every gate is runnable in the repo's current state, no third-party action required.

## 1. Hard gates

### G1 — Build + import

```bash
cd sdk/python && pip install -e '.[strands]'
python -c "from spendguard.integrations.strands import SpendGuardHookProvider, StrandsRunContext, run_context; print('ok')"
```

Expected: prints `ok`. No `ImportError`.

### G2 — Unit suite (mock Strands runtime)

```bash
cd sdk/python && pytest tests/integrations/test_strands.py -v
```

Expected: 18 tests pass (count from `tests.md` §2; final count may rise during implementation but never fall below 16).

### G3 — Integration suite (real strands + pytest-httpx + 3 backends)

```bash
cd sdk/python && pip install 'aws-strands-agents>=1.0,<2' pytest-httpx boto3 openai 'litellm[proxy]>=1.50,<2'
pytest tests/integrations/test_strands_real.py -v
```

Expected: 9 tests pass. NO outbound HTTP traffic leaves the test process (all calls intercepted by `pytest-httpx`). Each of the 3 backends (Bedrock, OpenAI, LiteLLM) exercised at least twice (1 ALLOW + 1 DENY).

### G4 — Default-estimator suite

```bash
cd sdk/python && pytest tests/integrations/test_default_estimators.py::TestStrands -v
```

Expected: 4 tests pass. Bedrock Anthropic + OpenAI + unknown-fallback + empty-messages paths all green.

### G5 — Existing integration tests still pass (regression)

```bash
cd sdk/python && pytest tests/integrations/ -v --ignore=tests/integrations/test_strands.py --ignore=tests/integrations/test_strands_real.py
```

Expected: all pre-existing tests unchanged. No test count change in `test_langchain*`, `test_litellm*`, `test_openai_agents*`, `test_agt*`, `test_pydantic_ai*`, `test_default_estimators.py` (excluding new TestStrands rows).

### G6 — Demo mode `agent_real_strands` passes

```bash
make demo-down
make demo-up DEMO_MODE=agent_real_strands
```

Expected:

- All compose services reach healthy.
- Demo driver exits 0.
- stdout contains `[demo] agent_real_strands ALL 3 steps PASS`.
- stdout contains `D20_STRANDS OK: ...` from SQL verification.
- stdout contains `D20_STRANDS OK: canonical_events` from the outbox closure verifier.
- All 3 model backends (Bedrock, OpenAI, LiteLLM) recorded at least 1 reserve + 1 commit row each (model_backend variety SQL gate).

### G7 — Demo mode `agent_real_strands_deny` passes

```bash
make demo-down
make demo-up DEMO_MODE=agent_real_strands_deny
```

Expected:

- Exit 0.
- stdout contains `[demo] agent_real_strands_deny ALL 3 substeps PASS`.
- counting-provider stub registered ZERO hits during the 2 DENY sub-steps.
- `denied_decision` row present in `ledger_transactions`.

### G8 — Demo tear-down clean

```bash
make demo-down
```

Expected: no orphaned containers, no orphaned volumes, exit 0.

### G9 — Public docs page renders

```bash
cd docs/site && npm run build
```

Expected: build succeeds. `docs/site/dist/docs/integrations/aws-strands/index.html` exists. Page contains:

- "1-minute setup" code snippet with `Agent(..., hooks=[SpendGuardHookProvider(...)])` call.
- Model-backend coverage matrix with rows: Bedrock (v1 verified) / OpenAI (v1 verified) / Anthropic (v1 covered, untested) / Gemini (v1 covered via LiteLLM, untested native) / Ollama (deferred) / LiteLLM (v1 verified).
- "Limitations" section explicitly listing the 4 non-goals from design.md §3 (per-tool budgets / streaming tokens / TS SDK / Ollama).
- Cross-link to D12 LiteLLM SDK shim docs noting the LiteLLM-routed sub-path is also covered transitively.

### G10 — README index entry present

```bash
grep -F "strands" README.md
```

Expected: exactly one row in the adapter integrations table for `AWS Strands`.

### G11 — PyPI extra wired

```bash
grep -F "strands = " sdk/python/pyproject.toml
```

Expected: `strands = [...]` extra defined with `aws-strands-agents>=1.0,<2`. Existing extras unchanged. No conflicting floor lift on `boto3`/`openai`/`litellm` (Strands transitive deps).

### G12 — No proto / no schema / no Rust changes

```bash
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '\.(proto|sql|rs)$' | grep -v '^deploy/demo/verify_step_strands\.sql$'
```

Expected: empty output. Only the new demo verify SQL is in the .sql allow-list; nothing else proto/SQL/Rust touched.

### G13 — Integration module isolation

```bash
git diff --name-only $(git merge-base main HEAD)..HEAD | grep '^sdk/python/src/spendguard/integrations/' | grep -v -E '(strands|_default_estimator)\.py$'
```

Expected: empty output. D20 must not touch `langchain.py`, `litellm.py`, `openai_agents.py`, `agt.py`, `pydantic_ai.py`.

### G14 — D12 LiteLLM SDK shim regression

```bash
cd sdk/python && pytest tests/integrations/test_litellm_shim.py tests/integrations/test_litellm_shim_real.py tests/integrations/test_crewai_via_shim.py -v
```

Expected: D12 baseline unchanged. The Strands `LiteLLMModel` backend test (I03 + I06) verifies that the hook layer wins over the D12 shim (single reserve, not double) — but D12's own tests remain green.

## 2. Invariants (must never regress)

| ID | Invariant | How verified |
|----|-----------|--------------|
| INV-1 | **DENY never hits the upstream provider, regardless of model backend.** All three backends (Bedrock InvokeModel, OpenAI Chat Completions, LiteLLM-wrapped Gemini) MUST register zero `pytest-httpx` calls on DENY. | I04 + I05 + I06 + demo deny driver + SQL stub-counter delta |
| INV-2 | **Pre-call reservation precedes provider HTTP for every backend.** Sidecar `RequestDecision` RPC fires before any HTTP request to a provider endpoint, on Bedrock, OpenAI, and LiteLLM paths. | U07 + I01-I03 (asyncio.Event ordering) |
| INV-3 | **Fail-closed default.** Sidecar DEGRADE → `SidecarUnavailable` raised → provider not called. Only `SPENDGUARD_STRANDS_FAIL_OPEN=1` permits otherwise. | U10 + U11 |
| INV-4 | **Stash isolation under concurrency.** N concurrent `agent.invoke_async()` calls each get their own stash entry keyed by `invocation_id`; no cross-invocation contamination. | U20 + I07 |
| INV-5 | **Single reserve per outermost invocation.** When the LiteLLM backend internally calls `litellm.acompletion` (D12 shim path), the hook layer's reserve wins and the shim's contextvar `_IN_FLIGHT` short-circuits. Exactly 1 reserve, not 2. | I06 (counts `fake_sidecar.reserve_call_count == 1`) |
| INV-6 | **Provider exception does not mask audit release.** When the provider HTTP raises, `after_invocation` still emits `outcome=FAILURE` and the original exception propagates. | U16 + I08 |
| INV-7 | **Backend coverage matrix asserted.** Demo `agent_real_strands` exercises ALL 3 backends (Bedrock + OpenAI + LiteLLM); SQL `model_backend` variety check enforces this at CI time. | SQL gate in `verify_step_strands.sql` |
| INV-8 | **No mutation of Strands event objects.** The hook reads `event.invocation` + `event.result` + `event.exception` but never writes to them. Strands' event bus contract is read-only. | U07 + U15 (event identity assertions) |
| INV-9 | **`invocation_id` contract pinned.** If Strands GA changes the contract and `Invocation` no longer carries `invocation_id`, the hook fails closed with a version-pin error rather than silently bypassing. | U08 |

## 3. Ship checklist

```
[ ] G1 build + import passes
[ ] G2 unit suite passes (≥ 16 tests; goal 18)
[ ] G3 integration suite passes (9 tests across 3 backends)
[ ] G4 default-estimator suite passes (4 tests)
[ ] G5 existing integration tests baseline unchanged
[ ] G6 `make demo-up DEMO_MODE=agent_real_strands` exits 0 + 3 backends covered
[ ] G7 `make demo-up DEMO_MODE=agent_real_strands_deny` exits 0 + INV-1 stub-counter delta zero
[ ] G8 `make demo-down` clean
[ ] G9 docs site builds + new page renders with backend matrix
[ ] G10 README adapter table updated
[ ] G11 pyproject.toml extra defined
[ ] G12 no proto / SQL / Rust drift outside the demo verify SQL
[ ] G13 only strands.py + _default_estimator.py touched in integrations/
[ ] G14 D12 LiteLLM SDK shim baseline unchanged
[ ] INV-1 .. INV-9 all green
[ ] All 5 slices merged in order S1 → S5 with R1-R5 review loop per build-plan §1.1
[ ] Memory write-back entry `project_coverage_D20_shipped.md` drafted per build-plan §8
```

## 4. Definition of done (per build-plan §7)

- All 5 slices merged into main.
- Acceptance gates G1..G14 + invariants INV-1..INV-9 green.
- README adapter row landed: `| AWS Strands | Python | pip install 'spendguard-sdk[strands]' |`.
- `docs/site/docs/integrations/aws-strands.md` live with model-backend coverage matrix.
- `Makefile` carries `DEMO_MODE=agent_real_strands` + `DEMO_MODE=agent_real_strands_deny` entries.
- Memory entry written per build-plan §8.
- Cross-link added from D12 docs page noting Strands transitively-covered LiteLLM-backend sub-path.
- Cross-link added from D19 (Google ADK) docs page once D19 ships, noting Strands as a sibling Python adapter.

## 5. Out-of-scope explicit declarations

D20 does NOT close any of:

- Per-tool budgets via `before_tool` / `after_tool` hooks — deferred to D20.1.
- Streaming token-by-token gating via `on_message` — end-of-invocation commit only.
- Strands TS SDK — handled separately by D05 + D08 family.
- Native Ollama backend coverage — Ollama runs locally without a fixturable HTTP boundary; deferred.
- Native Anthropic SDK backend coverage in CI — covered transitively via the hook layer but no fixture committed in v1.
- Strands' built-in observability tap (OpenTelemetry tracing) — SpendGuard's own audit is the system of record; cross-correlation deferred.
- Auto-installation via `default_hooks=` — operator MUST construct `SpendGuardHookProvider` explicitly so the wiring is observable.

These limitations are documented in `docs/site/docs/integrations/aws-strands.md` "Limitations" section so operator expectation matches shipping surface.

## 6. Post-ship implications for the build plan

D20 shipping changes the framework coverage plan:

- **Strands native Bedrock-only adopters** — covered directly (load-bearing path for AWS-shop adoption).
- **Strands + LiteLLM operators** — covered both by D20 and by D12 shim. Hook layer wins; D12 shim's contextvar guard prevents double-reserve. Documented as supported.
- **D21 DSPy / D22 Agno / D23 BeeAI / D25 SmolAgents / D28 Atomic Agents** — D20's hook-provider pattern is a reference template for "framework with event bus" adapters. Each downstream spec should compare against D20's design.md before drafting its own.
- The strategy memo `framework-coverage-2026-06.md` "AWS Strands Agents SDK" section should be updated post-ship to mark coverage as "native (Bedrock-load-bearing) + LiteLLM (via D12 shim) verified".
