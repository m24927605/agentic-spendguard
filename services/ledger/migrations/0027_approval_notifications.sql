-- Phase 5 GA hardening S15: approval notification outbox.
--
-- Spec invariant: "External notification failure must not lose the
-- approval request." → notifications follow the same outbox pattern
-- as audit_outbox + outbox_forwarder. Approval state changes write
-- a notification row inside the same transaction as the
-- approval_events insert; a background dispatcher (S15-followup)
-- reads `pending_dispatch=TRUE` rows and POSTs them with retry +
-- HMAC signing.
--
-- Deduplication on (approval_id, transition_event_id) — the
-- dispatcher's at-least-once retries don't produce duplicate
-- webhook deliveries past the receiver's idempotency check.

CREATE TABLE approval_notifications (
    notification_id    UUID NOT NULL DEFAULT gen_random_uuid()
                       PRIMARY KEY,
    -- Source approval + transition event.
    approval_id        UUID NOT NULL REFERENCES approval_requests(approval_id),
    transition_event_id UUID NOT NULL REFERENCES approval_events(event_id),

    tenant_id          UUID NOT NULL,

    -- What state transition triggered this notification.
    transition_kind    TEXT NOT NULL CHECK (transition_kind IN
                           ('created', 'approved', 'denied',
                            'cancelled', 'expired')),

    -- Webhook target. Per-tenant config; resolved by dispatcher
    -- from a tenants.notification_webhook_url-style column
    -- (operator config; not in this schema slice).
    target_url         TEXT NOT NULL,

    -- HMAC signing key id (matches the secret store key). The
    -- actual secret stays in the dispatcher's environment, never
    -- in this table.
    signing_key_id     TEXT NOT NULL,

    -- Frozen at creation. The dispatcher serializes this to the
    -- HTTP body verbatim so the HMAC signature stays stable across
    -- retries.
    payload            JSONB NOT NULL,

    -- Forwarding state — only column allowed to UPDATE post-insert.
    pending_dispatch   BOOLEAN NOT NULL DEFAULT TRUE,
    dispatched_at      TIMESTAMPTZ,
    dispatch_attempts  INT NOT NULL DEFAULT 0,
    last_dispatch_error TEXT,
    next_retry_at      TIMESTAMPTZ,

    created_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),

    -- Per-transition uniqueness — even a buggy double-INSERT from
    -- the same SP run can't produce two rows.
    UNIQUE (approval_id, transition_event_id)
);

CREATE INDEX approval_notifications_pending_dispatch_idx
    ON approval_notifications (next_retry_at NULLS FIRST, created_at)
    WHERE pending_dispatch = TRUE;

CREATE INDEX approval_notifications_tenant_recent_idx
    ON approval_notifications (tenant_id, created_at DESC);

COMMENT ON TABLE approval_notifications IS
    'S15: outbox for approval-state-change webhooks. Background dispatcher (S15-followup) polls pending_dispatch=TRUE rows + POSTs with HMAC sig + exponential backoff retry. UNIQUE on (approval_id, event) deduplicates retries.';
