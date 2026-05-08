-- webhook_dedupe table (Provider Webhook Receiver Phase 2B Step 11).
--
-- Spec references:
--   - Stage 2 §8.2.3 (webhook flow + dedupe)
--   - Stage 2 §11    (receiver responsibilities; audit goes via ledger.audit_outbox)
--
-- Design source: /tmp/codex-webhook-r3.txt (v3 LOCKED at round 3).
--
-- Purpose: provider event-id replay cache. Receiver inserts row AFTER
-- a successful Ledger gRPC call; subsequent retries with same
-- (provider, event_kind, provider_account, provider_event_id) hit this
-- table for fast 200/replay or 409/conflict response without re-calling
-- ledger.
--
-- This table is NOT atomic with ledger transactions. Authoritative
-- idempotency lives in ledger_transactions (UNIQUE per
-- tenant_id + operation_kind + idempotency_key). webhook_dedupe is a
-- best-effort co-located replay cache.
--
-- POC limits documented:
--   - No TTL cleanup (production: 24h Redis-style cron worker)
--   - PK includes provider_account so account-scoped event IDs do not
--     collide across accounts (Codex r2 Q1)
--   - canonical_hash is byte-exact match with ledger handler
--     canonical_request_hash (per event_kind); see
--     services/webhook_receiver/src/domain/canonical_hash.rs
--     for the receiver's implementation. ledger handler hashes are at
--     services/ledger/src/handlers/provider_report.rs:260-280 and
--     services/ledger/src/handlers/invoice_reconcile.rs:342-362.

CREATE TABLE webhook_dedupe (
    provider              TEXT NOT NULL,
    event_kind            TEXT NOT NULL,
    provider_account      TEXT NOT NULL,
    provider_event_id     TEXT NOT NULL,
    canonical_hash        BYTEA NOT NULL,
    ledger_transaction_id UUID NOT NULL,
    recorded_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, event_kind, provider_account, provider_event_id)
);

CREATE INDEX webhook_dedupe_recorded_at_idx ON webhook_dedupe (recorded_at);
