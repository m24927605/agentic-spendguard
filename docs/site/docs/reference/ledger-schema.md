# Ledger schema

Migrations live in [services/ledger/migrations/](https://github.com/m24927605/agentic-flow-cost-evaluation/tree/main/services/ledger/migrations).

| Migration | Purpose |
|---|---|
| 0000-0006 | Foundations: shards, units, budgets, accounts, pricing, fencing |
| 0007-0009 | ledger_transactions + ledger_entries + audit_outbox |
| 0010-0011 | Projections + immutability triggers |
| 0012 | post_ledger_transaction SP (reserve / adjustment) |
| 0013-0015 | commit_estimated / provider_reported / release SPs |
| 0016 | invoice_reconciled SP (dual-row audit) |
| 0017 | webhook_dedupe |
| 0019 | release SP v2 (TTL sweeper extensions) |
| **0020** | **post_denied_decision SP (Phase 3 wedge)** |

The SPs are the sole authority on idempotent replay, fencing CAS,
lock-order canonicalization, and audit_outbox atomicity. Handlers
NEVER pre-check `ledger_transactions` — TOCTOU guard.
