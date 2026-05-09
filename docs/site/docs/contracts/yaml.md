# Contract YAML reference

Minimum viable contract:

```yaml
apiVersion: contract.spendguard.io/v1alpha1
kind: Contract
metadata:
  id: 33333333-3333-4333-8333-333333333333
  name: my-contract
spec:
  budgets:
    - id: 44444444-4444-4444-8444-444444444444
      limit_amount_atomic: "1000000000"   # $1000 in atomic units
      currency: USD
      reservation_ttl_seconds: 600
      require_hard_cap: true
  rules:
    - id: hard-cap-deny
      when:
        budget_id: 44444444-4444-4444-8444-444444444444
        claim_amount_atomic_gt: "1000000000"
      then:
        decision: STOP
        reason_code: BUDGET_EXHAUSTED
    - id: threshold-approval
      when:
        budget_id: 44444444-4444-4444-8444-444444444444
        claim_amount_atomic_gte: "100000000"
      then:
        decision: REQUIRE_APPROVAL
        reason_code: AMOUNT_OVER_THRESHOLD
        approver_role: tenant-admin
```

## Decision values

- `CONTINUE` — proceed
- `DEGRADE` — apply mutation patch (POC: treated as APPLY_FAILED)
- `SKIP` — non-fatal skip
- `STOP` — terminate run
- `REQUIRE_APPROVAL` — pause pending operator (POC: terminal)

## Condition operators

POC supports:
- `claim_amount_atomic_gt` — claim > threshold
- `claim_amount_atomic_gte` — claim ≥ threshold

CEL predicates land in v1.

See [examples](examples.md) for common patterns.
