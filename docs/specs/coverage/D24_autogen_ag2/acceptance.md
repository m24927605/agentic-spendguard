# D24 — Acceptance Gates

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`review-standards.md`](review-standards.md).

D24 is shipped when **every** gate below runs green from a clean clone (or repo HEAD at slice-merge time) without privileged access.

## 1. Build gates

### 1.1 Module imports cleanly

```bash
cd sdk/python && pip install -e '.[autogen]' autogen-agentchat autogen-ext[openai]
python -c "from spendguard.integrations.autogen import SpendGuardChatCompletionClient, LINEAGE; assert LINEAGE in {'autogen', 'ag2', 'both', 'core-only'}; print(LINEAGE)"
```

Expected output: `autogen`.

```bash
cd sdk/python && pip install -e '.[autogen]' ag2
python -c "from spendguard.integrations.autogen import LINEAGE; print(LINEAGE)"
```

Expected output: `ag2`.

```bash
pip install -e '.[autogen]' autogen-agentchat autogen-ext[openai] ag2
python -c "from spendguard.integrations.autogen import LINEAGE; print(LINEAGE)"
```

Expected output: `both`.

### 1.2 ImportError contract

```bash
python -c "
import sys
sys.modules['autogen_core'] = None
sys.modules['autogen_core.models'] = None
try:
    import spendguard.integrations.autogen  # noqa
except ImportError as exc:
    assert 'spendguard-sdk[autogen]' in str(exc)
    assert 'autogen-agentchat' in str(exc) or 'ag2' in str(exc)
    print('PASS')
"
```

### 1.3 Module LOC budget

```bash
wc -l sdk/python/src/spendguard/integrations/autogen.py
```

Must be ≤ 400 LOC.

## 2. Test gates

### 2.1 Unit tests pass

```bash
cd sdk/python && pip install -e '.[autogen,dev]' autogen-agentchat autogen-ext[openai]
pytest tests/integrations/test_autogen.py -v
```

Must report **≥ 20 passed, 0 failed, 0 errors**.

### 2.2 Integration tests pass per lineage

```bash
# AutoGen lineage
pip install autogen-agentchat>=0.4 autogen-ext[openai]
pytest tests/integrations/test_autogen_real.py -v -k autogen

# AG2 lineage
pip install ag2>=0.7
pytest tests/integrations/test_autogen_real.py -v -k ag2
```

Each invocation must report **≥ 4 passed, 0 failed**. Parametrized fixture IDs must include both `[autogen]` and `[ag2]` suffixes when both packages installed.

### 2.3 Deny-path fail-closed assertion

The deny-path integration tests MUST assert (via `pytest-httpx` request inspection) that **zero HTTP requests** reach the inner OpenAI/Anthropic transport when the SpendGuard sidecar returns DENY. Test marked `@pytest.mark.xfail(strict=True)` if the assertion fires inverted — guarding against a regression where the wrapper falls through.

## 3. Demo gates

### 3.1 `agent_real_autogen` mode runs end-to-end

```bash
cd deploy/demo
DEMO_MODE=agent_real_autogen make demo
```

Expected:
- `docker compose run demo` exits 0.
- `verify_step_autogen.sql` reports 1 row with `(trigger='LLM_CALL_PRE', decision='ALLOW', route='llm.call')` AND 1 row with `(trigger='LLM_CALL_POST', outcome='SUCCESS')` paired by `decision_id`.
- `verify_step_autogen.sql` reports 1 row with `(trigger='LLM_CALL_PRE', decision='DENY')` with NO paired POST.

### 3.2 `agent_real_ag2` mode runs end-to-end

```bash
cd deploy/demo
DEMO_MODE=agent_real_ag2 make demo
```

Same audit-row assertions as 3.1. Same SQL verifier reused.

### 3.3 Default demo mode unaffected

```bash
cd deploy/demo
make demo   # default DEMO_MODE=decision
```

Must remain green — regression guard against D24 leaking dependency into the default path.

## 4. Lint / type gates

```bash
cd sdk/python
ruff check src/spendguard/integrations/autogen.py
mypy src/spendguard/integrations/autogen.py
```

Both must exit 0.

## 5. Docs gates

### 5.1 Public docs page exists

```bash
test -f docs/site/docs/integrations/autogen-ag2.md
```

Must contain:
- The decision table from `implementation.md` §3.
- A working `AssistantAgent(model_client=SpendGuardChatCompletionClient(...))` code block for the AutoGen lineage.
- A working `AssistantAgent(model_client=SpendGuardChatCompletionClient(...))` code block for the AG2 lineage.
- A pointer to D12 (LiteLLM SDK shim) for callers routing via LiteLLM.

### 5.2 README adapter table updated

```bash
grep -E '^\|.*AutoGen.*\|.*AG2|^\|.*\[autogen\]' README.md
```

Must match a row reading approximately: `| AutoGen 0.4+ / AG2 | spendguard.integrations.autogen.SpendGuardChatCompletionClient | spendguard-sdk[autogen] | D24 |`.

### 5.3 Site nav includes the page

```bash
grep -E 'autogen-ag2' docs/site/sidebars.ts docs/site/sidebar*.json 2>/dev/null
```

Must produce at least one match (Starlight or Docusaurus sidebar entry).

## 6. Cross-deliverable gates

### 6.1 No regression on D-shipped OpenAI Agents integration

```bash
cd sdk/python && pytest tests/integrations/test_openai_agents.py -v
```

Must report no new failures vs. `main`. D24 reuses `RunContext` / `run_context` and `current_run_context` — these MUST NOT be modified.

### 6.2 D12 LiteLLM SDK shim remains the documented path for LiteLLM-routed AutoGen/AG2

The `docs/site/docs/integrations/autogen-ag2.md` page MUST link to `docs/site/docs/integrations/litellm-sdk-shim.md` for the LiteLLM-routed transitive coverage case.

## 7. Definition of "100% feasible"

Every gate above is runnable at slice-spec time **without**:
- Network access to OpenAI/Anthropic (provider HTTP is mocked via `pytest-httpx` or a synthetic `FakeChatCompletionClient`).
- A live SpendGuard cloud tenant (uses the local `docker compose` sidecar from existing demo infra).
- A privileged AG2 / AutoGen maintainer action (no upstream PR is on the critical path).

If any gate fails this feasibility test at R1, the implementer MUST flag it before code review, NOT defer to a follow-on issue.
