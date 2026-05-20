---
description: >-
  The spend firewall for LLM agents — pre-call budget reservations,
  signed audit trail, <5ms. Works with LiteLLM, OpenAI Agents SDK,
  LangChain, LangGraph, Pydantic-AI, and Microsoft Agent Governance
  Toolkit (merged upstream).
---

# Agentic SpendGuard

**The spend firewall for LLM agents.**

Stops runaway agents *before* they hit your API bill — not after the
invoice arrives the next morning. Pre-call budget reservations, signed
audit trail, &lt;5ms per decision.

Works with **LiteLLM proxy**, **OpenAI Agents SDK**, **LangGraph**,
**LangChain**, **Pydantic-AI**, and **Microsoft Agent Governance
Toolkit** ([merged upstream](https://github.com/microsoft/agent-governance-toolkit/pull/2398)).

```bash
pip install 'spendguard-sdk[litellm]'
```

→ [90-second quickstart](quickstart.md) · [Microsoft AGT integration](https://github.com/microsoft/agent-governance-toolkit/blob/main/docs/integrations/spendguard-integration.md)

---

## Why this exists

Last Tuesday at 2:47am, a customer-support agent hit a rate-limited
tool. The retry policy kicked in. The agent re-planned, re-prompted,
re-tried — each retry a fresh `gpt-4o` call with the full conversation
in context. Forty minutes later, one stuck conversation had consumed
$380 in tokens. Multiply by the seventeen other tenants doing the
same during the incident.

The post-mortem starts with "we didn't know until the OpenAI dashboard
updated the next morning."

**SpendGuard moves detection from tomorrow to the 11th call.** Every
request reserves tokens against a per-tenant budget before it leaves
your process. Budget exhausted → HTTP 403 in &lt;5ms, with a signed
audit record of why.

---

## How it works

Three things happen on every LLM call:

1. **Reserve.** Before the API call, SpendGuard checks the per-tenant
   budget ledger and reserves the worst-case spend. If the tenant
   can't afford the call, the provider is never hit.
2. **Commit.** After the response, SpendGuard reads `response.usage`
   and commits the real amount. Overshoot is refunded.
3. **Audit.** Every reserve / commit / reject lands as a signed
   CloudEvent. When finance asks *"what did tenant X spend on
   Tuesday?"*, it's a query.

```
agent → SpendGuard (reserve) → provider → SpendGuard (commit) → ledger
```

If you've integrated Stripe: this is auth/capture, applied to LLM
tokens. Idempotent, atomic, fail-closed.

---

## Try it

```bash
# Install the SDK
pip install 'spendguard-sdk[litellm]'

# Or run the full demo (~5 min cold start):
git clone https://github.com/m24927605/agentic-spendguard
cd agentic-spendguard
make demo-up DEMO_MODE=litellm_real
```

Expected output:

```
[demo] (1) ALLOW: HTTP 200 completion_tokens=7
[demo] (2) DENY: HTTP 403 reasons=['BUDGET_EXHAUSTED', ...]
[demo] (3) STREAM: HTTP 200
[demo] (4) MULTI-TEAM: 2 isolated calls
[demo] litellm_real ALL 4 steps PASS
```

---

## Honest about where we are

- **Dev Status 4-Beta.** Single-maintainer open source. Solid demo
  coverage (8+ demo modes, all green) but zero production users yet.
- **What's production-ready vs not:** see [POC vs GA gates](poc-vs-ga.md).
- **What ships and what doesn't:** see the [roadmap](roadmap/ga-hardening-slices.md).

Use alongside Langfuse / Helicone / LangSmith — SpendGuard *prevents*,
they *observe*. Different category, not a competitor.

---

## Next

- [Quickstart](quickstart.md) — install + run the demo in 5 minutes
- [Architecture](concepts/architecture.md) — the 6-layer design (only if you care how it works internally)
- [Adapter integrations](integrations/litellm.md) — wire it into your stack
- [POC vs GA gates](poc-vs-ga.md) — what's production-ready vs not
