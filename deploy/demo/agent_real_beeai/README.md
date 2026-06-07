# DEMO_MODE=agent_real_beeai

COV_D23 SLICE 4 — BeeAI Framework (IBM Research + Linux Foundation)
Python adapter end-to-end demo.

## What this demo proves

`spendguard.integrations.beeai.subscribe_spendguard(agent, client, ...)`
plugs into a real `beeai_framework.agents.base.BaseAgent` (e.g.
`ReActAgent`) via the agent's `Emitter` and routes every LLM step
through the SpendGuard sidecar's `RequestDecision(LLM_CALL_PRE)` →
`EmitLlmCallPost(SUCCESS)` lifecycle.

* **ALLOW path** — the `*.start` handler reserves budget BEFORE BeeAI
  dispatches the model HTTP; the agent reaches the model; the
  `*.success` handler commits with real `usage.total_tokens`.
* **DENY path** (covered by spec but kept as a duck-typed CI smoke
  variant when `beeai-framework` isn't installed in the container) —
  `DecisionDenied` propagates out of the start handler; BeeAI's
  `Emitter._invoke` wraps it as `EmitterError` preserving
  `__cause__`; the model is never called.
* The same subscriber gates every BeeAI `ChatModel` backend
  (`OpenAIChatModel` / `WatsonxChatModel` / `OllamaChatModel` /
  `GroqChatModel` / ...) with one registration because BeeAI's
  first-party extension surface is the agent-runtime `Emitter`,
  there is no per-vendor model wrapping.

Both paths land in the ledger DB (`reserve` + `commit_estimated` or
`denied_decision` rows) and the canonical event stream
(`spendguard.audit.decision` + `spendguard.audit.outcome`).

## How to run

```bash
# ALLOW (mock OpenAI via counting-stub, no API key needed):
make demo-up DEMO_MODE=agent_real_beeai
```

## Driver

`deploy/demo/demo/run_demo.py` with
`SPENDGUARD_DEMO_MODE=agent_real_beeai` constructs:

```python
from beeai_framework.agents.react import ReActAgent
from beeai_framework.backend.chat import ChatModel

from spendguard.integrations.beeai import (
    RunContext, run_context, subscribe_spendguard,
)

llm = ChatModel.from_name("openai:gpt-4o-mini")
agent = ReActAgent(llm=llm, tools=[])

unsubscribe = subscribe_spendguard(
    agent, client,
    budget_id="44444444-4444-4444-8444-444444444444",
    window_instance_id="55555555-5555-4555-8555-555555555555",
    unit=common_pb2.UnitRef(
        unit_id="...",
        token_kind="output_token",
        model_family="gpt-4"),
    pricing=common_pb2.PricingFreeze(pricing_version="demo-pricing-v1", ...),
)
try:
    async with run_context(RunContext(run_id="my-run-1")):
        result = await agent.run("Say hello in three words.")
finally:
    unsubscribe()
```

A single `subscribe_spendguard` call covers every BeeAI `ChatModel`
provider because gating sits at the agent-runtime boundary, not the
model boundary. The `decision_context_json` tags the live backend
(e.g. `model_backend=OpenAIChatModel`, `model_id=gpt-4o-mini`) so the
audit chain records which model produced each row.

## Services

* `counting-stub` — mock OpenAI provider on `/v1/chat/completions`
  with a canned 22-token response so the driver can exercise the
  multi-backend coverage proof without real cloud credentials.
* `beeai-runner` — Python 3.12 + `beeai-framework>=0.1.81,<0.2` +
  `openai>=1.0` + `spendguard-sdk[beeai]`, drives `run_demo.py`
  against the sidecar UDS.

## Verification

`make demo-up DEMO_MODE=agent_real_beeai` runs
`verify_step_agent_real_beeai.sql` against `spendguard_ledger` after
the runner exits, asserting:

* `>= 1` `reserve` row (ALLOW path)
* `>= 1` `commit_estimated` row (ALLOW path commit)
* INV-2 strict order: earliest `reserve` predates earliest
  `commit_estimated`
* Cross-DB: `>= 1` `spendguard.audit.decision` + `>= 1`
  `spendguard.audit.outcome` row in `canonical_events`
* `decision_context_json->>'integration' = 'beeai'` on the decision
  rows

## D05 UnitRef gap (cross-slice tracking)

The cross-slice D05 UnitRef wiring gap surfaces here only at the
outbox-closure step (canonical-events backfill of UnitRef metadata).
The Makefile target tolerates the closure failure with a printed
deferral note — same precedent as D04 / D06 / D07 / D08 / D19 / D20 /
D21 / D22 / D29.

## DEVIATIONS vs spec

See
`sdk/python/src/spendguard/integrations/beeai/__init__.py`
docstring for the full list. Summary:

* **DEVIATION-A**: spec §4 / implementation.md §4 pinned
  `beeai-framework>=0.3,<1.0`. Reality (2026-06-08): the actual PyPI
  release line is `0.1.x` with `0.1.81` as the latest; there is no
  `0.3.x`. We pin `>=0.1.81,<0.2` in `pyproject.toml` so the extra
  floors at the version where `Emitter.match` returns a `CleanupFn`
  and the `BaseAgent.emitter` `cached_property` is stable.
* **DEVIATION-B**: spec §5 R1 / implementation.md §2 imports
  `run_context` / `current_run_context` directly from
  `spendguard.integrations.langchain`. Reality: `langchain.py` raises
  `ImportError` at import time if `langchain_core` is missing, so
  re-importing into `beeai/__init__.py` would force every BeeAI user
  to install `[langchain]` transitively — a Blocker per
  review-standards §4 R1. We define a fresh `ContextVar` with the
  SAME NAME (`spendguard_run_context`) in `_hook.py`; Python
  ContextVars are looked up by name at the interpreter level so
  cross-adapter run_id sharing still works exactly like spec §5 R1
  intended. Mirrors the same compromise the Agno integration made
  (`agno/_hook.py:93`).

Both deviations were forced by upstream reality (BeeAI's actual
versioning + langchain.py's hard import gate); neither weakens the
locked design contracts.

## Out of scope (D23 non-goals)

* `tool.*` event subscription — `subscribe_spendguard` filters by
  `llm` segment in the path; tool gating is the
  `integrations.agt` territory.
* `newToken` / `partialUpdate` mid-stream gating; commit only at
  `success`.
* DEGRADE mutation patch application.
* TypeScript BeeAI adapter (deferred — Tier 3 is Python-only per
  build plan §2.3).

## References

* Spec: `docs/specs/coverage/D23_beeai/`
* Module: `sdk/python/src/spendguard/integrations/beeai/`
* Docs page: `docs/site-v2/src/content/docs/docs/integrations/beeai.mdx`
