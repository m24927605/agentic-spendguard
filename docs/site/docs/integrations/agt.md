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

---

## Already using AGT? Three paths to add SpendGuard

Pick the one that matches how your existing AGT code is structured.
All three assume the [prerequisites](#prerequisites-one-time-setup) below are in place.

### Path A — Composite Evaluator *(recommended; least code change)*

You have an AGT `PolicyEvaluator` that gates tool actions. Wrap it once;
SpendGuard runs only on AGT-allowed actions.

```python
# Before — pure AGT
from agent_os.policies import PolicyEvaluator, PolicyDocument, ...

agt = PolicyEvaluator(policies=[...your existing rules...])
result = await agt.evaluate({"tool_name": "execute_code", ...})
```

```python
# After — composite (AGT first, SpendGuard on AGT-allow only)
from spendguard import SpendGuardClient
from spendguard.integrations.agt import SpendGuardCompositeEvaluator
from spendguard._proto.spendguard.common.v1 import common_pb2

async with SpendGuardClient(
    socket_path="/var/run/spendguard/adapter.sock",
    tenant_id="<your-tenant-uuid>",
) as sg:
    await sg.handshake()

    composite = SpendGuardCompositeEvaluator(
        agt_evaluator=agt,                       # ← existing AGT object, unchanged
        spendguard_client=sg,
        budget_id="<budget-uuid>",
        window_instance_id="<window-uuid>",
        unit=common_pb2.UnitRef(
            unit_id="<unit-uuid>",
            token_kind="output_token",
            model_family="gpt-4",
        ),
        pricing=common_pb2.PricingFreeze(pricing_version="...", ...),
        claim_estimator=lambda payload: [
            common_pb2.BudgetClaim(
                budget_id="<budget-uuid>",
                amount_atomic="500",             # ← your estimate per tool action
                unit=common_pb2.UnitRef(unit_id="<unit-uuid>"),
            )
        ],
    )
    result = await composite.evaluate({"tool_name": "execute_code", ...})
    # result.allowed: bool
    # result.reason: "AGT_DENY: ..." | "SPENDGUARD_DENY: ..." | "ALLOW (AGT + SpendGuard both PASS)"
    # result.matched_rule_ids: list[str]
```

**Your AGT rules are untouched.** SpendGuard runs only after AGT allows; AGT-deny short-circuits the sidecar call → no wasted reservation.

### Path B — `gate_budget()` hook *(for policy-callback-driven flows)*

If AGT already runs an async callback inside one of its policies, add a single line:

```python
from spendguard.integrations.agt import gate_budget

async def my_existing_policy_callback(payload):
    # ... your existing AGT logic ...

    await gate_budget(                           # ← new line
        payload,
        client=sg,
        budget_id="<budget-uuid>",
        window_instance_id="<window-uuid>",
        unit=unit,
        pricing=pricing,
        claim_estimator=estimator,
    )
    # `gate_budget` raises DecisionDenied if SpendGuard denies;
    # AGT chain surfaces that as a hard policy fail.

    return PolicyDecision.ALLOW
```

### Path C — Parallel call *(maximum flexibility)*

If your architecture has multiple evaluators and you want SpendGuard to live alongside (not inside) AGT, call `sg.request_decision(...)` directly wherever you want a budget gate. You're responsible for sequencing it against AGT yourself.

```python
# Whenever you decide to budget-gate, regardless of AGT
outcome = await sg.request_decision(
    trigger="LLM_CALL_PRE",
    run_id=run_id, step_id=step_id, llm_call_id=call_id,
    decision_id=decision_id, route="llm.call",
    projected_claims=[claim],
    idempotency_key=derive_idempotency_key(...),
)
```

---

## Prerequisites (one-time setup)

| Step | What |
|---|---|
| 1. **Sidecar deployed** | Helm: `helm install spendguard charts/spendguard` (DaemonSet — one pod per node). Docker Compose: `make demo-up` for local dev. |
| 2. **Postgres ledger reachable** | Pass connection string via `postgres.ledgerUrl` + `postgres.canonicalUrl` (Helm) or via the demo's compose config. |
| 3. **Tenant + budget seeded** | Insert via the control-plane REST API (`POST /v1/tenants`, `POST /v1/budgets`) or directly via the SP at install time. |
| 4. **Contract bundle published** | Write a `contract.yaml` with your rules (e.g. `hard-cap-deny when claim > 1B`), pack via `bundles-init` ConfigMap or your operator's bundle pipeline. |
| 5. **Python SDK installed** | `pip install --pre 'spendguard-sdk[agt]'` — pulls `agent-governance-toolkit>=3.4` + `agent-os-kernel>=3.0` as transitive deps. The chart is on PyPI as alpha so the `--pre` flag is required. |

---

## Operational gotchas

- **Reservation TTL defaults to 60s.** If the AGT-allowed tool action runs longer than that (a long shell command, a multi-turn LLM call), SpendGuard auto-releases the reservation. For long tool calls, bump `reservation_ttl_seconds` in the contract bundle's budget block, or pass `X-SpendGuard-Reservation-TTL` (proxy path).
- **AGT-deny actions do NOT appear in `canonical_events`.** AGT short-circuits before SpendGuard runs. AGT's own audit log captures the deny; SpendGuard's chain captures only AGT-allow → SpendGuard reservation/commit/release. The two chains can be reconciled on `decision_id` (both emit it) when an AGT → SpendGuard relay is shipped.
- **Composite `result.reason` follows AGT's verdict.** When both layers would deny, you'll see `AGT_DENY: ...` because AGT short-circuits. To see SpendGuard's reason in the deny case, AGT must allow first.
- **`claim_estimator` runs PER `evaluate()` call.** Each call is its own SpendGuard `request_decision` — so heavy AGT traffic translates 1:1 to sidecar UDS round trips (~1–3ms each on the same pod).
- **Multi-tenant.** Construct one `SpendGuardCompositeEvaluator` per tenant. AGT policies and SpendGuard `budget_id` then both reflect tenant-specific config without runtime branching.

---

## Quickest validation

The bundled demo exercises all three paths against a real sidecar + ledger + Postgres:

```bash
make demo-down -v
DEMO_MODE=agent_real_agt make demo-up
```

Expected output:

```
[demo] handshake ok session_id=...
[demo] (1) AGT-deny: allowed=False reason="AGT_DENY: Matched rule 'deny-dangerous'"
[demo] (2) AGT+SG allow: allowed=True reason='ALLOW (AGT + SpendGuard both PASS)'
[demo] (3) AGT-allow+SG-deny: allowed=False reason='SPENDGUARD_DENY: BUDGET_EXHAUSTED'
[demo] AGT composite all 3 paths PASS
```

Full demo source: [`deploy/demo/demo/run_demo.py::run_agt_composite_mode`](https://github.com/m24927605/agentic-spendguard/blob/main/deploy/demo/demo/run_demo.py) — copy-and-adapt for your own AGT rule set.

---

## Greenfield example (no existing AGT)

If you're starting from scratch and want to see both layers wired up:

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
            name="block-untrusted-tools", version="1.0",
            defaults=PolicyDefaults(action=PolicyAction.ALLOW),
            rules=[
                PolicyRule(
                    name="deny-dangerous",
                    condition=PolicyCondition(
                        field="tool_name",
                        operator=PolicyOperator.IN,
                        value=["shell", "delete_file"],
                    ),
                    action=PolicyAction.DENY,
                    priority=100,
                ),
            ],
        )
    ])

    # 2. SpendGuard client
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
        budget_id="44444444-4444-4444-8444-444444444444",
        window_instance_id="55555555-5555-4555-8555-555555555555",
        unit=common_pb2.UnitRef(
            unit_id="66666666-6666-4666-8666-666666666666",
            token_kind="output_token",
            model_family="gpt-4",
        ),
        pricing=common_pb2.PricingFreeze(pricing_version="demo-pricing-v1"),
        claim_estimator=lambda payload: [
            common_pb2.BudgetClaim(
                budget_id="44444444-4444-4444-8444-444444444444",
                window_instance_id="55555555-5555-4555-8555-555555555555",
                amount_atomic="500",
                unit=common_pb2.UnitRef(unit_id="66666666-6666-4666-8666-666666666666"),
            )
        ],
    )

    # 4. Evaluate
    result = await composite.evaluate({
        "tool_name": "web_search",
        "tool_args": {"q": "AI agent budget control"},
        "tenant_id": "00000000-0000-4000-8000-000000000001",
        "run_id": str(new_uuid7()),
    })
    print(result.allowed, result.reason)


asyncio.run(main())
```

---

## Related

- [Quickstart](../quickstart.md) — full stack up in 5 minutes
- [Contract DSL reference](../contracts/yaml.md) — author allow/stop rules
- Other integrations: [Pydantic-AI](pydantic-ai.md) · [LangChain & LangGraph](langchain.md) · [OpenAI Agents SDK](openai-agents.md) · [LiteLLM proxy](litellm.md)
