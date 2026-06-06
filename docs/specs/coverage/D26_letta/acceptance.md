# D26 — Acceptance Gates

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`review-standards.md`](review-standards.md).

D26 is shipped when **every** gate below runs green from a clean clone (or repo HEAD at slice-merge time) without privileged access.

## 1. Build gates

### 1.1 Module imports cleanly

```bash
cd sdk/python && pip install -e '.[letta]'
python -c "from spendguard.integrations.letta import SpendGuardLettaClient, wrap_llm_client; print('OK')"
```

Expected output: `OK`.

### 1.2 ImportError contract

```bash
python -c "
import sys
sys.modules['letta'] = None
sys.modules['letta.llm_api'] = None
sys.modules['letta.llm_api.llm_client_base'] = None
try:
    import spendguard.integrations.letta  # noqa
except ImportError as exc:
    assert 'spendguard-sdk[letta]' in str(exc)
    assert 'letta>=0.8' in str(exc)
    print('PASS')
"
```

### 1.3 Module LOC budget

```bash
wc -l sdk/python/src/spendguard/integrations/letta.py
```

Must be ≤ 400 LOC.

### 1.4 Extras pin

```bash
grep -A 3 '^letta = \[' sdk/python/pyproject.toml
```

Must show `"letta>=0.8,<1.0"`. Open-ended pins (`letta` with no bound) or pinning anything other than `letta` in the extra is a build-gate failure.

## 2. Test gates

### 2.1 Unit tests pass

```bash
cd sdk/python && pip install -e '.[letta,dev]'
pytest tests/integrations/test_letta.py -v
```

Must report **≥ 18 passed, 0 failed, 0 errors**.

### 2.2 Integration tests pass

```bash
cd sdk/python && pip install -e '.[letta,dev]'
pytest tests/integrations/test_letta_real.py -v
```

Must report **≥ 5 passed, 0 failed**. Without Letta installed, must report `SKIPPED` (not failure) due to `pytest.importorskip("letta", minversion="0.8")`.

### 2.3 Deny-path fail-closed assertion

The deny-path integration test `test_real_letta_deny_path_zero_provider_http` MUST assert (via `pytest-httpx` request inspection) that **zero HTTP requests** reach the inner OpenAI/Anthropic transport when the SpendGuard sidecar returns DENY. Test is marked `@pytest.mark.xfail(strict=True)` if the assertion fires inverted — guarding against a regression where the wrapper falls through.

### 2.4 Sync-in-loop guard

```bash
cd sdk/python
pytest tests/integrations/test_letta.py::test_send_llm_request_sync_inside_running_loop_raises -v
```

Must pass — silent `asyncio.run()` inside an active loop is a release-blocking defect.

## 3. Demo gates

### 3.1 `agent_real_letta` mode runs end-to-end

```bash
cd deploy/demo
DEMO_MODE=agent_real_letta make demo
```

Expected:
- `docker compose run demo` exits 0.
- `verify_step_letta.sql` reports 1 row with `(trigger='LLM_CALL_PRE', decision='ALLOW', route='llm.call')` AND 1 row with `(trigger='LLM_CALL_POST', outcome='SUCCESS')` paired by `decision_id`.
- `verify_step_letta.sql` reports 1 row with `(trigger='LLM_CALL_PRE', decision='DENY')` with NO paired POST.

### 3.2 Default demo mode unaffected

```bash
cd deploy/demo
make demo   # default DEMO_MODE=decision
```

Must remain green — regression guard against D26 leaking dependency into the default path.

## 4. Lint / type gates

```bash
cd sdk/python
ruff check src/spendguard/integrations/letta.py
mypy src/spendguard/integrations/letta.py
```

Both must exit 0.

## 5. Docs gates

### 5.1 Public docs page exists

```bash
test -f docs/site/docs/integrations/letta.md
```

Must contain:
- The library-vs-server decision table from `implementation.md` §3, with the `letta server` row listed **first** and pointing at D02/D03.
- A working `wrap_llm_client(inner=OpenAIClient(...), ...)` code block for the embedded-library mode.
- A polyglot trace example sharing `RunContext` with `spendguard.integrations.openai_agents`.
- A pointer to D12 (LiteLLM SDK shim) for the LiteLLM-routed transitive coverage case.

### 5.2 README adapter table updated

```bash
grep -E '^\|.*Letta.*\|.*\[letta\]' README.md
```

Must match a row reading approximately: `| Letta 0.8+ (library mode) | spendguard.integrations.letta.SpendGuardLettaClient | spendguard-sdk[letta] | D26 |`.

### 5.3 Site nav includes the page

```bash
grep -E 'letta' docs/site/sidebars.ts docs/site/sidebar*.json 2>/dev/null
```

Must produce at least one match (Starlight or Docusaurus sidebar entry pointing at `integrations/letta`).

## 6. Cross-deliverable gates

### 6.1 No regression on D-shipped OpenAI Agents integration

```bash
cd sdk/python && pytest tests/integrations/test_openai_agents.py -v
```

Must report no new failures vs. `main`. D26 reuses `RunContext` / `run_context` / `current_run_context` — these MUST NOT be modified.

### 6.2 D02/D03 documented as canonical server-mode path

The `docs/site/docs/integrations/letta.md` page MUST link to D02 (closed CLI install) and D03 (base-URL drop-in landing page) for the `letta server` deployment shape. Omitting either link is a docs-gate failure.

### 6.3 D12 documented as transitive LiteLLM path

The page MUST link to `docs/site/docs/integrations/litellm-sdk-shim.md` for the LiteLLM-routed transitive coverage case.

## 7. Definition of "100% feasible"

Every gate above is runnable at slice-spec time **without**:
- Network access to OpenAI/Anthropic (provider HTTP is mocked via `pytest-httpx` or a synthetic `FakeLLMClient`).
- A live SpendGuard cloud tenant (uses the local `docker compose` sidecar from existing demo infra).
- A privileged Letta maintainer action — no upstream PR is on the critical path.
- A real Letta archival-memory backend (`letta` self-test uses sqlite by default; demo wires that path).

If any gate fails this feasibility test at R1, the implementer MUST flag it before code review, NOT defer to a follow-on issue.
