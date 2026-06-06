# D25 — Acceptance Gates

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`review-standards.md`](review-standards.md).

D25 is shipped when **every** gate below runs green from a clean clone (or repo HEAD at slice-merge time) without privileged access.

## 1. Build gates

### 1.1 Module imports cleanly

```bash
cd sdk/python && pip install -e '.[smolagents]'
python -c "
from spendguard.integrations.smolagents import (
    SpendGuardSmolModel, spendguard_step_callback, ClaimEstimator,
)
print('ok')
"
```

Expected output: `ok`.

### 1.2 ImportError contract

```bash
python -c "
import sys
sys.modules['smolagents'] = None
sys.modules['smolagents.models'] = None
try:
    import spendguard.integrations.smolagents  # noqa
except ImportError as exc:
    assert 'spendguard-sdk[smolagents]' in str(exc)
    print('PASS')
"
```

### 1.3 Module LOC budget

```bash
wc -l sdk/python/src/spendguard/integrations/smolagents.py
```

Must be ≤ 400 LOC.

### 1.4 RunContext reuse asserted at static level

```bash
grep -nE 'from .openai_agents import current_run_context' \
  sdk/python/src/spendguard/integrations/smolagents.py
# Must produce exactly one match.

grep -nE 'class RunContext|asynccontextmanager.*run_context' \
  sdk/python/src/spendguard/integrations/smolagents.py
# Must produce ZERO matches — duplication is a Blocker per review-standards §1.3.
```

## 2. Test gates

### 2.1 Unit tests pass

```bash
cd sdk/python && pip install -e '.[smolagents,dev]'
pytest tests/integrations/test_smolagents.py -v
```

Must report **≥ 20 passed, 0 failed, 0 errors**.

### 2.2 Integration tests pass per inner

```bash
pip install smolagents>=1.5
pytest tests/integrations/test_smolagents_real.py -v -k inference_client
pytest tests/integrations/test_smolagents_real.py -v -k openai_server
```

Each invocation must report **≥ 3 passed, 0 failed**. Parametrized fixture IDs MUST include both `[inference_client]` and `[openai_server]` suffixes.

### 2.3 Deny-path fail-closed assertion

The deny-path integration tests MUST assert (via `pytest-httpx` request inspection) that **zero HTTP requests** reach the inner OpenAI-compatible or HF Inference transport when the SpendGuard sidecar returns DENY. Test marked `@pytest.mark.xfail(strict=True)` if the assertion fires inverted — guards against the wrapper falling through.

### 2.4 `__call__` alias coverage

```bash
pytest tests/integrations/test_smolagents.py::test_call_alias_routes_through_generate -v
pytest tests/integrations/test_smolagents.py::test_call_alias_propagates_kwargs -v
```

Both must pass. Missing alias coverage is a Blocker — `smolagents<1.5` agents would silently bypass the gate.

## 3. Demo gates

### 3.1 `agent_real_smolagents` mode runs end-to-end

```bash
cd deploy/demo
DEMO_MODE=agent_real_smolagents make demo
```

Expected:
- `docker compose run demo` exits 0.
- `verify_step_smolagents.sql` reports 1 row with `(trigger='LLM_CALL_PRE', decision='ALLOW', route='llm.call')` AND 1 row with `(trigger='LLM_CALL_POST', outcome='SUCCESS')` paired by `decision_id`.
- `verify_step_smolagents.sql` reports 1 row with `(trigger='LLM_CALL_PRE', decision='DENY')` with NO paired POST.
- `estimated_amount_atomic` on the SUCCESS POST is non-zero (extracted from `ChatMessage.token_usage`).

### 3.2 Default demo mode unaffected

```bash
cd deploy/demo
make demo   # default DEMO_MODE=decision
```

Must remain green — regression guard against D25 leaking the `smolagents` dependency into the default path.

## 4. Lint / type gates

```bash
cd sdk/python
ruff check src/spendguard/integrations/smolagents.py
mypy src/spendguard/integrations/smolagents.py
```

Both must exit 0.

## 5. Docs gates

### 5.1 Public docs page exists

```bash
test -f docs/site/docs/integrations/smolagents.md
```

Must contain:
- The decision table from `implementation.md` §3 (5 rows: `InferenceClientModel`, `OpenAIServerModel`, `TransformersModel`, `LiteLLMModel`, `step_callbacks`).
- A working `CodeAgent(model=SpendGuardSmolModel(inner=OpenAIServerModel(...)), tools=[...])` code block.
- A `spendguard_step_callback(client, run_id=...)` example wired into `MultiStepAgent.step_callbacks`, with a banner stating "informational only — does NOT gate".
- A pointer to D12 (LiteLLM SDK shim) for `LiteLLMModel` callers.
- A polyglot-run example sharing `run_context()` between a SmolAgents `CodeAgent` and an `openai_agents.Agent`.

### 5.2 README adapter table updated

```bash
grep -E '^\|.*SmolAgents.*\|.*\[smolagents\]' README.md
```

Must match a row reading approximately: `| SmolAgents | spendguard.integrations.smolagents.SpendGuardSmolModel | spendguard-sdk[smolagents] | D25 |`.

### 5.3 Site nav includes the page

```bash
grep -E 'smolagents' docs/site/sidebars.ts docs/site/sidebar*.json 2>/dev/null
```

Must produce at least one match.

## 6. Cross-deliverable gates

### 6.1 No regression on shipped adapters

```bash
cd sdk/python && pytest tests/integrations/test_openai_agents.py -v
```

Must report no new failures vs. `main`. D25 reuses `RunContext` / `run_context` / `current_run_context` — these MUST NOT be modified.

### 6.2 D12 LiteLLM SDK shim remains the documented path for `LiteLLMModel`

The `docs/site/docs/integrations/smolagents.md` page MUST link to `docs/site/docs/integrations/litellm-sdk-shim.md` for `LiteLLMModel` callers.

### 6.3 `step_callbacks` documented as non-gating

The helper docstring AND the docs page MUST state explicitly: `step_callbacks` fire AFTER each step completes and cannot deny a pending LLM call. The wrapper is the gating surface. Omission of this warning is a finding (parity with the AutoGen / OpenAI Agents pattern).

## 7. Definition of "100% feasible"

Every gate above is runnable at slice-spec time **without**:

- Network access to OpenAI / HuggingFace Inference (provider HTTP is mocked via `pytest-httpx` or wiremock).
- A live SpendGuard cloud tenant (uses local `docker compose` sidecar from existing demo infra).
- A privileged HuggingFace maintainer action (no upstream PR is on the critical path).
- A GPU runtime (`TransformersModel` token-cost wrap is exercised via a `FakeSmolModel` returning `ChatMessage(token_usage=TokenUsage(...))`; no actual transformers inference required).

If any gate fails this feasibility test at R1, the implementer MUST flag it before code review, NOT defer to a follow-on issue.
