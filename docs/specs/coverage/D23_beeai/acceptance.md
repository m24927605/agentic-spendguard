# D23 — BeeAI Framework `Emitter` adapter — Acceptance gates

A reviewer can run every gate below in the current repo state at slice-spec time. No third-party action (PR merge, vendor PR) is a gate. Per build-plan §3, the reviewer (`superpowers:code-reviewer`) re-runs every gate without privileged access.

## 1. Build & install

| ID | Gate | Command |
|----|------|---------|
| `b1` | `pyproject.toml` parses; `beeai` extra resolves on PyPI | `cd sdk/python && uv pip install -e '.[beeai]' --dry-run` (or `pip install --dry-run`) returns success |
| `b2` | Wheel build clean | `cd sdk/python && uv build` produces `spendguard_sdk-*.whl` with no warnings |
| `b3` | Missing-extra ImportError fires | `python -c "import sys; sys.modules['beeai_framework']=None; import spendguard.integrations.beeai"` exits non-zero with the install hint in stderr |

## 2. Lint / type

| ID | Gate | Command |
|----|------|---------|
| `l1` | ruff clean | `ruff check sdk/python/src/spendguard/integrations/beeai.py sdk/python/src/spendguard/integrations/_beeai_inflight.py` exit 0 |
| `l2` | mypy clean | `mypy sdk/python/src/spendguard/integrations/beeai.py` exit 0 |
| `l3` | No accidental beeai import from `__init__` | `python -c "import spendguard; import sys; assert 'beeai_framework' not in sys.modules"` exit 0 (lazy-import contract) |

## 3. Unit tests

| ID | Gate | Command |
|----|------|---------|
| `u1` | All new tests pass | `cd sdk/python && pytest tests/test_beeai_missing_extra.py tests/integrations/test_beeai_skeleton.py tests/integrations/test_beeai_subscribe.py tests/integrations/test_beeai_reserve_commit.py tests/integrations/test_beeai_default_e2e.py -q` exit 0 |
| `u2` | t1–t21 from `tests.md` all green | Above command reports `21 passed` (or more — coverage may add) |
| `u3` | No regression on sibling adapters | `cd sdk/python && pytest tests/integrations -q` exit 0 |
| `u4` | DENY safety property — t20 inverts on bug | Mutation-test scaffold (`# acceptance only`): replace `raise DecisionDenied` with `pass` in client stub; test must fail. Documented in `tests/integrations/test_beeai_reserve_commit.py` docstring as the canary. |

## 4. E2E demo — ALLOW path (`agent_real_beeai`)

Requires Docker + `make` + Python 3.11+. The demo uses BeeAI's `DummyChatModel` by default; if `OPENAI_API_KEY` is set the demo upgrades to real `OpenAIChatModel("gpt-4o-mini")` for parity with `agent_real_langchain`.

| ID | Gate | Command |
|----|------|---------|
| `e1` | Compose comes up | `cd deploy/demo && make demo-up DEMO_MODE=agent_real_beeai` returns exit 0; final stdout line includes `beeai run OK` |
| `e2` | Reserve fired BEFORE provider HTTP | `psql -h localhost -U spendguard -d spendguard -f deploy/demo/demo/verify_beeai.sql` asserts `v1` count 1 (`decision='ALLOW'`) AND `v2` count 1 AND `v3` count 1 |
| `e3` | Single LLM call observed | Container log `docker compose -p spendguard-demo logs upstream-stub` contains exactly 1 line with `POST /v1/chat/completions` (when `OPENAI_API_KEY` unset, the BeeAI DummyChatModel runs in-process; the counter is then `agent.create.call_count == 1` written to `/tmp/beeai_call_count` by the demo driver) |
| `e4` | Inflight map back to 0 | `[demo] inflight_map_size=0` printed after `agent.run` completes |
| `e5` | Clean teardown | `make demo-down` exit 0; no orphan containers (`docker ps -a --filter "label=com.docker.compose.project=spendguard-demo" -q` empty) |

## 5. E2E demo — DENY path (`agent_real_beeai_deny`)

Sidecar boots with `SPENDGUARD_HARD_CAP_ATOMIC=1` so the contract layer refuses anything above 1 atomic unit. The default estimator reserves ≥ 500, guaranteed DENY.

| ID | Gate | Command |
|----|------|---------|
| `e6` | Compose comes up | `cd deploy/demo && make demo-up DEMO_MODE=agent_real_beeai_deny` returns exit 0 |
| `e7` | `DecisionDenied` raised | Driver stdout contains `pre-call gate fired as expected` AND `DecisionDenied: …` (caught by driver) AND driver exits 0 |
| `e8` | No provider HTTP issued | `docker compose -p spendguard-demo logs upstream-stub` contains ZERO lines matching `POST /v1/chat/completions`; OR if running with DummyChatModel, `cat /tmp/beeai_call_count` returns `0` |
| `e9` | Decision audit recorded | `psql ... -f verify_beeai.sql` (DENY variant) asserts: 1 row with `decision='DENY'` in `decision_outbox`; 0 rows in `outcome_outbox` for that run_id; 0 rows in `ledger_movements` with `kind='COMMIT'` |
| `e10` | Clean teardown | `make demo-down` exit 0 |

## 6. Docs

| ID | Gate | Command |
|----|------|---------|
| `d1` | Integration page renders | `cd docs/site && npm run build` exit 0; output includes `Generated /integrations/beeai/` |
| `d2` | README adapter table has BeeAI row | `grep -E "BeeAI.*beeai\\.md" README.md` returns the row |
| `d3` | Example snippet in docs page is copy-paste runnable | The first code block of `docs/site/docs/integrations/beeai.md` is byte-identical to `examples/beeai/quickstart.py` (or a future fixture file) |

## 7. Memory write-back

Per build-plan §8, on D23 ship, write `~/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/project_coverage_D23_shipped.md` — single paragraph: merge commit + slice count + review-round count + arbitration y/n. Acceptance: the file exists.

## 8. Definition of done

D23 is **done** iff:

- `b1`–`b3`, `l1`–`l3`, `u1`–`u4`, `e1`–`e10`, `d1`–`d3` all green.
- All 4 slices (`COV_D23_S1_module_skeleton` → `COV_D23_S4_demo_and_docs`) merged into main.
- `README.md` adapter table updated.
- `docs/site/docs/integrations/beeai.md` shipped.
- Memory file written per §7.

## 9. Anti-acceptance (what is NOT a gate)

- Upstream `beeai-framework` PR merged — D23 is downstream-only; we do not block on IBM/LF.
- TS adapter shipped — explicitly deferred to a separate deliverable.
- `Workflow` mid-step gating — out of scope per design.md §3.
- Real production traffic dashboards — D23's observability is the existing decision/outcome outbox; no new dashboards needed (re-use the predictor dashboards from GA_05).
