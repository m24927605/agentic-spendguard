# DEMO_MODE=agent_real_atomic_agents

COV_D28 SLICE 4 — Atomic Agents (BrainBlend AI) + Instructor (Jason Liu) Python adapter end-to-end demo.

## What this demo proves

`spendguard.integrations.atomic_agents.wrap_instructor_client` wraps
an `instructor.Instructor` (or `AsyncInstructor`) via composition,
intercepts the per-attempt raw provider `create` method that
Instructor's retry loop drives, and gates every call — INCLUDING
Instructor's internal validation-retry attempts — through the
SpendGuard sidecar's `RequestDecision(LLM_CALL_PRE)` →
`EmitLlmCallPost(SUCCESS)` lifecycle.

* **ALLOW path** — the gated raw `create` reserves budget BEFORE
  Instructor dispatches the OpenAI HTTP; the `BaseAgent` reaches the
  provider; the gate commits with real
  `ChatCompletion.usage.total_tokens`.
* **DENY path** — the gated raw raises `DecisionDenied` directly out
  of the per-attempt boundary; the provider HTTP is **never**
  reached. Instructor's outer retry loop may wrap the raise in
  `InstructorRetryException`, but the audit chain records the DENY
  decision row in the ledger BEFORE any provider HTTP fires
  (verified by counting-stub hit counts staying flat on the DENY
  turn).
* **Validation-retry coverage** — when Instructor's Pydantic
  validation re-prompts the provider after a `ValidationError`, each
  retry attempt calls the gated raw `create` independently → each
  gets its own reservation. Wrapping the raw provider method at
  this layer is what makes per-retry gating work (the rejected
  alternative of wrapping `chat.completions.create_with_completion`
  would gate only the OUTER call ONCE per retry loop —
  undercount).

Both paths land in the ledger DB (`reserve` + `commit_estimated` or
`denied_decision` rows) and the canonical event stream
(`spendguard.audit.decision` + `spendguard.audit.outcome`).

## How to run

```bash
# ALLOW (mock OpenAI via counting-stub, no API key needed):
make demo-up DEMO_MODE=agent_real_atomic_agents
```

## Driver

`deploy/demo/demo/run_demo.py` with
`SPENDGUARD_DEMO_MODE=agent_real_atomic_agents` constructs:

```python
import instructor
from openai import OpenAI
from atomic_agents.agents.base_agent import BaseAgent, BaseAgentConfig
from pydantic import BaseModel

from spendguard.integrations.atomic_agents import (
    wrap_instructor_client,
    RunContext, run_context,
)

class Answer(BaseModel):
    final: str

raw = instructor.from_openai(OpenAI())
guarded = wrap_instructor_client(
    raw,
    spendguard_client=client,
    budget_id="44444444-4444-4444-8444-444444444444",
    window_instance_id="55555555-5555-4555-8555-555555555555",
    unit=common_pb2.UnitRef(
        unit_id="...",
        token_kind="output_token",
        model_family="gpt-4"),
    pricing=common_pb2.PricingFreeze(pricing_version="demo-pricing-v1", ...),
    claim_estimator=lambda kwargs: [common_pb2.BudgetClaim(...)],
)

agent = BaseAgent(BaseAgentConfig(
    client=guarded, model="gpt-4o-mini",
    system_prompt_generator=..., input_schema=..., output_schema=Answer,
))

async with run_context(RunContext(run_id="my-run-1")):
    result = agent.run({"query": "What's 2+2?"})
```

A single `SpendGuardInstructorProxy` instance covers every Instructor
backend (OpenAI / Anthropic / Gemini / Cohere) because gating sits at
the raw provider method boundary — wrapped via composition + closure,
no per-vendor adapter required. The decision_context tags the inner
Instructor's class name (e.g. `inner_client=Instructor`,
`integration=atomic_agents`) so the audit chain records which client
produced each row.

## DEVIATIONS

### DEVIATION-A — `atomic-agents>=2.0,<3` pin

Spec design.md §8.2 pinned `atomic-agents>=1.0,<2.0`. Reality
(2026-06-08): the actual PyPI release line is 2.x (latest `2.8.0`);
there is no published 1.x line under the current `atomic-agents`
package name. We pin `>=2.0,<3` so the extra:

1. Fail-closes against a future breaking-change major (3.x line),
2. Floors at the version where `BaseAgent` /
   `BaseAgentConfig(client=<instructor>)` are GA — verified against
   `atomic-agents==2.8.0` from PyPI.

### DEVIATION-B — subpackage layout

Spec implementation.md §1 specified a single flat `atomic_agents.py`
module. We split into a `atomic_agents/` subpackage (`__init__`,
`_errors`, `_options`, `_hook`) mirroring the autogen / beeai / dspy
layout. The import-time guard fires cleanly on a missing extra while
`_hook` stays directly importable for tests that bypass the barrel.

### DEVIATION-C — gate at raw provider method, not `create_with_completion`

Spec design.md §4 described gating at
`Instructor.chat.completions.create_with_completion` with the claim
that "Instructor's internal retries re-enter this proxy → each gets
its own reservation". Reality (verified against
`instructor==1.14.5` and `1.15.1`): Instructor's outer
`create_with_completion` is called ONCE; `instructor.core.retry.retry_sync`
then calls `self.create_fn` per attempt, which is the
`instructor.patch`-wrapped function whose retry loop calls the raw
`openai_client.chat.completions.create` method per attempt.

The correct gate point is the raw provider method
(`inner.client.chat.completions.create`). The proxy:

1. Locates the raw method via
   `inner.client.chat.completions.create` (or
   `inner.create_fn.__wrapped__` fallback).
2. Wraps it with a sync/async gated closure that does PRE / inner /
   POST.
3. Calls `instructor.patch(create=gated_raw, mode=inner.mode)` to
   build a new `create_fn` that drives Instructor's retry loop
   against the gated raw method.

This is the load-bearing intercept point — each retry attempt
naturally re-enters the gate because Instructor's retry loop calls
the raw method per attempt.

## Services

* `counting-stub` — mock OpenAI provider on `/v1/chat/completions`
  serving a function-tool call payload whose `arguments` JSON
  deserializes into the `Answer` Pydantic schema, plus a 32-token
  usage block so the gated raw POST commits a real estimated amount.
* `atomic-agents-runner` — Python 3.12 +
  `spendguard-sdk[atomic-agents]` + `atomic-agents>=2.0,<3` +
  `instructor>=1.5,<2.0` + `openai>=1.0` + `pydantic>=2.0`, drives
  `run_demo.py` against the sidecar UDS.

## Verification

`make demo-up DEMO_MODE=agent_real_atomic_agents` runs
`verify_step_agent_real_atomic_agents.sql` against
`spendguard_ledger` after the runner exits, asserting:

* `>= 1` `reserve` row (ALLOW path)
* `>= 1` `commit_estimated` row (ALLOW path commit)
* INV-2 strict order: earliest `reserve` predates earliest
  `commit_estimated`
* Cross-DB: `>= 1` `spendguard.audit.decision` + `>= 1`
  `spendguard.audit.outcome` row in `canonical_events`
* `decision_context_json->>'integration' = 'atomic_agents'` on the
  decision rows

## D05 UnitRef gap (cross-slice tracking)

The cross-slice D05 UnitRef wiring gap surfaces here only at the
outbox-closure step (canonical-events backfill of UnitRef metadata).
The Makefile target tolerates the closure failure with a printed
deferral note — same precedent as D04 / D06 / D07 / D08 / D19 / D20 /
D21 / D22 / D23 / D24 / D25 / D26 / D27.

## Out of scope (D28 non-goals)

* Streaming (`instructor.Partial[...]` / `Iterable[...]`) — design.md
  §3 non-goal; commit only after the final parsed response via
  Instructor's standard `create_with_completion` path.
* Wrapping `client.messages.create` (Anthropic-native Instructor
  surface) — Atomic Agents documents `chat.completions`.
* Patching `BaseAgent` directly — surface churns per release.
* Wrapping Instructor's `Mode` selection logic — Instructor's concern.
* An Atomic Agents `Hook`-system PR upstream — none exists; not
  critical path.

## References

* Spec: `docs/specs/coverage/D28_atomic_agents/`
* Module: `sdk/python/src/spendguard/integrations/atomic_agents/`
* Docs page: `docs/site-v2/src/content/docs/docs/integrations/atomic-agents.mdx`
* Test suite: `sdk/python/tests/integrations/atomic_agents/`
