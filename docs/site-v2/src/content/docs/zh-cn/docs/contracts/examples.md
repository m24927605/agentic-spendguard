---
title: "Contract 规则示例"
---

## 硬上限(Hard cap)

```yaml
- id: hard-cap-stop
  when:
    budget_id: <uuid>
    claim_amount_atomic_gt: "1000000000"
  then:
    decision: STOP
    reason_code: BUDGET_EXHAUSTED
```

## 超过阈值走审批门(Approval gate)

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

## 按 provider 维度 DENY(占位;取决于规则分类体系的扩展)

```yaml
# Future syntax — POC doesn't yet support context fields beyond claim
# amount and budget_id. Tracked in the v1 spec.
```

目前请把按 provider 维度的规则放到应用边界处理(Pydantic-AI 的
claim_estimator 针对不同 provider 返回不同的 unit_id)。参见
[pricing-and-usd](../concepts/pricing-and-usd.md)。
