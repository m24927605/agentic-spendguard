# DEMO_MODE=agent_real_dspy

COV_D21 SLICE 3 — DSPy Python adapter end-to-end demo.

## What this demo proves

`spendguard.integrations.dspy.SpendGuardDSPyCallback` plugs into a real
`dspy.LM` via `dspy.configure(callbacks=[callback])` and routes every
LM call through the SpendGuard sidecar's
`RequestDecision(LLM_CALL_PRE)` → `EmitLlmCallPost(SUCCESS)` lifecycle.

* **ALLOW path** — `on_lm_start` reserves budget BEFORE the upstream
  provider HTTP fires; the LM reaches the model; `on_lm_end` commits
  with real `outputs[0].usage` tokens.
* **DENY path** — sidecar returns DENY → `DecisionDenied` raised → the
  counting-stub records ZERO new hits (INV-1 zero-provider-HTTP-on-DENY
  proof).
* **CUSTOM-LM path** — a custom `dspy.LM` subclass demonstrates that
  D21 covers direct-path callers that bypass the D12 LiteLLM shim.
* **D12 coexistence** — the shared `_SHIM_IN_FLIGHT` contextvar blocks
  D12 (when installed) from double-reserving the same call.

All paths land in the ledger DB (`reserve` + `commit_estimated` or
`denied_decision` rows) and the canonical event stream
(`spendguard.audit.decision` + `spendguard.audit.outcome`).

## How to run

```bash
# ALLOW + DENY + CUSTOM-LM (mock OpenAI via counting-stub, no API key needed):
make demo-up DEMO_MODE=agent_real_dspy
```

## Driver

`deploy/demo/demo/run_demo.py` with `SPENDGUARD_DEMO_MODE=agent_real_dspy`
constructs:

```python
import dspy
from spendguard.integrations.dspy import (
    SpendGuardDSPyCallback, BudgetBinding,
)

def resolve(model_str):
    return BudgetBinding(
        budget_id="...", window_instance_id="...",
        unit=unit, pricing=pricing,
    )

def reconcile(outputs):
    first = outputs[0] if outputs else None
    usage = getattr(first, "usage", {}) or {}
    total = usage.get("total_tokens", 100)
    return [common_pb2.BudgetClaim(
        budget_id="...", unit=unit, amount_atomic=str(total),
        direction=common_pb2.BudgetClaim.DEBIT,
        window_instance_id="...")]

callback = SpendGuardDSPyCallback(
    client=client,
    budget_resolver=resolve,
    claim_reconciler=reconcile,
)
dspy.configure(
    lm=dspy.LM("openai/gpt-4o-mini"),
    callbacks=[callback],  # MUST be FIRST in list
)

qa = dspy.ChainOfThought("question -> answer")
result = qa(question="What is 2+2?")
```

The callback is wired FIRST in the callbacks list so reserve precedes
any user observer callback.

## Services

* `counting-stub` — mock OpenAI-compatible provider on
  `/v1/chat/completions`. The runner's `OPENAI_BASE_URL` env points
  DSPy/LiteLLM at this stub so no real API key is needed.
* `dspy-runner` — Python 3.12 + `dspy-ai` +
  `spendguard-sdk[dspy]`, drives `run_demo.py` against the sidecar UDS.

## Verification

`make demo-up DEMO_MODE=agent_real_dspy` runs
`verify_step_agent_real_dspy.sql` against `spendguard_ledger` after the
runner exits, asserting:

* `>= 1` `reserve` row (ALLOW path)
* `>= 1` `commit_estimated` row (ALLOW path commit)
* INV-2 strict order: earliest `reserve` predates earliest
  `commit_estimated`
* Cross-DB: `>= 1` `spendguard.audit.decision` + `>= 1`
  `spendguard.audit.outcome` row in `canonical_events`
* `decision_context_json->>'integration' = 'dspy'` on the decision rows

## D05 UnitRef gap (cross-slice tracking)

The cross-slice D05 UnitRef wiring gap surfaces here only at the
outbox-closure step (canonical-events backfill of UnitRef metadata).
The Makefile target tolerates the closure failure with a printed
deferral note — same precedent as D04 / D06 / D07 / D08 / D19 / D20.

## Out of scope (D21 non-goals)

* Token-by-token streaming gating (commit only at `on_lm_end`).
* `on_tool_start` / `on_tool_end` per-tool budget gating (D21.1).
* `on_module_start` / `on_module_end` callbacks (subsumed by LM-boundary
  gating).
* Async DSPy callbacks (DSPy >= 2.6 hooks are sync; raises
  `SyncInAsyncContext` when invoked from inside a running loop).
* Per-attempt idempotency for DSPy retry loops — each retry IS a new
  reservation (parity with LangChain).

## References

* Spec: `docs/specs/coverage/D21_dspy/`
* Module: `sdk/python/src/spendguard/integrations/dspy/`
* Docs page: `docs/site-v2/src/content/docs/docs/integrations/dspy.mdx`
* D12 sibling: `sdk/python/src/spendguard/integrations/litellm_sdk_shim/`
  (transitive coverage when DSPy routes through LiteLLM)
