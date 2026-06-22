---
title: "Dashboard"
---

`http://<host>:8090/`（POC）—— 单页运维控制台，展示预算状态、最近决策、DENY 直方图，以及 outbox forwarder 健康状况。

鉴权：通过 Authorization header 传 bearer token（POC 阶段每个实例一个 token）。生产环境用 Microsoft Entra ID JWT 按租户解析。

`/api/` 下的 JSON 接口均为只读：

- `/api/budgets` —— 按 (budget, unit) 维度的 net available / reserved / committed
- `/api/decisions` —— 最近 50 条 ledger 事务
- `/api/deny-stats` —— 最近 24h 内按小时聚合的 denied_decision 计数
- `/api/outbox-health` —— pending + forwarded 计数，以及 oldest pending age

源码：`services/dashboard/`。
