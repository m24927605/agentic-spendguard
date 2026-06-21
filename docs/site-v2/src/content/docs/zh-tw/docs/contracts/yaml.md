---
title: "Contract YAML 參考"
---

最小可用的 contract 長這樣:

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

## Decision 值

- `CONTINUE` — 放行,繼續執行
- `DEGRADE` — 套用 mutation patch(POC 階段:會被當成 APPLY_FAILED 處理)
- `SKIP` — 非致命的略過
- `STOP` — 終止整個 run
- `REQUIRE_APPROVAL` — 暫停,等待 operator 介入(POC 階段:視為終態)

## Condition 運算子

POC 目前支援:
- `claim_amount_atomic_gt` — claim > threshold
- `claim_amount_atomic_gte` — claim ≥ threshold

CEL predicate 會在 v1 才進來。

常見的寫法可以參考 [examples](examples.md)。
