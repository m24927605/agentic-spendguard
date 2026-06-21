---
title: "Contract 規則範例"
---

## 硬上限（Hard cap）

```yaml
- id: hard-cap-stop
  when:
    budget_id: <uuid>
    claim_amount_atomic_gt: "1000000000"
  then:
    decision: STOP
    reason_code: BUDGET_EXHAUSTED
```

## 超過門檻就走核可流程

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

## 各 provider 個別 DENY（佔位用；要等規則 taxonomy 擴充）

```yaml
# Future syntax — POC doesn't yet support context fields beyond claim
# amount and budget_id. Tracked in the v1 spec.
```

目前先把各 provider 的規則放在 application 邊界處理（讓
Pydantic-AI 的 claim_estimator 依不同 provider 回傳不同的 unit_id）。
詳見 [pricing-and-usd](../concepts/pricing-and-usd.md)。
