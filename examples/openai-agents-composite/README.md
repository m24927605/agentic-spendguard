# OpenAI Agents SDK + SpendGuard — runnable example

> **Status: first-party reference example.** Demonstrates how to wrap an
> `agents.Agent`'s model with `SpendGuardAgentsModel` so every step inside
> `Runner.run()` is gated against a budget before the OpenAI HTTP request
> is issued.

This is the parallel of `deploy/demo/demo/run_demo.py::run_openai_agents_mode`
extracted into a standalone, copy-pasteable script so contributors and
reviewers can try the integration without spinning up the full demo Docker
stack.

## What this proves

The example exercises a single hard invariant of the integration:

> **SpendGuard DENY ⇒ the inner Model is NEVER invoked.**

If you point your OpenAI client at SpendGuard's egress proxy (or wrap your
agent's Model with `SpendGuardAgentsModel`), an exhausted budget short-circuits
**before** the provider HTTP request leaves your process. The provider invoice
clock never starts.

Two modes:

| Mode | Dependencies | What it shows |
|---|---|---|
| `--mock` (default) | Python stdlib only | The PRE→ALLOW→LLM and PRE→DENY→short-circuit contracts, in-process, fully assertable |
| `--real` | `openai-agents>=0.17` + `spendguard-sdk[openai-agents]` + live sidecar + `OPENAI_API_KEY` | Full end-to-end against gpt-4o-mini and the real signed audit chain |

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│  agents.Agent (your code, unchanged)                          │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  SpendGuardAgentsModel (subclasses agents.Model)       │  │
│  │    1) PRE  → SpendGuard sidecar request_decision()     │  │
│  │       DENY → raise → Runner.run propagates             │  │
│  │       ALLOW → call inner.get_response()                │  │
│  │    2) inner = OpenAIChatCompletionsModel(...)          │  │
│  │    3) POST → emit_llm_call_post (commit reservation)   │  │
│  └────────────────────────────┬───────────────────────────┘  │
└────────────────────────────────┼─────────────────────────────┘
                                 │ Unix domain socket
                                 ▼
                  ┌──────────────────────────────┐
                  │ SpendGuard sidecar (Rust)    │
                  │   • contract DSL evaluator   │
                  │   • per-pod fencing lease    │
                  └──────────────┬───────────────┘
                                 │ mTLS gRPC
                                 ▼
                  ┌──────────────────────────────┐
                  │ Postgres ledger              │
                  │   • Stripe-style auth/cap.   │
                  │   • signed append-only audit │
                  └──────────────────────────────┘
```

## Prerequisites

### For `--mock` (recommended first run)

None. The mock mode uses zero non-stdlib dependencies — `requirements.txt` is
only for `--real`.

### For `--real`

```bash
pip install --pre -r examples/openai-agents-composite/requirements.txt
export OPENAI_API_KEY=sk-...
```

Plus a live SpendGuard sidecar + ledger:

```bash
# In a separate shell, from the repo root
make demo-up
```

`make demo-up` brings up sidecar + Postgres + auto-seeded demo tenant/budget
via Docker Compose. UUIDs are pinned to the defaults this script uses; no
flags required for the standard demo setup.

## How to run

```bash
# Mock mode — runs offline, no API key, no Docker
python examples/openai-agents-composite/openai_agents_composite_demo.py --mock

# Real mode — requires sidecar + OPENAI_API_KEY
OPENAI_API_KEY=sk-... \
  python examples/openai-agents-composite/openai_agents_composite_demo.py --real
```

## Expected output — `--mock`

```
============================================================
  SpendGuard + OpenAI Agents SDK Demo (mock mode)
============================================================

--- Setup ---
  Inner model: MOCK (canned response — no OpenAI API key needed)
  SpendGuard transport: MOCK (in-process — no sidecar)
  Budget cap: 1000 atomic units, 800 already used (200 remaining)

--- Path 1: ALLOW (budget has room) ---
  Prompt: 'Say hello (cheap)'  (estimated 100 atomic units)
  PRE decision: ALLOW
  LLM called: True
  Response: '[mock-llm] echo: Say hello (cheap)'
  Remaining budget: 100 atomic units

--- Path 2: DENY (budget exhausted) ---
  Prompt: 'Generate a long essay'  (estimated 500 atomic units)
  PRE decision: DENY (BUDGET_EXHAUSTED)
  LLM called: False
  Provider HTTP request was NOT issued — fail-closed enforcement works as expected.

--- Path 3: ledger state after the run ---
  PRE calls recorded: 2 (expected 2)
  LLM calls recorded: 1 (expected 1)
  Atomic consumed:    100 (expected 100)

============================================================
  All paths PASS — wrapper invariant verified:
    SpendGuard DENY ⇒ inner Model is NEVER invoked
============================================================
```

## What the mock mode does NOT do

In the interest of running offline, the mock mode trades fidelity for
simplicity:

- It does not subclass `agents.models.interface.Model`, so the wrapper's
  exact `get_response` signature (system_instructions, model_settings,
  tools, output_schema, handoffs, tracing, …) is not exercised. The real
  wrapper in `sdk/python/src/spendguard/integrations/openai_agents.py`
  does conform.
- It does not exercise the `RunContext` / `run_context()` contextvar
  contract — that's only used by the real wrapper to share trace
  identifiers with sibling integrations (LangChain, Pydantic-AI).
- It does not serialize `BudgetClaim` protos or emit the
  `LLM_CALL_POST` reservation commit. Those happen against a real ledger.

The single invariant the mock mode *does* exercise — `DENY ⇒ no inner call` —
is the one that catches the largest class of integration regressions and is
what the AGT analogue (`examples/spendguard-composite/`) verifies for AGT.

## Related

- **Full demo stack with audit-chain verification**:
  `DEMO_MODE=agent_real_openai_agents make demo-up`
- **Multi-step agent variant**: `DEMO_MODE=agent_real_openai_agents_multistep`
- **Egress-proxy variant (no SDK changes)**:
  `DEMO_MODE=agent_real_openai_agents_proxy` — points `OPENAI_BASE_URL` at the
  proxy; no code change required.
- **AGT analogue**: [`examples/spendguard-composite/`](https://github.com/microsoft/agent-governance-toolkit/blob/main/examples/spendguard-composite/)
  is the equivalent for Microsoft's Agent Governance Toolkit.
- **OpenAI Cookbook notebook**:
  [`budget_guardrails_with_spendguard.ipynb`](https://github.com/openai/openai-cookbook/pull/2722)
  — a Jupyter-notebook variant suitable for `Runner.run()` walkthroughs.
- **Integration doc**: [`docs/site/docs/integrations/openai-agents.md`](../../docs/site/docs/integrations/openai-agents.md)
- **SDK source**: [`sdk/python/src/spendguard/integrations/openai_agents.py`](../../sdk/python/src/spendguard/integrations/openai_agents.py)
