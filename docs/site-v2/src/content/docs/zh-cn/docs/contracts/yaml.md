---
title: "Contract YAML 参考"
---

最小可用 contract:

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

## Decision 取值

- `CONTINUE` — 放行
- `DEGRADE` — 打上 mutation patch（POC：等同 APPLY_FAILED）
- `SKIP` — 非致命跳过
- `STOP` — 终止 run
- `REQUIRE_APPROVAL` — 暂停，等待 operator 介入（POC：终态）

## Condition 操作符

POC 支持：
- `claim_amount_atomic_gt` — claim > 阈值
- `claim_amount_atomic_gte` — claim ≥ 阈值

CEL predicate 在 v1 落地。

常见用法见 [examples](examples.md)。
