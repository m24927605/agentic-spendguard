# DEMO_MODE=agent_real_agno

COV_D22 SLICE 4 — Agno Python adapter end-to-end demo.

## What this demo proves

`spendguard.integrations.agno.SpendGuardAgnoPreHook` /
`SpendGuardAgnoPostHook` plug into a real `agno.agent.Agent` via
`Agent(pre_hooks=[pre()], post_hooks=[post()])` and route every
`Agent.arun(...)` call through the SpendGuard sidecar's
`RequestDecision(LLM_CALL_PRE)` → `EmitLlmCallPost(SUCCESS)` lifecycle.

* **ALLOW path** — the pre-hook reserves budget BEFORE Agno
  dispatches the model HTTP; the agent reaches the model; the
  post-hook commits with real `run_output.metrics.total_tokens`.
* **DENY path** — `DecisionDenied` is wrapped into Agno's
  `InputCheckError` (DEVIATION-1, see
  `sdk/python/src/spendguard/integrations/agno/_hook.py`); the model
  is never called.
* The same hook pair gates every Agno `Model` backend (OpenAIChat /
  Claude / Gemini / Groq / xAI / DeepSeek / ...) with one registration
  because Agno's first-party extension surface is the callable-based
  hooks list on `Agent` — there is no per-vendor model wrapping.

Both paths land in the ledger DB (`reserve` + `commit_estimated` or
`denied_decision` rows) and the canonical event stream
(`spendguard.audit.decision` + `spendguard.audit.outcome`).

## How to run

```bash
# ALLOW (mock OpenAI via counting-stub, no API key needed):
make demo-up DEMO_MODE=agent_real_agno
```

## Driver

`deploy/demo/demo/run_demo.py` with
`SPENDGUARD_DEMO_MODE=agent_real_agno` constructs:

```python
from agno.agent import Agent
from agno.models.openai import OpenAIChat

from spendguard.integrations.agno import (
    RunContext, SpendGuardAgnoPreHook, SpendGuardAgnoPostHook,
    run_context,
)

pre = SpendGuardAgnoPreHook(
    client=client,
    budget_id="44444444-4444-4444-8444-444444444444",
    window_instance_id="55555555-5555-4555-8555-555555555555",
    unit=common_pb2.UnitRef(
        unit_id="...",
        token_kind="output_token",
        model_family="gpt-4"),
    pricing=common_pb2.PricingFreeze(pricing_version="demo-pricing-v1", ...),
)
post = SpendGuardAgnoPostHook(
    client=client, unit=pre._unit, pricing=pre._pricing,
)

agent = Agent(
    model=OpenAIChat(id="gpt-4o-mini"),
    pre_hooks=[pre()], post_hooks=[post()],
)
async with run_context(RunContext(run_id="my-run-1")):
    response = await agent.arun("Say hello in three words.")
```

A single `SpendGuardAgnoPreHook` / `SpendGuardAgnoPostHook` pair
covers every Agno `Model` provider because gating sits at the
agent-runtime boundary, not the model boundary. The decision_context
tags the live backend (e.g. `model_backend=OpenAIChat`,
`model_id=gpt-4o-mini`) so the audit chain records which model
produced each row.

## Services

* `counting-stub` — mock OpenAI provider on `/v1/chat/completions`
  with a canned 22-token response so the driver can exercise the
  multi-backend coverage proof without real cloud credentials.
* `agno-runner` — Python 3.12 + `agno>=2.0,<3` + `openai>=1.0` +
  `spendguard-sdk[agno]`, drives `run_demo.py` against the sidecar UDS.

## Verification

`make demo-up DEMO_MODE=agent_real_agno` runs
`verify_step_agent_real_agno.sql` against `spendguard_ledger` after
the runner exits, asserting:

* `>= 1` `reserve` row (ALLOW path)
* `>= 1` `commit_estimated` row (ALLOW path commit)
* INV-2 strict order: earliest `reserve` predates earliest
  `commit_estimated`
* Cross-DB: `>= 1` `spendguard.audit.decision` + `>= 1`
  `spendguard.audit.outcome` row in `canonical_events`
* `decision_context_json->>'integration' = 'agno'` on the decision rows

## D05 UnitRef gap (cross-slice tracking)

The cross-slice D05 UnitRef wiring gap surfaces here only at the
outbox-closure step (canonical-events backfill of UnitRef metadata).
The Makefile target tolerates the closure failure with a printed
deferral note — same precedent as D04 / D06 / D07 / D08 / D19 / D20 /
D21 / D29.

## DEVIATIONS vs spec

See `sdk/python/src/spendguard/integrations/agno/__init__.py`
docstring for the full list. Summary:

* **DEVIATION-1**: spec §6.5 said "STOP / DENY raises DecisionDenied,
  Agno propagates". Reality: Agno 2.x's hook loop swallows
  everything except `InputCheckError` / `OutputCheckError`. The
  pre-hook wraps `DecisionDenied` → `InputCheckError`; original
  preserved on `__cause__`.
* **DEVIATION-2**: spec §6.9 said post-hook declares
  `(agent, run_response)`. Reality: Agno 2.x passes the result under
  the key `"run_output"` and filters by parameter name. The closure
  follows reality and declares `run_output`.

Both deviations were forced by Agno 2.x crystallising the
public-surface contract after the spec was authored.

## Out of scope (D22 non-goals)

* `tool_hooks` per-tool budget gating (D22.1).
* Intra-stream chunk gating; commit only at end of stream.
* DEGRADE mutation patch application.

## References

* Spec: `docs/specs/coverage/D22_agno/`
* Module: `sdk/python/src/spendguard/integrations/agno/`
* Docs page: `docs/site-v2/src/content/docs/docs/integrations/agno.mdx`
