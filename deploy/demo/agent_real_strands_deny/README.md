# DEMO_MODE=agent_real_strands_deny

COV_D20 SLICE 5 — Strands DENY-path variant.

## What this demo proves

When the sidecar returns DENY on `before_invocation`,
`SpendGuardStrandsHookProvider` raises `DecisionDenied` which Strands'
typed event bus surfaces as a `HookExecutionError`. The provider HTTP
is **never** issued — the counting-stub records `calls == 0` for
that turn.

This is the INV-2 "zero provider HTTP on DENY" guarantee that makes
SpendGuard's gating safe to deploy in front of expensive frontier
models.

## How to run

```bash
make demo-up DEMO_MODE=agent_real_strands_deny
```

The runner walks one DENY turn, asserts the upstream stub recorded
zero hits, then exits 0.

## Driver

Same `SpendGuardStrandsHookProvider` wiring as `agent_real_strands`,
but the sidecar contract is preloaded to return STOP on the second
turn. The driver:

1. Runs one ALLOW turn (proves the happy path still works in this
   demo mode) → counting-stub hits = 1.
2. Runs one DENY turn → catches `HookExecutionError` whose
   `__cause__` is `DecisionDenied`, asserts counting-stub hits is
   still 1 (no new HTTP fired).

## Verification

`make demo-up DEMO_MODE=agent_real_strands_deny` runs the shared
`verify_step_agent_real_strands.sql` against `spendguard_ledger`,
asserting:

* `>= 1` `reserve` row (ALLOW turn 1)
* `>= 1` `commit_estimated` row (ALLOW turn 1)
* `>= 1` `denied_decision` row (DENY turn 2)
* INV-2 strict order: earliest `reserve` predates earliest
  `commit_estimated`
* `decision_context_json->>'integration' = 'strands'` on every row

## References

* Spec: `docs/specs/coverage/D20_aws_strands/`
* Module: `sdk/python/src/spendguard/integrations/strands/`
* Sibling demo: `deploy/demo/agent_real_strands/`
