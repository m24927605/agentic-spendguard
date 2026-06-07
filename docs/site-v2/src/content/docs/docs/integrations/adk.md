---
title: "Google ADK budget control with SpendGuard"
description: >-
  Pre-call token budget enforcement for Google Agent Development Kit
  (google-adk) agents using Agentic SpendGuard. One callback covers both
  before_model_callback and after_model_callback slots, works with
  Gemini direct, Vertex-backed Gemini, and LiteLlm-wrapped OpenAI /
  Anthropic.
---


> Your Google ADK `LlmAgent` just hit a tool-call loop on a tricky
> reasoning chain. Each `before_model_callback` turn ships another
> request to Gemini. Without a gate, you find out the cost on next
> month's billing dashboard. SpendGuard plugs a single
> `SpendGuardAdkCallback` into both `before_model_callback` and
> `after_model_callback` so every model turn reserves against a budget
> *before* the upstream call goes out — and the same callback works for
> Vertex Gemini or LiteLlm-wrapped OpenAI / Anthropic.

## Why you'd want this

- **One callback, two slots.** `SpendGuardAdkCallback` is a single
  instance you register to *both* `before_model_callback` and
  `after_model_callback`. Dispatch is by payload type — `LlmRequest`
  routes to PRE, `LlmResponse` routes to POST.
- **Multi-vendor by shape, not by string.** Works against
  `LlmAgent(model="gemini-2.0-flash")`, Vertex-backed Gemini, and
  `LlmAgent(model=LiteLlm("openai/gpt-4o-mini"))` because usage
  extraction reads `usage_metadata` field shape, not a model-string
  match.
- **Pre-call refusal, not post-hoc accounting.** Over-budget calls
  return a synthetic `LlmResponse(error_code="SPENDGUARD_DENY")` so
  ADK short-circuits the turn — the Gemini API is never touched.
- **Audit + approval pipeline shared with every other framework.** The
  callback writes to the same SpendGuard ledger as the LangChain,
  Pydantic-AI, and OpenAI Agents integrations, so a multi-framework
  agent fleet gets a single decision log.

## Setup (60 seconds)

```bash
pip install 'spendguard-sdk[adk]'
```

Bring up a sidecar via the demo stack:

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard && make demo-up
```

## Wire it up

```python
import asyncio

from google.adk.agents import LlmAgent
from google.adk.runners import InMemoryRunner

from spendguard import SpendGuardClient
from spendguard.integrations.adk import SpendGuardAdkCallback
from spendguard._proto.spendguard.common.v1 import common_pb2


async def main() -> None:
    client = SpendGuardClient(
        socket_path="/var/run/spendguard/adapter.sock",
        tenant_id="00000000-0000-4000-8000-000000000001",
    )
    await client.connect()
    await client.handshake()

    cb = SpendGuardAdkCallback(
        client=client,
        budget_id="my-budget",
        window_instance_id="my-window",
        unit=common_pb2.UnitRef(
            unit_id="usd_micros",
            token_kind="output_token",
            model_family="gemini-2.0-flash",
        ),
        pricing=common_pb2.PricingFreeze(pricing_version="2025-q4"),
    )

    agent = LlmAgent(
        name="budget-aware-agent",
        model="gemini-2.0-flash",
        instructions="You are a budget-aware assistant.",
        # Same `cb` instance plugged into BOTH slots:
        before_model_callback=cb,
        after_model_callback=cb,
    )

    runner = InMemoryRunner(agent=agent)
    async for event in runner.run_async(
        session_id=client.session_id,
        user_id="alice",
        new_message="Say hello in three words.",
    ):
        print(event)


asyncio.run(main())
```

## What you get

- **Pre-call budget reservation** on every `LlmAgent` model turn,
  including tool-loop iterations.
- **Multi-vendor coverage.** Gemini direct, Vertex Gemini, LiteLlm
  wrappers — all extract usage by `usage_metadata` shape.
- **Concurrent-safe.** ADK constructs a fresh `CallbackContext` per
  `Runner.run_async` invocation, so concurrent runs are inherently
  isolated through `callback_context.state`.
- **No raise on DENY.** The callback returns the documented
  `LlmResponse(error_code="SPENDGUARD_DENY", ...)` short-circuit
  channel so the user's own `after_model_callback` chain (if any)
  still sees the deny.

## Common patterns

### Custom run_id for cross-framework correlation

```python
cb = SpendGuardAdkCallback(
    client=client,
    budget_id="...",
    window_instance_id="...",
    unit=common_pb2.UnitRef(...),
    pricing=common_pb2.PricingFreeze(...),
    run_id_fn=lambda ctx: my_parent_trace_id_for(ctx),
)
```

Default is `ctx.invocation_id` (ADK assigns one UUID per
`Runner.run_async`). Override when you want the run_id to correlate
with a LangChain or OpenAI Agents parent trace.

### Custom claim estimator

```python
def my_estimator(req):
    # Inspect req.contents for image parts, sum tokens for text parts,
    # surcharge image parts at a per-image rate.
    ...
    return [common_pb2.BudgetClaim(...)]

cb = SpendGuardAdkCallback(
    client=client, budget_id="...", window_instance_id="...",
    unit=..., pricing=..., claim_estimator=my_estimator,
)
```

When omitted, the callback dispatches the default estimator off
`req.model` (Gemini family / OpenAI via LiteLlm prefix strip / chars/4
fallback for unknown models with a one-shot warning).

### DENY behavior

When `request_decision` returns DENY, the callback:

1. Sets `ctx.state["spendguard.denied"] = True`.
2. Returns a synthetic `LlmResponse` with
   `error_code="SPENDGUARD_DENY"` and `error_message` containing the
   comma-joined reason codes (defaults to `BUDGET_EXHAUSTED`).
3. ADK terminates the turn — the inner Gemini transport is never hit.
4. POST is a no-op (no commit, no release — the deny carries no
   reservation).

You can chain your own `after_model_callback` *after* SpendGuard's
to log the deny without losing the short-circuit semantics.

### Tool callbacks

Tool callbacks (`before_tool_callback` / `after_tool_callback`) are
out of scope for v0.1.x. Spend gating sits at the model boundary;
tool calls don't drive spend directly. Tool-budget enforcement is
tracked as a future enhancement.

## Related

- [Quickstart](../quickstart.md) — full stack up in 5 minutes
- [Contract DSL reference](../contracts/yaml.md) — author allow/stop rules
- Other integrations: [Pydantic-AI](pydantic-ai.md) · [LangChain & LangGraph](langchain.md) · [OpenAI Agents SDK](openai-agents.md) · [Microsoft AGT](agt.md)
