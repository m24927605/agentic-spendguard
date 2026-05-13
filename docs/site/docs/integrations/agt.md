---
description: >-
  Composite policy evaluator combining Microsoft AGT (Agent Governance Toolkit)
  with Agentic SpendGuard. AGT's deterministic access check runs first; SpendGuard's
  budget reservation runs on AGT-allowed actions only — no wasted reservations
  on AGT denies.
---

# Microsoft AGT (Agent Governance Toolkit) integration

> Microsoft's Agent Governance Toolkit handles deterministic policy
> (this user can call this tool, this tenant can access this data).
> SpendGuard handles spend-shaped policy (this budget can afford this
> call right now). Run them in a composite: AGT vetoes cheap;
> SpendGuard reserves on the remainder. AGT-denied actions never
> consume a SpendGuard reservation.

## Why you'd want this

- **Layered governance.** Deterministic access control (AGT) plus
  spend control (SpendGuard) behind a single `evaluate()` entry
  point.
- **No wasted reservations.** AGT-denies short-circuit before the
  SpendGuard sidecar call, so denied actions cost nothing in the
  ledger.
- **Two audit trails, reconcilable.** AGT writes its own audit log;
  SpendGuard writes to `canonical_events`. A relay that ingests AGT
  events into the SpendGuard chain is on the roadmap (out of scope
  for the integration itself).

## Setup (60 seconds)

```bash
pip install 'spendguard-sdk[agt]'
```

## Wire it up

```python
import asyncio

from agent_os.policies import (
    PolicyEvaluator, PolicyDocument, PolicyRule, PolicyCondition,
    PolicyAction, PolicyOperator, PolicyDefaults,
)

from spendguard import SpendGuardClient, new_uuid7
from spendguard.integrations.agt import SpendGuardCompositeEvaluator
from spendguard._proto.spendguard.common.v1 import common_pb2


async def main() -> None:
    # 1. AGT: deterministic access policy
    agt = PolicyEvaluator(policies=[
        PolicyDocument(
            id="block-untrusted-tools",
            rules=[
                PolicyRule(
                    when=PolicyCondition(
                        field="tool_name",
                        operator=PolicyOperator.IN,
                        value=["web_search", "calculator"],
                    ),
                    action=PolicyAction.ALLOW,
                ),
            ],
            defaults=PolicyDefaults(action=PolicyAction.DENY),
        )
    ])

    # 2. SpendGuard: budget reservation
    client = SpendGuardClient(
        socket_path="/var/run/spendguard/adapter.sock",
        tenant_id="00000000-0000-4000-8000-000000000001",
    )
    await client.connect()
    await client.handshake()

    # 3. Composite
    composite = SpendGuardCompositeEvaluator(
        agt_evaluator=agt,
        spendguard_client=client,
        budget_id="my-budget",
        window_instance_id="my-window",
        unit=common_pb2.UnitRef(
            unit_id="usd_micros",
            token_kind="usd_micros",
            model_family="gpt-4",
        ),
        pricing=common_pb2.PricingFreeze(pricing_version="2025-q4"),
        claim_estimator=lambda payload: [
            common_pb2.BudgetClaim(
                budget_id="my-budget",
                window_instance_id="my-window",
                amount_micros=1_000_000,
            )
        ],
    )

    # 4. Evaluate
    result = await composite.evaluate({
        "tool_name": "web_search",
        "tool_args": {"q": "AI agent budget control"},
        "tenant_id": "...",
        "run_id": str(new_uuid7()),
    })
    print(result.allowed, result.reason)
    # result.allowed: bool
    # result.reason: "AGT_DENY: ..." | "SPENDGUARD_DENY: ..." | "ALLOW (...)"


asyncio.run(main())
```

## What you get

- **AGT-first short-circuit.** If AGT denies, SpendGuard isn't
  called — no reservation is allocated and no `canonical_events`
  row is written for that action.
- **SpendGuard budget reservation** for every AGT-ALLOW action.
- **Distinct reason strings** so you can tell whether a deny came
  from AGT or SpendGuard without digging into either log.

## Common patterns

### Cross-system reconciliation

For now, AGT events and SpendGuard events live in separate stores.
If you need a unified view, ingest the AGT audit log into your data
warehouse alongside `canonical_events` and join on `decision_id`
(emitted by both).

### Per-tenant AGT + SpendGuard config

Construct one `SpendGuardCompositeEvaluator` per tenant. AGT
policies and SpendGuard `budget_id` then both reflect tenant-specific
policy without runtime branching.

## Related

- [Quickstart](../quickstart.md) — full stack up in 5 minutes
- [Contract DSL reference](../contracts/yaml.md) — author allow/stop rules
- Other integrations: [Pydantic-AI](pydantic-ai.md) · [LangChain & LangGraph](langchain.md) · [OpenAI Agents SDK](openai-agents.md)
