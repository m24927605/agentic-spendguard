# Contract rule examples

## Hard cap

```yaml
- id: hard-cap-stop
  when:
    budget_id: <uuid>
    claim_amount_atomic_gt: "1000000000"
  then:
    decision: STOP
    reason_code: BUDGET_EXHAUSTED
```

## Approval gate above threshold

```yaml
- id: large-spend-approval
  when:
    budget_id: <uuid>
    claim_amount_atomic_gte: "100000000"   # $100 in micro-USD if budget is USD
  then:
    decision: REQUIRE_APPROVAL
    reason_code: AMOUNT_OVER_THRESHOLD
    approver_role: tenant-admin
```

## Per-provider DENY (placeholder; depends on rule taxonomy expansion)

```yaml
# Future syntax — POC doesn't yet support context fields beyond claim
# amount and budget_id. Tracked in the v1 spec.
```

For now, place per-provider rules at the application boundary
(Pydantic-AI claim_estimator returning different unit_id per
provider). See [pricing-and-usd](../concepts/pricing-and-usd.md).
