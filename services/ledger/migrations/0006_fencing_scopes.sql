-- Fencing scopes (per Ledger §5.4 + §22 v2.1 patch).

CREATE TABLE fencing_scopes (
    fencing_scope_id        UUID        PRIMARY KEY,
    scope_type              TEXT        NOT NULL CHECK (scope_type IN
                                ('reservation', 'budget_window',
                                 'control_plane_writer')),
    tenant_id               UUID        NOT NULL,
    budget_id               UUID,
    reservation_id          UUID,
    window_instance_id      UUID,
    workload_kind           TEXT,
    current_epoch           BIGINT      NOT NULL DEFAULT 0,
    active_owner_instance_id TEXT,
    ttl_expires_at          TIMESTAMPTZ,
    epoch_source_authority  TEXT        NOT NULL DEFAULT 'ledger_lease',
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),

    CHECK (
        (scope_type = 'reservation'
            AND reservation_id IS NOT NULL
            AND window_instance_id IS NULL
            AND budget_id IS NOT NULL) OR
        (scope_type = 'budget_window'
            AND window_instance_id IS NOT NULL
            AND reservation_id IS NULL
            AND budget_id IS NOT NULL) OR
        (scope_type = 'control_plane_writer'
            AND budget_id IS NULL
            AND reservation_id IS NULL
            AND window_instance_id IS NULL
            AND workload_kind IS NOT NULL)
    )
);

CREATE UNIQUE INDEX fencing_scope_reservation_uq
    ON fencing_scopes (tenant_id, budget_id, reservation_id)
    WHERE scope_type = 'reservation';

CREATE UNIQUE INDEX fencing_scope_budget_window_uq
    ON fencing_scopes (tenant_id, budget_id, window_instance_id)
    WHERE scope_type = 'budget_window';

CREATE UNIQUE INDEX fencing_scope_control_plane_writer_uq
    ON fencing_scopes (tenant_id, workload_kind)
    WHERE scope_type = 'control_plane_writer';

CREATE INDEX idx_fencing_active_lookup
    ON fencing_scopes (scope_type, tenant_id, budget_id, ttl_expires_at);

-- Per Ledger §22 v2.1: fencing history projection.
CREATE TABLE fencing_scope_events (
    fencing_event_id  UUID        PRIMARY KEY,
    fencing_scope_id  UUID        NOT NULL REFERENCES fencing_scopes(fencing_scope_id),
    old_epoch         BIGINT      NOT NULL,
    new_epoch         BIGINT      NOT NULL,
    owner_instance_id TEXT        NOT NULL,
    action            TEXT        NOT NULL CHECK (action IN
                          ('acquire', 'renew', 'revoke', 'promote', 'recover')),
    audit_event_id    UUID        NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE INDEX idx_fencing_scope_events_history
    ON fencing_scope_events (fencing_scope_id, created_at);

COMMENT ON TABLE fencing_scopes IS
    'Single source of fencing authority (per Sidecar §9, Stage 2 §4.4). Webhook receivers + reconciliation use control_plane_writer scope.';
