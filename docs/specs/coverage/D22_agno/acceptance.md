# D22 — Acceptance

A slice is "shipped" iff every gate below runs green from a clean clone with no privileged access.

## 1. Repo-state gates (build / lint / type)

| # | Gate | Command | Pass condition |
|---|---|---|---|
| A1 | Module importable, extra resolves | `python -c "from spendguard.integrations.agno import SpendGuardAgnoPreHook, SpendGuardAgnoPostHook, RunContext, run_context"` (after `pip install -e 'sdk/python[agno]'`) | exit 0, no traceback |
| A2 | Module importable WITHOUT the extra → friendly `ImportError` | `python -c "import spendguard.integrations.agno"` in env where `agno` is NOT installed | `ImportError` mentions `pip install 'spendguard-sdk[agno]'` |
| A3 | mypy passes on the new module | `cd sdk/python && mypy src/spendguard/integrations/agno.py` | exit 0 |
| A4 | ruff passes | `cd sdk/python && ruff check src/spendguard/integrations/agno.py tests/integrations/test_agno_pre_post.py tests/integrations/test_agno_default_estimator.py` | exit 0 |
| A5 | Wheel build still produces `spendguard-sdk-*.whl` | `cd sdk/python && python -m build --wheel` | wheel built; contains `spendguard/integrations/agno.py` |

## 2. Unit + integration tests

| # | Gate | Command | Pass condition |
|---|---|---|---|
| B1 | All ≥ 22 unit cases pass | `cd sdk/python && pytest tests/integrations/test_agno_pre_post.py -v` | all PASSED |
| B2 | Default-estimator cases pass | `cd sdk/python && pytest tests/integrations/test_agno_default_estimator.py -v` | all PASSED |
| B3 | Coverage ≥ 85% on `agno.py` | `cd sdk/python && pytest tests/integrations/test_agno_pre_post.py tests/integrations/test_agno_default_estimator.py --cov=spendguard.integrations.agno --cov-fail-under=85` | exit 0 |
| B4 | Integration `test_real_agent_with_stub_openai` proves PRE before vendor SDK | grep test output for `provider_event_id` assertion | PASSED |
| B5 | `test_real_agent_deny_short_circuits` proves the OpenAI stub is NEVER invoked on STOP | `MockAsyncOpenAI.call_count == 0` after `DecisionDenied` raised | PASSED |
| B6 | `test_real_agent_provider_error` reports `outcome="PROVIDER_ERROR"` | mock client recorded outcome PROVIDER_ERROR | PASSED |
| B7 | Closure signature names preserved | tests #16, #17 in `tests.md` PASS | `inspect.signature` matches `(agent, run_input)` / `(agent, run_response)` |

## 3. Demo mode `agent_real_agno`

Repository state after slice 4 must satisfy:

| # | Gate | Command | Pass condition |
|---|---|---|---|
| C1 | `agent_real_agno` branch wired in dispatcher | `grep -n 'agent_real_agno' deploy/demo/demo/run_demo.py` | matches a `if DEMO_MODE == "agent_real_agno":` branch |
| C2 | Demo script exists | `test -f examples/agno-prehooks/run.py && test -f examples/agno-prehooks/README.md` | both files present |
| C3 | Makefile target | `make -n demo-agent-real-agno` | non-zero stub output, no "no rule to make target" |
| C4 | End-to-end demo PASS against the local sidecar | `make demo-agent-real-agno` (with `OPENAI_API_KEY` set; CI uses a recorded fixture) | exit 0; stdout shows `reserve` → `commit_estimated` events; sidecar audit_outbox contains one `LLM_CALL_PRE` decision row and one `LLM_CALL_POST` row for the run |
| C5 | Deny path proves OpenAI was NOT reached | `DEMO_MODE=agent_real_agno DENY=1 make demo-agent-real-agno` | exit code 13 (DenyExit); stderr contains `DecisionDenied`; OpenAI request log absent in sidecar capture |
| C6 | `scripts/check-demo-modes.py` recognises the new mode | `python scripts/check-demo-modes.py` | exit 0; `agent_real_agno` listed in supported modes |

## 4. Docs

| # | Gate | Command | Pass condition |
|---|---|---|---|
| D1 | Integration page exists | `test -f docs/site/docs/integrations/agno.md` | present, ≥ 400 words, sections: Install / Usage / How it works / Limitations |
| D2 | README adapter table row added | `grep -nE '\| Agno \|' README.md` | row exists with the `[agno]` extra and `agno >= 1.0` pin |
| D3 | `docs/site/sidebars.ts` or `astro.config` entry includes the new page | `grep -n 'integrations/agno' docs/site/` | match found |
| D4 | Internal cross-reference: framework-coverage strategy doc lists D22 status | `grep -n 'D22' docs/strategy/framework-coverage-build-plan-2026-06.md` | row reflects "shipped" or `agno` adapter present |

## 5. Memory-bank write-back

Final slice commit must, per `framework-coverage-build-plan-2026-06.md` §8, append a project-memory file `project_coverage_D22_shipped.md` with: merge commit SHA, R-rounds taken, arbitration y/n, closed issues, and the demo gates that passed. Gate: `test -f ~/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/project_coverage_D22_shipped.md` after final merge.

## 6. Non-goals NOT regressed

| # | Gate | Command | Pass condition |
|---|---|---|---|
| E1 | LangChain integration tests still pass | `cd sdk/python && pytest tests/integrations/test_langchain.py` | all PASSED |
| E2 | OpenAI Agents integration tests still pass | `cd sdk/python && pytest tests/integrations/test_openai_agents.py` | all PASSED |
| E3 | `_default_estimator.py` unchanged behaviour for the existing four estimators | `cd sdk/python && pytest tests/integrations/test_default_estimator.py` | all PASSED |
| E4 | Wheel size delta ≤ +5 KB | `du -b dist/spendguard_sdk-*.whl` before vs after | delta ≤ 5 KB (no accidental asset inclusion) |

## 7. Slice-level pass-through gates

| Slice | Gate set |
|---|---|
| `COV_D22_01_module_skel_extra` | A1, A2, A3, A4 |
| `COV_D22_02_pre_post_hooks` | A1, A3, A4, A5, B1 (subset: signature derivation + inflight cases), B2 |
| `COV_D22_03_tests_mock_sidecar` | B1 (all), B3, B4, B5, B6, B7, E1, E2, E3 |
| `COV_D22_04_demo_docs` | C1-C6, D1-D4, E4, and the memory-bank write-back |

A slice is "passed" by `superpowers:code-reviewer` (R1) iff every assigned gate above is green AND `findings == 0` against `review-standards.md`. R5 panel arbitration triggers only on five consecutive non-zero reviews.
