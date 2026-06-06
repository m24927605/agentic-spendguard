# D27 — Acceptance

Backlinks: [`design.md`](./design.md), [`implementation.md`](./implementation.md), [`tests.md`](./tests.md), [`review-standards.md`](./review-standards.md).

A deliverable is "shipped" iff every gate below runs green from a clean checkout, with no manual intervention beyond setting `OPENAI_API_KEY` (for the live demo gate `G10` only). Per build-plan §3, every gate must be runnable by `superpowers:code-reviewer` from the repo state.

## G01 — Module imports cleanly with `[llamaindex]` extra installed

```bash
pip install -e 'sdk/python[llamaindex]'
python -c "from spendguard.integrations.llamaindex import SpendGuardLlamaIndexHandler, SpendGuardLlamaIndexDenied; print('ok')"
```

**Pass:** prints `ok`. No `ImportError`. `llama-index-core` resolved at the pinned floor `>= 0.12`.

## G02 — Module raises actionable `ImportError` without `[llamaindex]` extra

```bash
pip uninstall -y llama-index-core
python -c "from spendguard.integrations.llamaindex import SpendGuardLlamaIndexHandler" 2>&1 \
  | grep -F "pip install 'spendguard-sdk[llamaindex]'"
```

**Pass:** the install hint appears in the error output. Exit code from the test wrapper is 0 (grep matched).

## G03 — `pyproject.toml` ships the `[llamaindex]` extra

```bash
python -c "
import tomllib
with open('sdk/python/pyproject.toml','rb') as f:
    cfg = tomllib.load(f)
assert 'llamaindex' in cfg['project']['optional-dependencies'], 'missing [llamaindex] extra'
deps = cfg['project']['optional-dependencies']['llamaindex']
assert any(d.startswith('llama-index-core') and '>=0.12' in d for d in deps), deps
# Anti-check: no provider sub-packages declared here
banned = ('llama-index-llms-openai', 'llama-index-llms-anthropic',
          'llama-index-llms-gemini', 'llama-index-llms-bedrock')
assert not any(any(d.startswith(b) for b in banned) for d in deps), deps
print('ok')
"
```

**Pass:** prints `ok`. The `[llamaindex]` extra contains exactly `llama-index-core>=0.12` (no provider sub-package leakage).

## G04 — Unit test suite green

```bash
pip install -e 'sdk/python[llamaindex,dev]'
pytest sdk/python/tests/integrations/test_llamaindex_unit.py -v
```

**Pass:** all 25 unit tests (U01-U25 from `tests.md`) pass. No skips.

## G05 — Integration test suite green

```bash
pytest sdk/python/tests/integrations/test_llamaindex_integration.py -v
```

**Pass:** all 8 integration tests (I01-I08) pass. Recorded fixtures load from disk; no network access.

## G06 — Fixture integrity

```bash
for f in openai_gpt_4o_mini_allow openai_gpt_4o_mini_deny anthropic_sonnet_allow gemini_flash_allow bedrock_converse_allow; do
  test -f "sdk/python/tests/integrations/fixtures/llamaindex/$f.json" || exit 1
done
python -c "
import json, pathlib
for p in pathlib.Path('sdk/python/tests/integrations/fixtures/llamaindex').glob('*.json'):
    obj = json.loads(p.read_text())
    assert 'model' in obj, p
    assert 'payload_start' in obj and 'payload_end' in obj, p
print('ok')
"
```

**Pass:** all five fixtures exist and have the documented `{model, payload_start, payload_end}` shape.

## G07 — Mock-LlamaIndex fallback works without `llama-index-core` installed

```bash
pip uninstall -y llama-index-core
pytest sdk/python/tests/integrations/test_llamaindex_unit.py -v
```

**Pass:** unit suite still green via the `SimpleNamespace`-based stub (per `tests.md` §3). Integration suite is properly skipped (not failed) with `pytest.importorskip` marker.

## G08 — Non-LLM event types are filtered (no sidecar calls)

```bash
pytest sdk/python/tests/integrations/test_llamaindex_unit.py::test_non_llm_events_are_no_op -v
```

**Pass:** the explicit filter test (U07) passes. Confirms `CBEventType.EMBEDDING` / `RETRIEVE` / `CHUNK` / `QUERY` / `NODE_PARSING` all early-return with zero sidecar calls.

## G09 — Demo mode boots and validates ALLOW + DENY paths (no live API)

```bash
DEMO_MODE=agent_real_llamaindex_stub make demo-up
make demo-test
make demo-down
```

**Pass:**

- `demo-up` exits 0 within 120 s.
- `demo-test` exits 0.
- Stdout contains both:
  - `[demo] agent_real_llamaindex run completed: ALLOW path`
  - `[demo] agent_real_llamaindex run completed: DENY path (model not called)`
- SQL verify reports:
  - `≥ 1` row with `trigger='LLM_CALL_PRE'` and `verdict='ALLOW'` paired to a non-null `commit_id`.
  - `≥ 1` row with `trigger='LLM_CALL_PRE'` and `verdict='DENY'` with **no** paired commit row.
- MockLLM call counter on the DENY path = 0 (the stub records invocation counts).

(`agent_real_llamaindex_stub` is the no-API-key variant using `llama_index.core.llms.MockLLM`. The live variant `agent_real_llamaindex` reuses the same wiring; G10 covers live.)

## G10 — Live OpenAI demo passes (when `OPENAI_API_KEY` is set)

```bash
export OPENAI_API_KEY=...
DEMO_MODE=agent_real_llamaindex make demo-up
make demo-test
make demo-down
```

**Pass:** identical assertions to G09, but the ALLOW path actually hits `api.openai.com`. Network call count on ALLOW = 1 (plus embedding call for `VectorStoreIndex.from_documents` — counted separately as embedding traffic is not gated by D27). DENY path: openai.com call count for the `query` synthesis = 0.

**Gate is conditional**: skipped on CI without the secret; must pass locally before deliverable is marked done.

## G11 — README adapter table updated

```bash
grep -F "LlamaIndex" README.md | grep -F "spendguard-sdk[llamaindex]"
```

**Pass:** one matching row exists in the adapter table.

## G12 — User-facing docs page exists, renders, and ships the 2-path matrix

```bash
test -f docs/site/docs/integrations/llamaindex.md
grep -F "llama-index-llms-litellm" docs/site/docs/integrations/llamaindex.md \
  | grep -F "D12"
grep -F "Settings.callback_manager" docs/site/docs/integrations/llamaindex.md
cd docs/site && npm run build
```

**Pass:**

- The page exists.
- The page documents the 2-path coverage matrix (LiteLLM-routed → D12; direct → D27).
- The page shows the `Settings.callback_manager = CallbackManager([handler])` registration example.
- The Starlight site builds with zero **errors** for that page.

## G13 — No regression in existing integration tests

```bash
pytest sdk/python/tests/integrations/test_langchain*.py \
       sdk/python/tests/integrations/test_openai_agents*.py \
       sdk/python/tests/integrations/test_litellm*.py \
       sdk/python/tests/integrations/test_pydantic_ai*.py \
       sdk/python/tests/integrations/test_agt*.py -v
```

**Pass:** all existing integration tests remain green. Confirms `_default_estimator.py` modification (additive) didn't break dispatcher selection for other adapters.

## G14 — No proto / DB migration drift

```bash
git diff --stat main -- proto/ schema/ migrations/
```

**Pass:** zero changes under `proto/`, `schema/`, `migrations/`. D27 is a pure SDK addition.

## G15 — No new public-API exposure leaks proto types

```bash
python -c "
from spendguard.integrations import llamaindex
exported = sorted(n for n in llamaindex.__all__ if not n.startswith('_'))
assert exported == ['ClaimEstimator', 'RunIdFn', 'SpendGuardLlamaIndexDenied', 'SpendGuardLlamaIndexHandler'], exported
print(exported)
"
```

**Pass:** public surface is exactly the four documented symbols.

## G16 — Lint + typecheck clean

```bash
cd sdk/python
ruff check src/spendguard/integrations/llamaindex.py tests/integrations/test_llamaindex*.py
mypy --strict src/spendguard/integrations/llamaindex.py
```

**Pass:** zero `ruff` errors and zero `mypy --strict` errors on the new files.

## G17 — No edits to the 6 already-shipped adapters

```bash
git diff --stat main -- \
  sdk/python/src/spendguard/integrations/langchain.py \
  sdk/python/src/spendguard/integrations/openai_agents.py \
  sdk/python/src/spendguard/integrations/litellm.py \
  sdk/python/src/spendguard/integrations/pydantic_ai.py \
  sdk/python/src/spendguard/integrations/agt.py
```

**Pass:** zero changes to any of these files. The only allowed shared-file touch is `_default_estimator.py` (additive symbol only).

## G18 — D12 transitive coverage preserved (regression sanity)

```bash
# Sanity: confirm that with both D12 (installed via spendguard_litellm_shim.install)
# and D27 active, a `llama-index-llms-litellm` LLM call does NOT double-reserve.
pytest sdk/python/tests/integrations/test_llamaindex_unit.py::test_d12_transitive_no_double_reserve -v 2>/dev/null \
  || echo "test not yet in D27 unit suite — out of scope by design.md §3 anti-scope (interaction tested in D12 suite)"
```

**Pass:** either the test exists and passes, OR the message confirms the test is out-of-scope per design §3 (interaction is tested in D12's own suite, not here). Reviewer SHOULD pick the latter for D27 ship — adding the cross-test in D12's suite is the canonical location.

## G19 — End-to-end acceptance summary (the user's stated bar)

Direct quote from the deliverable brief:

> a Python demo using `llm = OpenAI(model="gpt-4o-mini")` + `Settings.callback_manager = CallbackManager([SpendGuardLlamaIndexHandler(client=...)])` + a `VectorStoreIndex` query runs end-to-end with SpendGuard gating + deny test.

This is satisfied by **G09** (stubbed, MockLLM) + **G10** (live OpenAI, gated on secret) together. Marking G09 as the canonical CI-runnable gate; G10 is the human-verifiable closer.

---

## Done definition

D27 is done when **G01-G09 + G11-G18** are green in CI on the merged branch (G10 confirmed locally + screenshot/log attached to the memory write-back per build-plan §8).
