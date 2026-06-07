# DEMO_MODE=agent_real_strands

COV_D20 SLICE 5 — AWS Strands Agents Python adapter end-to-end demo.

## What this demo proves

`spendguard.integrations.strands.SpendGuardStrandsHookProvider` plugs
into a real `strands.Agent` via `hooks=[provider]` and routes every
invocation through the SpendGuard sidecar's
`RequestDecision(LLM_CALL_PRE)` → `EmitLlmCallPost(SUCCESS)` lifecycle.

* **ALLOW path** — `before_invocation` reserves budget BEFORE the
  upstream provider HTTP fires; the agent reaches the model;
  `after_invocation` commits with real `result.usage` tokens.
* The same provider gates every Strands `Model` backend
  (Bedrock / OpenAI / Anthropic / Gemini / Ollama / LiteLLM) with one
  registration — usage extraction reads `result.usage` field shape,
  not a model-string match.

Both paths land in the ledger DB (`reserve` + `commit_estimated` or
`denied_decision` rows) and the canonical event stream
(`spendguard.audit.decision` + `spendguard.audit.outcome`).

## How to run

```bash
# ALLOW (mock Bedrock + mock OpenAI via counting-stub, no API key needed):
make demo-up DEMO_MODE=agent_real_strands

# DENY (zero provider HTTP — see agent_real_strands_deny/):
make demo-up DEMO_MODE=agent_real_strands_deny
```

## Driver

`deploy/demo/demo/run_demo.py` with
`SPENDGUARD_DEMO_MODE=agent_real_strands` constructs:

```python
from strands import Agent
from strands.models.bedrock import BedrockModel

from spendguard.integrations.strands import SpendGuardStrandsHookProvider

def reconcile(invocation, result):
    usage = result.usage
    total = (
        getattr(usage, "total_tokens", None)
        or ((getattr(usage, "input_tokens", 0) or 0)
            + (getattr(usage, "output_tokens", 0) or 0))
    )
    return [common_pb2.BudgetClaim(
        budget_id="...", unit=unit, amount_atomic=str(total),
        direction=common_pb2.BudgetClaim.DEBIT,
        window_instance_id="...")]

guard = SpendGuardStrandsHookProvider(
    client=client,
    budget_id="44444444-4444-4444-8444-444444444444",
    window_instance_id="55555555-5555-4555-8555-555555555555",
    unit=common_pb2.UnitRef(
        unit_id="...",
        token_kind="output_token",
        model_family="anthropic.claude-3-5-sonnet"),
    pricing=common_pb2.PricingFreeze(pricing_version="demo-pricing-v1", ...),
    claim_estimator=estimate_claims,
    claim_reconciler=reconcile,
)

agent = Agent(
    model=BedrockModel(model_id="anthropic.claude-3-5-sonnet-20241022-v2:0"),
    hooks=[guard],
)
result = await agent.invoke_async(prompt="Hello")
```

A single `SpendGuardStrandsHookProvider` instance covers every model
backend Strands supports — Bedrock / OpenAI / Anthropic / Gemini /
Ollama / LiteLLM — because gating is at the agent-runtime boundary,
not the model boundary. The decision_context tags the live backend so
the audit chain records which model produced each row.

## Services

* `counting-stub` — mock provider that accepts both OpenAI-shape
  (`/v1/chat/completions`) and Bedrock-shape
  (`/model/.../invoke`) requests so the driver can exercise multiple
  backends without real cloud credentials.
* `strands-runner` — Python 3.12 + `strands-agents` +
  `spendguard-sdk[strands]`, drives `run_demo.py` against the sidecar
  UDS.

## Verification

`make demo-up DEMO_MODE=agent_real_strands` runs
`verify_step_agent_real_strands.sql` against `spendguard_ledger` after
the runner exits, asserting:

* `>= 1` `reserve` row (ALLOW path)
* `>= 1` `commit_estimated` row (ALLOW path commit)
* INV-2 strict order: earliest `reserve` predates earliest
  `commit_estimated`
* Cross-DB: `>= 1` `spendguard.audit.decision` + `>= 1`
  `spendguard.audit.outcome` row in `canonical_events`
* `decision_context_json->>'integration' = 'strands'` on the decision
  rows
* `decision_context_json->>'model_backend' IN ('BedrockModel',
  'OpenAIModel', 'LiteLLMModel')` — the multi-backend coverage proof

## D05 UnitRef gap (cross-slice tracking)

The cross-slice D05 UnitRef wiring gap surfaces here only at the
outbox-closure step (canonical-events backfill of UnitRef metadata).
The Makefile target tolerates the closure failure with a printed
deferral note — same precedent as D04 / D06 / D07 / D08 / D19 / D29.

## Out of scope (D20 non-goals)

* `before_tool` / `after_tool` per-tool budget gating (D20.1).
* `on_message` streaming token gating; commit only at
  `after_invocation`.
* Strands' TS SDK (D05 / D08 family).
* Pinning beyond `strands-agents>=1.0,<2`.

## References

* Spec: `docs/specs/coverage/D20_aws_strands/`
* Module: `sdk/python/src/spendguard/integrations/strands/`
* Docs page: `docs/site-v2/src/content/docs/docs/integrations/aws-strands.md`
