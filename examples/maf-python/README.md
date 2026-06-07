# `examples/maf-python`

End-to-end Python demo for the `spendguard.integrations.agent_framework`
adapter — the Microsoft Agent Framework (MAF) middleware integration
(the Python half of D07).

```bash
# laptop iteration, no sidecar required
python examples/maf-python/run.py --mock

# end-to-end against the sidecar + counting-stub
make demo-up DEMO_MODE=maf_python_real
```

## What it shows

Wires a `SpendGuardMiddleware` against a real `SpendGuardClient` (in
`--real`) or in-process mock (in `--mock`) and drives 3 chat-client
calls through MAF's `ChatMiddleware.process(context, call_next)`
boundary:

| Step | Outcome | Inner `call_next` fires? | Notes |
| ---- | ------- | ------------------------ | ----- |
| **1. ALLOW** | `CONTINUE` | YES | small message within budget. `client.request_decision(LLM_CALL_PRE)` returns `CONTINUE`; `call_next` runs the inner chat client; `client.emit_llm_call_post(SUCCESS)` commits the reservation. |
| **2. DENY** | `STOP` | NO | message tagged `trigger-deny`. In `--mock` the in-process stub client raises `DecisionDenied`; in `--real` the contract evaluator emits `SPENDGUARD_DENY` via `spendguard_estimate_override`. Middleware raises `DecisionDenied`; the inner counting-stub HTTP **never fires**. |
| **3. ALLOW2** | `CONTINUE` | YES | second small message — proves cross-call determinism. Replaces D04 / D06 / D08's STREAM step (streaming gating is v0.1.x non-goal — `design.md §3`). |

Success line on a clean run (LOCKED — CI grep depends on the exact
spelling, mirrors the `openai_agents_ts` / `inngest_agent_kit`
composite convention):

```
[demo] maf_python ALL 3 steps PASS (ALLOW + DENY + ALLOW2)
```

## Install

```bash
pip install 'spendguard-sdk[agent-framework]'
```

`agent-framework>=1.0,<2` is pulled in as an optional extra. The
`SpendGuardMiddleware` subclass of `agent_framework.ChatMiddleware`
is the integration's public surface.

## Wire shape

```python
import asyncio

from agent_framework import ChatAgent
from agent_framework.openai import OpenAIChatClient

from spendguard import SpendGuardClient
from spendguard.integrations.agent_framework import (
    SpendGuardAgentFrameworkOptions,
    SpendGuardMiddleware,
    run_context,
    RunContext,
)
from spendguard._proto.spendguard.common.v1 import common_pb2


async def main() -> None:
    client = SpendGuardClient(
        socket_path="/var/run/spendguard/adapter.sock",
        tenant_id="00000000-0000-4000-8000-000000000001",
        runtime_kind="microsoft-agent-framework-python",
    )
    await client.connect()
    await client.handshake()

    options = SpendGuardAgentFrameworkOptions(
        tenant_id="00000000-0000-4000-8000-000000000001",
        budget_id="44444444-4444-4444-8444-444444444444",
        window_instance_id="55555555-5555-4555-8555-555555555555",
        sidecar_socket_path="/var/run/spendguard/adapter.sock",
    )

    def _claim_estimator(messages):
        claim = common_pb2.BudgetClaim()
        claim.budget_id = options.budget_id
        claim.window_instance_id = options.window_instance_id
        claim.amount_atomic = "1000000"  # 1 USD micros placeholder
        claim.unit.unit_id = "usd_micros"
        return [claim]

    chat_middleware = SpendGuardMiddleware(
        client=client,
        options=options,
        unit=common_pb2.UnitRef(unit_id="usd_micros"),
        pricing=common_pb2.PricingFreeze(pricing_version="demo-pricing-v1"),
        claim_estimator=_claim_estimator,
    )

    agent = ChatAgent(
        chat_client=OpenAIChatClient(...),
        middleware=[chat_middleware],
    )

    async with run_context(RunContext(run_id="run-123")):
        result = await agent.run("Hello!")
        print(result)


if __name__ == "__main__":
    asyncio.run(main())
```

The sample's `run.py` substitutes the `OpenAIChatClient` with a
counting-stub-backed HTTP `call_next` closure so the demo runs
deterministically in the offline container; production wiring uses
the real `OpenAIChatClient` (or any `agent_framework` chat client).

## Anti-scope

- **No real provider key required.** `--real` mode talks to the
  demo's counting-stub via plain HTTP. Production wiring substitutes
  the real MAF chat client.
- **No `ChatAgent` orchestration in the script body.** The middleware
  is the load-bearing surface; agent orchestration is upstream of it.
  The wire-shape snippet above documents the `ChatAgent` registration
  for production callers.
- **Streaming-per-chunk gating is anti-scope for v0.1.x.** The MAF
  middleware contract observes the chat-client boundary; per-chunk
  gating is a follow-up.

## Known gap (cross-slice)

The `--real` demo currently surfaces the same D05 `UnitRef.unit_id
empty` substrate validation error D04 / D06 / D08 also surface
against the same sidecar. The Python adapter's middleware + wire
shape are independently verified by the pytest suite
(`sdk/python/tests/integrations/agent_framework/test_middleware.py`);
the `--mock` mode exercises the bracket end-to-end without going
through the substrate's `ReserveSet` validator. A future SDK-side
`unit_id` broadening lands the `--real` demo green here and across
the sibling D04 / D06 / D08 modes simultaneously.

## Related

- [`sdk/python/src/spendguard/integrations/agent_framework/`](../../sdk/python/src/spendguard/integrations/agent_framework/)
  — the Python adapter source.
- [`examples/maf-dotnet/`](../maf-dotnet/) — .NET sibling demo, same
  3-step matrix.
- [`docs/site-v2/src/content/docs/docs/integrations/microsoft-agent-framework.mdx`](../../docs/site-v2/src/content/docs/docs/integrations/microsoft-agent-framework.mdx)
  — the published integration page.
