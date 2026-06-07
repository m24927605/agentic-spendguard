# DEMO_MODE=agent_real_autogen

COV_D24 SLICE 5 — AutoGen 0.4+ / AG2 Python adapter end-to-end demo.

## What this demo proves

`spendguard.integrations.autogen.SpendGuardChatCompletionClient`
subclasses `autogen_core.models.ChatCompletionClient` (the LLM
abstraction shared by AutoGen 0.4+ and AG2 — AG2 vendored the
namespace unchanged) and wraps an `OpenAIChatCompletionClient` so
every `AssistantAgent.on_messages(...)` call routes through the
SpendGuard sidecar's
`RequestDecision(LLM_CALL_PRE)` → `EmitLlmCallPost(SUCCESS)` lifecycle.

* **ALLOW path** — the wrapper reserves budget BEFORE AutoGen
  dispatches the model HTTP; the `AssistantAgent` reaches the model;
  the wrapper commits with real `CreateResult.usage.prompt_tokens +
  completion_tokens`.
* **DENY path** — `DecisionDenied` propagates directly out of
  `wrapper.create(...)` (no DEVIATION-1-style wrap needed —
  `ChatCompletionClient` has no framework-side catch on the create()
  path); the model is never called.
* **LINEAGE coverage** — the same wrapper instance works against
  AutoGen `AssistantAgent` (Microsoft lineage, `autogen-agentchat`)
  AND AG2 `AssistantAgent` (`ag2`) with zero code changes. The
  `LINEAGE` constant tells operators which lineage is loaded
  (`autogen` / `ag2` / `both` / `core-only`) but is telemetry-only —
  business logic never branches on it.

Both paths land in the ledger DB (`reserve` + `commit_estimated` or
`denied_decision` rows) and the canonical event stream
(`spendguard.audit.decision` + `spendguard.audit.outcome`).

## How to run

```bash
# ALLOW (mock OpenAI via counting-stub, no API key needed):
make demo-up DEMO_MODE=agent_real_autogen
```

## Driver

`deploy/demo/demo/run_demo.py` with
`SPENDGUARD_DEMO_MODE=agent_real_autogen` constructs:

```python
from autogen_agentchat.agents import AssistantAgent
from autogen_ext.models.openai import OpenAIChatCompletionClient

from spendguard.integrations.autogen import (
    SpendGuardChatCompletionClient,
    RunContext, run_context,
)

guarded = SpendGuardChatCompletionClient(
    inner=OpenAIChatCompletionClient(model="gpt-4o-mini"),
    client=client,
    budget_id="44444444-4444-4444-8444-444444444444",
    window_instance_id="55555555-5555-4555-8555-555555555555",
    unit=common_pb2.UnitRef(
        unit_id="...",
        token_kind="output_token",
        model_family="gpt-4"),
    pricing=common_pb2.PricingFreeze(pricing_version="demo-pricing-v1", ...),
    claim_estimator=lambda messages: [common_pb2.BudgetClaim(...)],
)

agent = AssistantAgent(name="x", model_client=guarded)

async with run_context(RunContext(run_id="my-run-1")):
    result = await agent.on_messages([...], cancellation_token)
```

A single `SpendGuardChatCompletionClient` instance covers every
provider backend (OpenAI / Anthropic / Azure / LiteLLM / custom)
because gating sits at the `ChatCompletionClient` ABC boundary, not
the vendor SDK boundary. The decision_context tags the live backend
(e.g. `inner_client=OpenAIChatCompletionClient`, `lineage=autogen`)
so the audit chain records which client produced each row.

For AG2 callers, the only line that changes is the `AssistantAgent`
import path:

```python
from ag2.agents import AssistantAgent  # AG2 lineage
```

Everything else — including the `SpendGuardChatCompletionClient`
construction — is identical.

## Services

* `counting-stub` — mock OpenAI provider on `/v1/chat/completions`
  with a canned 32-token response so the driver can exercise the
  multi-lineage coverage proof without real cloud credentials.
* `autogen-runner` — Python 3.12 + `autogen-core>=0.4,<1.0` +
  `autogen-agentchat>=0.4` + `autogen-ext[openai]` +
  `spendguard-sdk[autogen]`, drives `run_demo.py` against the sidecar
  UDS.

## Verification

`make demo-up DEMO_MODE=agent_real_autogen` runs
`verify_step_agent_real_autogen.sql` against `spendguard_ledger`
after the runner exits, asserting:

* `>= 1` `reserve` row (ALLOW path)
* `>= 1` `commit_estimated` row (ALLOW path commit)
* INV-2 strict order: earliest `reserve` predates earliest
  `commit_estimated`
* Cross-DB: `>= 1` `spendguard.audit.decision` + `>= 1`
  `spendguard.audit.outcome` row in `canonical_events`
* `decision_context_json->>'integration' = 'autogen'` on the decision rows

## D05 UnitRef gap (cross-slice tracking)

The cross-slice D05 UnitRef wiring gap surfaces here only at the
outbox-closure step (canonical-events backfill of UnitRef metadata).
The Makefile target tolerates the closure failure with a printed
deferral note — same precedent as D04 / D06 / D07 / D08 / D19 / D20 /
D21 / D22 / D23 / D29.

## Out of scope (D24 non-goals)

* Per-chunk gating inside `create_stream()`. Stream gating brackets
  the WHOLE stream at the model boundary; intra-stream tool calls
  inherit the parent reservation. Per-chunk gating is tracked as
  follow-on parity with the OpenAI Agents POC.
* Wrapping `count_tokens()` / `total_usage()` / `remaining_tokens()`
  introspection methods — pass-through to the inner client (these
  carry no side effects to avoid confusing `AssistantAgent`'s
  token-budget caps).
* AG2-specific extensions (e.g. AG2's `register_for_llm` decorator)
  — orthogonal to the LLM gate.
* Microsoft AGT integration (D7, already shipped). AGT is a separate
  framework, not AutoGen.

## References

* Spec: `docs/specs/coverage/D24_autogen_ag2/`
* Module: `sdk/python/src/spendguard/integrations/autogen/`
* Docs page: `docs/site-v2/src/content/docs/docs/integrations/autogen.mdx`
* Test suite: `sdk/python/tests/integrations/autogen/test_autogen.py`
