# DEMO_MODE=agent_real_adk

COV_D19 SLICE 5 — Google ADK Python adapter end-to-end demo.

## What this demo proves

`spendguard.integrations.adk.SpendGuardAdkCallback` wraps a real
`google.adk.agents.LlmAgent` and routes every model turn through the
SpendGuard sidecar's `RequestDecision(LLM_CALL_PRE)` →
`EmitLlmCallPost(SUCCESS)` lifecycle:

* **ALLOW path** — `before_model_callback` reserves budget; the agent
  reaches Gemini; `after_model_callback` commits with real
  `usage_metadata.total_token_count`.
* **DENY path** — sidecar returns DENY; the callback returns a
  synthetic `LlmResponse(error_code="SPENDGUARD_DENY")` and ADK
  short-circuits the turn. The model is **never** called.

Both paths land in the ledger DB (`reserve` + `commit_estimated` or
`denied_decision` rows) and the canonical event stream
(`spendguard.audit.decision` + `spendguard.audit.outcome`).

## How to run

```bash
# With a real GOOGLE_API_KEY (hits real Gemini):
GOOGLE_API_KEY=... make demo-up DEMO_MODE=agent_real_adk

# Without a key (stub variant for CI):
make demo-up DEMO_MODE=agent_real_adk_stub
```

## Driver

`deploy/demo/demo/run_demo.py` with `SPENDGUARD_DEMO_MODE=agent_real_adk`
constructs:

```python
from google.adk.agents import LlmAgent
from google.adk.runners import InMemoryRunner
from spendguard.integrations.adk import SpendGuardAdkCallback

cb = SpendGuardAdkCallback(
    client=client,
    budget_id="44444444-4444-4444-8444-444444444444",
    window_instance_id="55555555-5555-4555-8555-555555555555",
    unit=common_pb2.UnitRef(unit_id="...", token_kind="output_token", model_family="gemini-2.0-flash"),
    pricing=common_pb2.PricingFreeze(pricing_version="demo-pricing-v1", ...),
    claim_estimator=lambda req: [common_pb2.BudgetClaim(...)],
)

agent = LlmAgent(
    name="spendguard-demo-adk-agent",
    model="gemini-2.0-flash",
    instructions="You are a budget-aware assistant.",
    before_model_callback=cb,
    after_model_callback=cb,
)
runner = InMemoryRunner(agent=agent)
async for _ in runner.run_async(...):
    pass
```

The same callback instance registers to **both** slots — dispatch is
by `isinstance(payload, LlmRequest)` vs `LlmResponse`. State handoff
between PRE and POST uses `callback_context.state` (ADK's documented
per-invocation dict).

## Services

* `counting-stub` — mock provider that accepts both OpenAI-shape
  (`/v1/chat/completions`, LiteLlm path) and Gemini-shape
  (`...generateContent`) requests. Used in `agent_real_adk_stub` mode;
  the real `agent_real_adk` mode hits live Gemini via the `google-adk`
  default transport.
* `adk-runner` — Python 3.12 + `google-adk` + `spendguard-sdk[adk]`,
  drives `run_demo.py` against the sidecar UDS.

## Verification

`make demo-up DEMO_MODE=agent_real_adk` runs
`verify_step_agent_real_adk.sql` against `spendguard_ledger` after the
runner exits, asserting:

* `>= 1` `reserve` row (ALLOW path)
* `>= 1` `commit_estimated` row (ALLOW path POST commit)
* INV-2 strict order: earliest `reserve` predates earliest
  `commit_estimated`
* Cross-DB: `>= 1` `spendguard.audit.decision` + `>= 1`
  `spendguard.audit.outcome` row in `canonical_events`

## D05 UnitRef gap (cross-slice tracking)

The cross-slice D05 UnitRef wiring gap surfaces here only at the
outbox-closure step (canonical-events backfill of UnitRef metadata).
The Makefile target tolerates the closure failure with a printed
deferral note — same precedent as D04 / D06 / D07 / D08 / D29.

## Out of scope (D19 non-goals)

* TS / Go / Java / Kotlin ADK ports (D19.5 covers the TS port).
* Streaming intra-turn gating (`run_live`).
* `before_tool_callback` / `after_tool_callback` wiring.
* Replacing the egress proxy path — D19 is additive.

## References

* Spec: `docs/specs/coverage/D19_google_adk/`
* Module: `sdk/python/src/spendguard/integrations/adk/`
* Docs page: `docs/site-v2/src/content/docs/docs/integrations/adk.md`
