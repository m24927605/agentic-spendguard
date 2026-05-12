---
description: >-
  Pre-call token budget enforcement for Pydantic-AI agents using SpendGuard.
  Every Model.request() is reserved against a budget before the LLM call is
  sent, with signed audit trail and human-approval flow on over-budget calls.
---

# Pydantic-AI budget control with SpendGuard

> Your Pydantic-AI agent calls `agent.run("...")` and the run loop dispatches
> `Model.request()` repeatedly — once per step, once per retry, once per
> multi-step tool loop. Without a gate, every iteration is a free shot at the
> provider. SpendGuard wraps the model so each `request()` reserves against a
> budget *before* the upstream LLM call ships.

## Why you'd want this

- **Pre-call enforcement, not post-hoc dashboards.** Reservation
  happens before the OpenAI/Anthropic call. Over-budget calls raise
  `DecisionStopped` and the upstream request never goes out.
- **Retry-safe idempotency.** Pydantic-AI re-enters `request()` on
  transient errors. SpendGuard derives a stable `idempotency_key`
  from messages + settings + run_id, so the retry collapses onto the
  original reservation instead of allocating a new one.
- **Tool loops stay budgeted.** Multi-step tool-using agents are
  gated on every model call, including steps spawned by tool output.
- **Audit trail.** Every decision (allow / stop / require_approval /
  degrade) is signed and chained for post-hoc analysis.
- **Human-in-the-loop approval.** Pause-and-resume with
  `await e.resume(client)` when a contract fires `REQUIRE_APPROVAL`.

## Setup (60 seconds)

```bash
pip install 'spendguard-sdk[pydantic-ai]'
```

You also need a running SpendGuard sidecar reachable on a Unix Domain
Socket. The fastest path is the demo stack:

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard && make demo-up
```

The demo binds the sidecar UDS at `deploy/demo/runtime/uds/adapter.sock`.

## Wire it up

```python
import asyncio

from pydantic_ai import Agent
from pydantic_ai.models.openai import OpenAIModel

from spendguard import SpendGuardClient, new_uuid7
from spendguard.integrations.pydantic_ai import (
    RunContext, SpendGuardModel, run_context,
)
from spendguard._proto.spendguard.common.v1 import common_pb2


async def main() -> None:
    client = SpendGuardClient(
        socket_path="/var/run/spendguard/adapter.sock",
        tenant_id="00000000-0000-4000-8000-000000000001",
    )
    await client.connect()
    await client.handshake()

    guarded = SpendGuardModel(
        inner=OpenAIModel("gpt-4o-mini"),
        client=client,
        budget_id="my-budget",
        window_instance_id="my-window",
        unit=common_pb2.UnitRef(
            unit_id="usd_micros",
            token_kind="usd_micros",
            model_family="gpt-4",
        ),
        pricing=common_pb2.PricingFreeze(pricing_version="2025-q4"),
        claim_estimator=lambda messages, settings: [
            common_pb2.BudgetClaim(
                budget_id="my-budget",
                window_instance_id="my-window",
                amount_micros=1_000_000,  # 1 USD reservation per call
            )
        ],
    )

    agent = Agent(model=guarded)
    async with run_context(RunContext(run_id=str(new_uuid7()))):
        result = await agent.run("Hello")
        print(result.output)


asyncio.run(main())
```

## What you get

- **Pre-call budget reservation.** The wrapped model raises
  `DecisionStopped` instead of calling the LLM when the reservation
  would exceed the budget.
- **Signed audit chain.** Every decision is recorded in the ledger
  with a cryptographic signature; replay-safe via the `audit_outbox`
  transactional pattern.
- **Approval continuation.** When a contract fires `REQUIRE_APPROVAL`,
  the exception carries `e.resume(client)` — call it after an
  operator approves in the dashboard.

## Common patterns

### Per-tenant budgets

Pass distinct `budget_id` / `window_instance_id` values per tenant.
The control plane API (`POST /v1/budgets`) provisions budgets
without restarting the agent.

### Handling approvals

```python
from spendguard import ApprovalRequired

try:
    result = await agent.run(prompt)
except ApprovalRequired as e:
    await wait_for_operator_approval(e.decision_id)
    result = await e.resume(client)
```

### Testing without burning tokens

Replace `OpenAIModel` with `pydantic_ai.models.test.TestModel`. The
SpendGuard wrapper still records reservations and decisions, so you
can unit-test budget logic without provider keys.

## Related

- [Quickstart](../quickstart.md) — full stack up in 5 minutes
- [Contract DSL reference](../contracts/yaml.md) — author allow/stop rules
- Other integrations: [LangChain & LangGraph](langchain.md) · [OpenAI Agents SDK](openai-agents.md) · [Microsoft AGT](agt.md)
