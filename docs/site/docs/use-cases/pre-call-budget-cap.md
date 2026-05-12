---
description: >-
  Set hard token budget caps on LLM API requests that are enforced before the
  spend happens, not after. Stripe-style auth/capture reservations for OpenAI,
  Anthropic, and any provider — over-budget calls are refused at the boundary.
---

# Pre-call budget caps for LLM API requests

> You want a cap that says "this agent can spend at most $X on gpt-4o per
> hour, and any call that would push it past the line must be refused
> *before* the request goes to the provider." Token-usage dashboards and
> daily alerts give you the post-hoc number, not the gate. Here's the
> pattern that gives you the gate.

## Why the standard answer doesn't work

Most LLM cost tooling is **reconciliation**, not **control**:

| Approach | What it does | When you find out |
|---|---|---|
| Provider invoice / billing API | Tells you what you spent | End of billing cycle |
| Usage dashboard | Aggregates token counts | Hours later, after the spend |
| Rate limit on the provider key | Caps requests per second/minute | Not by dollar — by count |
| Soft alert ("you're at 80%") | Pings a webhook | After the budget is mostly gone |

None of these *prevent* the call. They tell you the bill, hopefully before
the next bill. When an agent is in a retry loop or a tool-use loop, the
gap between "spend the money" and "see the dashboard" is exactly when
real damage happens.

## The pattern that does

A budget reservation sits in front of every LLM call. The reservation
acts like a Stripe auth/capture:

```
agent → SDK wrapper
          │
          ▼
       sidecar.request_decision(budget_id, projected_claim)
          │
          ├── budget would be exceeded ───► STOP   (raise, no LLM call)
          │
          ├── budget can cover it     ───► RESERVE (auth) ──┐
          │                                                  │
          │                                                  ▼
          │                                          your LLM call goes out
          │                                                  │
          ├── provider response ──────► sidecar.commit (capture actual)
          │                              or sidecar.release (cancel auth)
          │
          └── crash / timeout         ─► reservation auto-releases on TTL
```

Three properties that make this work:

1. **Pre-call refusal is mechanical.** The over-budget path is a thrown
   exception, not a soft warning. Application code can't accidentally
   ignore it.
2. **Reservations are accounted, not estimates.** The ledger tracks
   reservations (auth-stage) and commits (capture-stage) separately,
   so an estimated 1,500 tokens reserved but actually 800 used releases
   700 back to the budget.
3. **Idempotent on retry.** A retried call with identical inputs
   collapses onto the original reservation instead of allocating a new
   one. Otherwise a 47-retry loop would burn 47x the reservation.

## Show me the code

The reservation is one call. The SpendGuard SDK handles the
auth/commit/release lifecycle:

```python
from spendguard import SpendGuardClient, DecisionStopped

async with SpendGuardClient(socket_path="/var/run/spendguard/adapter.sock",
                            tenant_id=tenant_id) as sg:
    await sg.handshake()
    try:
        outcome = await sg.request_decision(
            trigger="LLM_CALL_PRE",
            run_id=run_id, decision_id=decision_id,
            route="llm.call",
            projected_claims=[claim],          # estimated USD or tokens
            idempotency_key=derive_key(...),   # stable across retries
        )
        # Reservation made. Make the LLM call now.
    except DecisionStopped as e:
        # Over budget. The LLM call must not happen.
        raise
```

The framework adapters (Pydantic-AI / LangChain / OpenAI Agents / AGT)
wrap this in a single `Model.request()` override so application code
doesn't change.

## Read more

- [Pydantic-AI integration](../integrations/pydantic-ai.md) — drop-in
  `Model` wrapper that handles the auth/capture lifecycle
- [Reservation pattern deep-dive](reservation-pattern.md) — the
  architectural reasoning behind auth/capture for LLM spend
- [Stop a runaway agent](agent-runaway-protection.md) — the failure
  mode this pattern is specifically built to prevent
- [Contract DSL reference](../contracts/yaml.md) — author the rules
  that decide allow vs stop vs require-approval per call
