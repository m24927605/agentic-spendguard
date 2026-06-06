# D28 — Acceptance Gates

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`review-standards.md`](review-standards.md).

D28 is shipped when **every** gate below runs green from a clean clone (or repo HEAD at slice-merge time) without privileged access.

## 1. Build gates

### 1.1 Module imports cleanly

```bash
cd sdk/python && pip install -e '.[atomic-agents]'
python -c "from spendguard.integrations.atomic_agents import SpendGuardInstructorProxy, SpendGuardAsyncInstructorProxy, wrap_instructor_client; print('OK')"
```

Expected output: `OK`.

### 1.2 ImportError contract

```bash
python -c "
import sys
sys.modules['instructor'] = None
try:
    import spendguard.integrations.atomic_agents  # noqa
except ImportError as exc:
    assert 'spendguard-sdk[atomic-agents]' in str(exc)
    print('PASS')
"
```

Plus a sibling check that strips `atomic_agents` and asserts the import error message points at `atomic-agents>=1.0,<2.0`.

### 1.3 Module LOC budget

```bash
wc -l sdk/python/src/spendguard/integrations/atomic_agents.py
```

Must be ≤ 400 LOC.

### 1.4 Extras pin

```bash
grep -A 4 '^atomic-agents = \[' sdk/python/pyproject.toml
```

Must show both `"atomic-agents>=1.0,<2.0"` AND `"instructor>=1.5,<2.0"`. Open-ended pins are a build-gate failure. Pinning anything other than those two packages in the extra is a finding.

## 2. Test gates

### 2.1 Unit tests pass

```bash
cd sdk/python && pip install -e '.[atomic-agents,dev]'
pytest tests/integrations/test_atomic_agents.py -v
```

Must report **≥ 22 passed, 0 failed, 0 errors**.

### 2.2 Integration tests pass

```bash
cd sdk/python && pip install -e '.[atomic-agents,dev]'
pytest tests/integrations/test_atomic_agents_real.py -v
```

Must report **≥ 6 passed, 0 failed**. Without `instructor` or `atomic_agents` installed, must report `SKIPPED` (not failure) due to `pytest.importorskip` guards.

### 2.3 Deny-path fail-closed assertion

The deny-path integration test `test_real_atomic_agents_deny_path_zero_provider_http` MUST assert (via `pytest-httpx` request inspection) that **zero HTTP requests** reach the inner OpenAI transport when the SpendGuard sidecar returns DENY. Test marked `@pytest.mark.xfail(strict=True)` if the assertion fires inverted.

### 2.4 Instructor validation-retry reservation-per-attempt

```bash
cd sdk/python
pytest tests/integrations/test_atomic_agents_real.py::test_real_instructor_validation_retry_creates_per_retry_reservation -v
```

Must pass — the load-bearing test that justifies wrapping the Instructor object instead of the raw provider SDK. Failure means the adapter undercounts cost on validation retries.

### 2.5 Raw-OpenAI-wrap rejection

```bash
cd sdk/python
pytest tests/integrations/test_atomic_agents.py::test_wrap_instructor_client_rejects_raw_openai_client \
       tests/integrations/test_atomic_agents_real.py::test_raw_openai_wrap_rejected_by_factory -v
```

Both must pass.

## 3. Demo gates

### 3.1 `agent_real_atomic_agents` mode runs end-to-end

```bash
cd deploy/demo
DEMO_MODE=agent_real_atomic_agents make demo
```

Expected:
- `docker compose run demo` exits 0.
- `verify_step_atomic_agents.sql` reports 1 row with `(trigger='LLM_CALL_PRE', decision='ALLOW', route='llm.call')` AND 1 row with `(trigger='LLM_CALL_POST', outcome='SUCCESS')` paired by `decision_id`.
- `verify_step_atomic_agents.sql` reports 1 row with `(trigger='LLM_CALL_PRE', decision='DENY')` with NO paired POST.

### 3.2 Default demo mode unaffected

```bash
cd deploy/demo
make demo   # default DEMO_MODE=decision
```

Must remain green — regression guard against D28 leaking dependency into the default path.

## 4. Lint / type gates

```bash
cd sdk/python
ruff check src/spendguard/integrations/atomic_agents.py
mypy src/spendguard/integrations/atomic_agents.py
```

Both must exit 0.

## 5. Docs gates

### 5.1 Public docs page exists

```bash
test -f docs/site/docs/integrations/atomic-agents.md
```

Must contain:
- The Instructor-wrap-vs-raw-SDK rationale table from `implementation.md` §3 item 1, with the rejected raw-SDK row explicitly explained (Instructor's validation retries bypass raw-transport wraps).
- A working `wrap_instructor_client(instructor.from_openai(OpenAI()), ...)` code block.
- A `BaseAgent` + Pydantic `output_schema` example.
- A polyglot trace example sharing `RunContext` with `spendguard.integrations.openai_agents`.
- A pointer to `spendguard.integrations.openai_agents._default_estimator` for the OpenAI-routed default-tokenizer estimator.

### 5.2 README adapter table updated

```bash
grep -E '^\|.*Atomic Agents.*\|.*\[atomic-agents\]' README.md
```

Must match a row reading approximately: `| Atomic Agents 1.0+ (Instructor wrap) | spendguard.integrations.atomic_agents.wrap_instructor_client | spendguard-sdk[atomic-agents] | D28 |`.

### 5.3 Site nav includes the page

```bash
grep -E 'atomic-agents' docs/site/sidebars.ts docs/site/sidebar*.json 2>/dev/null
```

Must produce at least one match.

## 6. Cross-deliverable gates

### 6.1 No regression on shipped OpenAI Agents integration

```bash
cd sdk/python && pytest tests/integrations/test_openai_agents.py -v
```

Must report no new failures vs. `main`. D28 reuses `current_run_context` — `openai_agents.py` MUST NOT be modified.

### 6.2 No regression on shipped sibling adapters

```bash
cd sdk/python && pytest tests/integrations/test_letta.py tests/integrations/test_pydantic_ai.py tests/integrations/test_langchain.py -v
```

Must report no new failures. D28 introduces no shared-module change beyond the new file.

### 6.3 Async + sync dispatch matches Atomic Agents documented surface

The factory `wrap_instructor_client(...)` MUST accept any `instructor.Instructor` or `instructor.AsyncInstructor`. Atomic Agents' `BaseAgent(BaseAgentConfig(client=...))` accepts either; the adapter must not force operators to pick async-only or sync-only.

## 7. Definition of "100% feasible"

Every gate above is runnable at slice-spec time **without**:
- Network access to OpenAI / Anthropic (provider HTTP is mocked via `pytest-httpx` or a synthetic `httpx.MockTransport`).
- A live SpendGuard cloud tenant (uses the local `docker compose` sidecar from existing demo infra).
- A privileged Atomic Agents or Instructor maintainer action — no upstream PR is on the critical path.
- A real Pydantic-validation-failing model response — the validation-retry test synthesizes the failing payload via `pytest-httpx`.

If any gate fails this feasibility test at R1, the implementer MUST flag it before code review, NOT defer to a follow-on issue.
