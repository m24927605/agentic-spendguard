---
title: "Dashboard"
---

`http://<host>:8090/` (POC) — 單頁 operator UI,顯示 budget
狀態、近期 decisions、DENY 直方圖,以及 outbox forwarder 健康狀態。

驗證方式:透過 Authorization header 帶 bearer token(POC 階段每個 instance
一組 token)。正式環境則用 Microsoft Entra ID JWT 逐租戶解析。

JSON endpoints(位於 `/api/` 下)皆為唯讀:

- `/api/budgets` — 每組 (budget, unit) 的 net available / reserved / committed
- `/api/decisions` — 最近 50 筆 ledger transactions
- `/api/deny-stats` — 過去 24h 內每小時的 denied_decision 數量
- `/api/outbox-health` — pending + forwarded 計數,以及最舊一筆 pending 已等待的時間

來源:`services/dashboard/`。
