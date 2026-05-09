# Microsoft Agent Governance Toolkit (AGT) integration

`SpendGuardCompositeEvaluator` runs AGT first (cheap deterministic
policy check), then SpendGuard's sidecar request_decision on AGT
ALLOW results. Saves a wasted reservation on AGT-deny actions.

```python
from agent_os.policies import (
    PolicyEvaluator, PolicyDocument, PolicyRule, PolicyCondition,
    PolicyAction, PolicyOperator, PolicyDefaults,
)
from spendguard import SpendGuardClient
from spendguard.integrations.agt import SpendGuardCompositeEvaluator

agt = PolicyEvaluator(policies=[PolicyDocument(...)])

composite = SpendGuardCompositeEvaluator(
    agt_evaluator=agt,
    spendguard_client=client,
    budget_id=...,
    # ...
    claim_estimator=lambda payload: [common_pb2.BudgetClaim(...)],
)

result = await composite.evaluate({
    "tool_name": "web_search",
    "tool_args": {"q": "..."},
    "tenant_id": "...",
    "run_id": "...",
})
# result.allowed: bool
# result.reason: "AGT_DENY: ..." | "SPENDGUARD_DENY: ..." | "ALLOW (...)"
```

Install: `pip install 'spendguard-sdk[agt]'`

Audit chain: AGT writes to its own audit log, SpendGuard writes to
`canonical_events`. Cross-system reconciliation (a relay that
ingests AGT events into SpendGuard) is a follow-on.
