-- Ledger transactions (per Ledger §5.2 + §22 v2.1 patch + Stage 2 alignment).

CREATE TABLE ledger_transactions (
    ledger_transaction_id  UUID         PRIMARY KEY,
    tenant_id              UUID         NOT NULL,

    operation_kind         TEXT         NOT NULL CHECK (operation_kind IN
                               ('reserve', 'release',
                                'commit_estimated', 'provider_report',
                                'invoice_reconcile',
                                'overrun_debt', 'adjustment',
                                'refund_credit', 'dispute_adjustment',
                                'compensating')),

    posting_state          TEXT         NOT NULL DEFAULT 'pending'
                               CHECK (posting_state IN
                                   ('pending', 'posted', 'voided')),
    posted_at              TIMESTAMPTZ,

    -- Idempotency replay (privacy split per Ledger §7).
    idempotency_key        TEXT         NOT NULL,
    request_hash           BYTEA        NOT NULL,
    minimal_replay_response JSONB        NOT NULL DEFAULT '{}'::JSONB,
    response_payload_ref   TEXT,
    response_payload_hash  BYTEA,
    replay_expires_at      TIMESTAMPTZ,

    -- CMK schema interface (Phase 1 reserved; Phase 2 active).
    encryption_key_id      TEXT,
    encryption_context     JSONB,

    -- Trace anchors (per Trace §11.1).
    trace_event_id         UUID,
    audit_decision_event_id UUID,

    -- Audit chain anchor (per Stage 2 §4.3).
    decision_id            UUID,

    -- Time semantics.
    effective_at           TIMESTAMPTZ  NOT NULL,
    recorded_at            TIMESTAMPTZ  NOT NULL DEFAULT clock_timestamp(),

    -- Lock ordering (per Stage 2 §8.2.1.1).
    lock_order_token       TEXT         NOT NULL,

    -- Fencing.
    fencing_scope_id       UUID         REFERENCES fencing_scopes(fencing_scope_id),
    fencing_epoch_at_post  BIGINT,

    -- Provider dispute (per Ledger §22 v2.1).
    provider_dispute_id    TEXT,
    case_state             TEXT,
    resolved_at            TIMESTAMPTZ,

    UNIQUE (tenant_id, operation_kind, idempotency_key)
);

CREATE INDEX idx_ledger_transactions_audit
    ON ledger_transactions (audit_decision_event_id);

CREATE INDEX idx_ledger_transactions_decision
    ON ledger_transactions (tenant_id, decision_id)
    WHERE decision_id IS NOT NULL;

CREATE INDEX idx_ledger_transactions_pending
    ON ledger_transactions (tenant_id, recorded_at)
    WHERE posting_state = 'pending';

CREATE INDEX idx_ledger_transactions_dispute
    ON ledger_transactions (provider_dispute_id, case_state)
    WHERE provider_dispute_id IS NOT NULL;
