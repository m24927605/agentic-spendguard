---
description: >-
  The reservation pattern for LLM spend. Stripe-style auth/capture ledger that
  turns runtime budget enforcement into a primitive. How the pre-call gate,
  estimated reservations, and post-call capture/release stages compose into a
  cost-correct audit trail.
---

# The reservation pattern for LLM budgets

> The auth/capture pattern is forty years old in payments and a few
> years old in feature flags. It's not yet a primitive for LLM spend,
> but it should be — the same shape solves the same class of problem.
> This page explains the pattern from first principles and shows how
> Agentic SpendGuard implements it.

## Why the standard answer doesn't work

The naive design for "control LLM spend" is a counter:

```
budget_remaining = 1000.00
on each LLM call:
    budget_remaining -= actual_cost   # arrives after the call
    if budget_remaining < 0: alert
```

This fails because **the actual cost arrives after the call**. The
provider returns a usage record that tells you how many tokens it
charged for. By the time you decrement the counter, you've already
spent the money. The counter is a record-keeping device, not a gate.

To gate, you need to know the cost *before* the call. But LLM cost
isn't fixed — output tokens depend on what the model generates, which
depends on... the call you haven't made yet.

This is the problem auth/capture solves in payments. The Visa network
doesn't know the final amount of a hotel charge when you check in —
the hotel could add bar tabs, room service, damages. So the hotel
**authorizes** an estimated amount (the auth-hold), which reserves
funds without charging. At checkout, the hotel **captures** the actual
amount, releasing the unused portion back.

## The pattern that does

The reservation pattern, mapped to LLM budgets:

```
Phase 1 — Estimate
    Given:  the messages, the model, the pricing table.
    Output: a projected claim (e.g., "this call will cost ~$0.04").

Phase 2 — Auth (Reserve)
    Sidecar checks projected_claim against budget.
    Budget can cover? → record a reservation entry, return RESERVED.
    Budget can't cover? → return STOP, the LLM call must not happen.

Phase 3 — Upstream LLM call
    Application makes the actual provider call.
    Provider returns actual_cost in the usage record.

Phase 4 — Capture (Commit)
    Sidecar receives actual_cost.
    Ledger: reservation → commit, freeing unused portion.

Phase 5 — Release on failure
    If Phase 3 throws / times out / crashes:
        Application calls sidecar.release(decision_id).
        Reservation rolls back, budget is restored.
    Otherwise, a TTL background sweeper auto-releases stale
    reservations after a configurable timeout.
```

Properties that emerge:

- **Pre-call refusal is mechanical.** The over-budget path raises
  before the LLM call. There's no soft-warning branch.
- **Auth-stage estimates can be conservative.** A model that returns
  fewer tokens than estimated has the unused portion freed at capture
  time. The budget reservation is never permanently locked beyond
  actual usage.
- **Idempotency is structural.** Re-running the same decision (same
  decision_id, same idempotency_key) collapses onto the existing
  reservation. Retries don't double-charge.
- **Crash-safe.** A pod that dies after auth but before capture loses
  the in-memory state, not the durable record. The ledger entry is
  still there; the TTL sweeper releases it eventually.

## Show me the code

The pattern surfaces in the SDK as one decision call per LLM call:

```python
# Phase 1+2: estimate + auth
outcome = await sg.request_decision(
    trigger="LLM_CALL_PRE",
    run_id=run_id, decision_id=decision_id,
    route="llm.call",
    projected_claims=[estimated_claim],
    idempotency_key=derive_key(...),
)

# Phase 3: upstream LLM call
try:
    response = await openai.chat.completions.create(...)
except Exception:
    # Phase 5: release on failure
    await sg.release(decision_id)
    raise

# Phase 4: capture
await sg.commit(
    decision_id=decision_id,
    actual_claims=[claim_from(response.usage)],
)
```

The framework adapters bundle this into a single `Model.request()`
override so application code is one wrap-the-model line, not a
five-step protocol per call.

## What this is not

- **It is not a billing system.** SpendGuard doesn't generate invoices
  or settle with providers. It gates calls; the provider still bills
  you for the calls you make.
- **It is not a usage analytics dashboard.** It records every
  reservation and capture in an audit chain, but turning that into BI
  charts is a separate concern.
- **It is not free.** Each decision is a UDS gRPC round-trip
  (sub-5ms p99 in the POC). For agents that make tens of LLM calls
  per second, this is negligible. For higher-frequency systems,
  measure first.

## Read more

- [6-layer architecture](../concepts/architecture.md) — where the
  reservation pattern fits in the larger SpendGuard runtime
- [Decision lifecycle](../concepts/decision-lifecycle.md) — auth →
  capture → release state machine in detail
- [Ledger storage spec](../reference/ledger-schema.md) — the
  Postgres schema that implements the audit chain
- [Pre-call budget caps](pre-call-budget-cap.md) — the practical
  use-case framing of this pattern
