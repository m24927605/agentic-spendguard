# D19 — Acceptance

Backlinks: [`design.md`](./design.md), [`implementation.md`](./implementation.md), [`tests.md`](./tests.md), [`review-standards.md`](./review-standards.md).

A deliverable is "shipped" iff every gate below runs green from a clean checkout, with no manual intervention beyond setting `GOOGLE_API_KEY` (for live demo gate `G09` only). Per build-plan §3, every gate must be runnable by `superpowers:code-reviewer` from the repo state.

## G01 — Module imports cleanly with `[adk]` extra installed

```bash
pip install -e 'sdk/python[adk]'
python -c "from spendguard.integrations.adk import SpendGuardAdkCallback; print('ok')"
```

**Pass:** prints `ok`. No `ImportError`. `google-adk` resolved at the pinned floor `>= 1.0`.

## G02 — Module raises actionable `ImportError` without `[adk]` extra

```bash
pip uninstall -y google-adk
python -c "from spendguard.integrations.adk import SpendGuardAdkCallback" 2>&1 \
  | grep -F "pip install 'spendguard-sdk[adk]'"
```

**Pass:** the install hint appears in the error output. Exit code from the test wrapper is 0 (grep matched).

## G03 — `pyproject.toml` ships the `[adk]` extra

```bash
python -c "
import tomllib
with open('sdk/python/pyproject.toml','rb') as f:
    cfg = tomllib.load(f)
assert 'adk' in cfg['project']['optional-dependencies'], 'missing [adk] extra'
deps = cfg['project']['optional-dependencies']['adk']
assert any(d.startswith('google-adk') and '>=1.0' in d for d in deps), deps
print('ok')
"
```

**Pass:** prints `ok`.

## G04 — Unit test suite green

```bash
pip install -e 'sdk/python[adk,dev]'
pytest sdk/python/tests/integrations/test_adk_unit.py -v
```

**Pass:** all 20 unit tests (U01-U20 from `tests.md`) pass. No skips.

## G05 — Integration test suite green

```bash
pytest sdk/python/tests/integrations/test_adk_integration.py -v
```

**Pass:** all 5 integration tests (I01-I05) pass. Recorded fixtures load from disk; no network access.

## G06 — Fixture integrity

```bash
test -f sdk/python/tests/integrations/fixtures/adk/gemini_2_0_flash_allow.json
test -f sdk/python/tests/integrations/fixtures/adk/gemini_2_0_flash_deny.json
test -f sdk/python/tests/integrations/fixtures/adk/litellm_gpt_4o_mini_allow.json
python -c "
import json, pathlib
for p in pathlib.Path('sdk/python/tests/integrations/fixtures/adk').glob('*.json'):
    obj = json.loads(p.read_text())
    assert 'request' in obj and 'response' in obj, p
print('ok')
"
```

**Pass:** all three fixtures exist and have the documented `{request, response}` shape.

## G07 — Mock-ADK fallback works without `google-adk` installed

```bash
pip uninstall -y google-adk
pytest sdk/python/tests/integrations/test_adk_unit.py -v
```

**Pass:** unit suite still green via the `SimpleNamespace`-based stub (per `tests.md` §3). Integration suite is properly skipped (not failed) with `pytest.importorskip` marker.

## G08 — Demo mode boots and validates ALLOW + DENY paths (no live API)

```bash
# Headless demo using fake-Gemini stub (no GOOGLE_API_KEY required):
DEMO_MODE=agent_real_adk_stub make demo-up
make demo-test
make demo-down
```

**Pass:**

- `demo-up` exits 0 within 120 s.
- `demo-test` exits 0.
- Stdout contains both:
  - `[demo] agent_real_adk run completed: ALLOW path`
  - `[demo] agent_real_adk run completed: DENY path (model not called)`
- SQL verify reports:
  - `≥ 1` row with `trigger='LLM_CALL_PRE'` and `verdict='ALLOW'` paired to a non-null `commit_id`.
  - `≥ 1` row with `trigger='LLM_CALL_PRE'` and `verdict='DENY'` with **no** paired commit row.
- Mock-egress stub recorded 0 outbound HTTP calls on the DENY path.

(`agent_real_adk_stub` is the no-API-key variant that points `LlmAgent` at a local fake-Gemini server. The live variant `agent_real_adk` reuses the same wiring; G09 covers live.)

## G09 — Live Gemini demo passes (when `GOOGLE_API_KEY` is set)

```bash
export GOOGLE_API_KEY=...
DEMO_MODE=agent_real_adk make demo-up
make demo-test
make demo-down
```

**Pass:** identical assertions to G08, but the ALLOW path actually hits `generativelanguage.googleapis.com`. Network call count on ALLOW = 1; on DENY = 0.

**Gate is conditional**: skipped on CI without the secret, must pass locally before deliverable is marked done.

## G10 — README adapter table updated

```bash
grep -F "Google ADK" README.md | grep -F "spendguard-sdk[adk]"
```

**Pass:** one matching row exists in the adapter table.

## G11 — User-facing docs page exists and renders

```bash
test -f docs/site/docs/integrations/adk.md
cd docs/site && npm run build 2>&1 | grep -v "warning"
test ! -f /tmp/adk-docs-build-error
```

**Pass:** the page exists; the Starlight site builds with zero **errors** for that page. Warnings (e.g. unused link refs) are non-fatal but should be triaged separately.

## G12 — No regression in existing integration tests

```bash
pytest sdk/python/tests/integrations/test_langchain*.py \
       sdk/python/tests/integrations/test_openai_agents*.py \
       sdk/python/tests/integrations/test_litellm*.py \
       sdk/python/tests/integrations/test_pydantic_ai*.py \
       sdk/python/tests/integrations/test_agt*.py -v
```

**Pass:** all existing integration tests remain green. Confirms `_default_estimator.py` modification (additive) didn't break dispatcher selection for other adapters.

## G13 — No proto / DB migration drift

```bash
git diff --stat main -- proto/ schema/ migrations/
```

**Pass:** zero changes under `proto/`, `schema/`, `migrations/`. D19 is a pure SDK addition.

## G14 — No new public-API exposure leaks proto types

```bash
python -c "
from spendguard.integrations import adk
exported = [n for n in adk.__all__ if not n.startswith('_')]
assert exported == ['ClaimEstimator', 'RunIdFn', 'SpendGuardAdkCallback'], exported
print(exported)
"
```

**Pass:** public surface is exactly the three documented symbols.

## G15 — Lint + typecheck clean

```bash
cd sdk/python
ruff check src/spendguard/integrations/adk.py tests/integrations/test_adk*.py
mypy --strict src/spendguard/integrations/adk.py
```

**Pass:** zero `ruff` errors and zero `mypy --strict` errors on the new files.

## G16 — End-to-end acceptance summary (the user's stated bar)

Direct quote from the deliverable brief:

> a Python demo using `LlmAgent(model="gemini-2.0-flash", before_model_callback=spendguard_callback, after_model_callback=spendguard_callback)` runs end-to-end with SpendGuard gating + a denial test that shows the LLM call short-circuited.

This is satisfied by **G08** (stubbed) + **G09** (live, gated on secret) together. Marking G08 as the canonical CI-runnable gate; G09 is the human-verifiable closer.

---

## Done definition

D19 is done when **G01-G08 + G10-G15** are green in CI on the merged branch (G09 confirmed locally + screenshot/log attached to the memory write-back per build-plan §8).
