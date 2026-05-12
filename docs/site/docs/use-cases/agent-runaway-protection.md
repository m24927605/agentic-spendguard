---
description: >-
  When an AI agent retries the same failed LLM call hundreds of times, the bill
  lands six hours later. Here's how to gate every retry against a budget so a
  runaway loop is mechanically impossible — not just monitored after the fact.
---

# Stopping a runaway AI agent before it bills you

> It's 3 AM. Your LangChain agent has retried the same `gpt-4o` call 47
> times because a downstream tool keeps returning a malformed response.
> Each retry shipped to OpenAI. Each retry charged the provider. By the
> time the on-call rotation notices the alert at 6 AM, the agent has
> burned $400 and produced zero useful output. Token-usage dashboards
> caught it — six hours late.

## Why the standard answer doesn't work

The instinct is to add monitoring: track per-agent spend, alert when
the daily budget hits 80%, page the on-call when it hits 100%. This
is the standard playbook. It fails three ways in the runaway case:

1. **Monitoring is post-hoc.** The agent spent the money before the
   alert page. The alert tells you the bad thing already happened.
2. **Per-call cost is unpredictable.** A retried call may consume
   1,000 tokens or 100,000 tokens depending on context-window growth.
   You can't pre-compute a per-call cap from a daily budget.
3. **Retry loops don't respect rate limits.** Provider rate limits
   bound request count, not dollar amount. A model that costs 10x
   more per token (gpt-4o vs gpt-4o-mini) burns through 10x as much
   under the same rate limit.

The right gate isn't "alert when usage looks weird". The right gate is
"refuse the call if the budget can't cover it". That has to live in
the request path, before the upstream call, every time.

## The pattern that does

Every LLM call goes through a sidecar that holds an auth/capture
ledger. On the request path:

1. The SDK wrapper computes a projected claim (estimated USD or token
   cost based on input length + model pricing).
2. The sidecar checks the projected claim against the budget
   reservation balance.
3. **If the budget can't cover it:** the wrapper raises
   `DecisionStopped`. The LLM call never goes out. Application code
   sees this as a Python exception and propagates it normally.
4. **If the budget can cover it:** the sidecar reserves the amount and
   returns. The wrapper makes the upstream call.

Two properties make this resilient to runaway loops specifically:

- **Idempotent reservations.** A retry with identical inputs (same
  messages + same model settings + same run/step IDs) collapses onto
  the original reservation. A 47-retry loop allocates one reservation,
  not 47.
- **Auto-release on crash.** If the wrapper or pod dies after
  reserving but before completing the upstream call, the reservation
  expires after a configurable TTL (default 600s). The budget is not
  permanently locked.

The net effect: a runaway loop fires its first retry, gets the
reservation, hits the gate on subsequent retries (idempotent
collapse), and stalls on whatever it was failing on — not on
catastrophic spending.

## Show me the code

The LangChain wrapper makes the gate transparent to the agent:

```python
from langchain_openai import ChatOpenAI
from langgraph.prebuilt import create_react_agent

from spendguard import SpendGuardClient
from spendguard.integrations.langchain import (
    RunContext, SpendGuardChatModel, run_context,
)

client = SpendGuardClient(socket_path="...", tenant_id="...")
await client.connect()
await client.handshake()

guarded = SpendGuardChatModel(
    inner=ChatOpenAI(model="gpt-4o"),
    client=client,
    budget_id="my-budget",            # the cap you want enforced
    window_instance_id="hourly-2026",
    # ... pricing + claim_estimator config
)

agent = create_react_agent(guarded, tools=[my_tool])

async with run_context(RunContext(run_id=str(new_uuid7()))):
    await agent.ainvoke({"messages": [HumanMessage(content="...")]})
```

When the budget is exhausted, the agent's next `invoke()` raises
`DecisionStopped` instead of calling OpenAI. The retry loop unwinds
without burning more spend.

## Read more

- [LangChain & LangGraph integration](../integrations/langchain.md) —
  the wrapper used above
- [Pre-call budget caps](pre-call-budget-cap.md) — the reservation
  pattern in detail
- [Quickstart](../quickstart.md) — full stack up in 5 minutes,
  including a DENY demo that exercises this exact path
- [Contract DSL reference](../contracts/yaml.md) — author the rules
  that decide which calls trigger the gate
