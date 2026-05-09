# Dashboard

`http://<host>:8090/` (POC) — single-page operator UI showing budget
state, recent decisions, DENY histogram, and outbox forwarder health.

Auth: bearer token via Authorization header (single token per
instance for POC). Production resolves per-tenant via Microsoft Entra
ID JWT.

JSON endpoints (under `/api/`) are read-only:

- `/api/budgets` — net available / reserved / committed per (budget, unit)
- `/api/decisions` — last 50 ledger transactions
- `/api/deny-stats` — denied_decision count by hour over last 24h
- `/api/outbox-health` — pending + forwarded counts + oldest pending age

Source: `services/dashboard/`.
