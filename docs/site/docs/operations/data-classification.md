# Data classification (Phase 5 S19)

Catalog of every event field SpendGuard records, with its data
class. Operators consult this when configuring
`tenant_data_policy.export_redaction_field_paths` and when
running compliance reviews.

## Classes

| Class            | Description                                      | Default redaction at export | Default retention      |
|------------------|--------------------------------------------------|------------------------------|-----------------------|
| `metadata`       | Identity, structural, never user content         | NEVER redacted               | full audit window     |
| `pricing`        | Pricing freeze tuple, model id, token counts     | NEVER redacted (billing evidence) | full audit window |
| `decision`       | matched_rule_ids, reason_codes, contract version | NEVER redacted               | full audit window     |
| `prompt`         | User prompt or LLM input/output text             | redacted by default at export | tenant.prompt_retention_days |
| `provider_raw`   | Verbatim provider API response                   | redacted by default at export | tenant.provider_raw_retention_days |
| `pii`            | User identity (email, name) when in payload      | redacted by default at export | tenant.prompt_retention_days |
| `provider_secret`| Webhook signing keys, API tokens (in error logs only) | NEVER stored in events    | n/a                   |

## Per-table catalog

### `audit_outbox` (and `canonical_events`)

| Field path                              | Class      | Notes                                                   |
|-----------------------------------------|------------|---------------------------------------------------------|
| `audit_outbox_id`                       | metadata   | UUID v7                                                 |
| `tenant_id`                             | metadata   |                                                         |
| `decision_id`                           | metadata   |                                                         |
| `audit_decision_event_id`               | metadata   |                                                         |
| `event_type`                            | metadata   |                                                         |
| `cloudevent_payload->>'specversion'`    | metadata   |                                                         |
| `cloudevent_payload->>'type'`           | metadata   |                                                         |
| `cloudevent_payload->>'source'`         | metadata   |                                                         |
| `cloudevent_payload->>'id'`             | metadata   |                                                         |
| `cloudevent_payload->>'time'`           | metadata   |                                                         |
| `cloudevent_payload->'data'`            | **prompt** | Raw decision/outcome content; may include user prompt   |
| `cloudevent_payload->'data'->>'snapshot_hash'` | metadata | hex hash, not content                              |
| `cloudevent_payload->>'producer_id'`    | metadata   |                                                         |
| `cloudevent_payload->>'producer_sequence'` | metadata |                                                       |
| `cloudevent_payload_signature`          | metadata   | Ed25519 signature bytes                                 |
| `signing_key_id`                        | metadata   |                                                         |
| `producer_sequence`                     | metadata   |                                                         |
| `idempotency_key`                       | metadata   | Hash of business intent; not content                    |
| `recorded_at`                           | metadata   |                                                         |

The S19 redaction sweeper, when prompt_retention_days has
elapsed, sets `cloudevent_payload->'data'` to a marker
JSONB (`{"_redacted": true, "redacted_at": "..."}`) and
copies the SHA-256 hash of the original bytes to a separate
`cloudevent_payload->'_data_sha256_hex'` field. The audit
chain hash stays valid because the producer_signature was
computed over the ORIGINAL bytes; verifiers re-derive
canonical bytes from the redacted form's hash + the
remaining metadata.

### `provider_usage_records`

| Field path           | Class           | Notes                                              |
|----------------------|-----------------|----------------------------------------------------|
| `record_id`          | metadata        |                                                    |
| `provider`           | metadata        |                                                    |
| `tenant_id`          | metadata        |                                                    |
| `provider_event_id`  | metadata        |                                                    |
| `model_id`           | metadata        |                                                    |
| `prompt_tokens`      | pricing         | Token count, not text                              |
| `completion_tokens`  | pricing         |                                                    |
| `cost_micros_usd`    | pricing         |                                                    |
| `raw_payload`        | **provider_raw**| Verbatim provider API response — may include text  |

S19 redaction sweeper for `provider_raw_retention_days`:
clears `raw_payload` (sets to `{"_redacted": true, ...}`)
+ retains only the structured fields above for billing
forensics.

### `ledger_transactions` / `ledger_entries`

All fields class `metadata` or `pricing` — no prompt /
provider raw content lands in these tables. NEVER redacted
or deleted (S19 invariant; trigger enforces).

### `approval_requests`

| Field path           | Class      | Notes                                                  |
|----------------------|------------|--------------------------------------------------------|
| `decision_context`   | mixed      | Contains pricing tuple (metadata) + matched rules (decision); may include `data` echo (prompt) |
| `requested_effect`   | metadata   | Projected claims; no prompt content                    |
| `resolution_reason`  | metadata   | Operator-supplied text (typically business reason)     |

If `decision_context.data` echoes prompt bytes, the export
redaction policy applies the same way as for
`audit_outbox.cloudevent_payload->'data'`.

## Operator playbook

### Set tenant prompt retention to 0 (store hashes only)

```sql
UPDATE tenant_data_policy
   SET prompt_retention_days = 0,
       updated_by = 'me@example.com'
 WHERE tenant_id = '...';
```

The retention sweeper (S19-followup) on its next pass finds
any audit rows whose `cloudevent_payload->'data'` field is
non-empty for this tenant and redacts them in place. New
events from that tenant get redacted at write time
(application-level enforcement).

### Tombstone a tenant

```sql
UPDATE tenant_data_policy
   SET tombstoned = TRUE,
       tombstoned_at = clock_timestamp(),
       tombstoned_by = 'me@example.com',
       tombstoned_reason = 'customer offboarded'
 WHERE tenant_id = '...';
```

The trigger enforces tombstone is one-way (cannot revert).
Tombstoned tenant's audit chain stays queryable.

### Audit recent redactions

```sql
SELECT sweep_kind, count(*), sum(rows_redacted)
  FROM retention_sweeper_log
 WHERE started_at > now() - interval '30 days'
 GROUP BY 1;
```

## What's NEVER redacted (S19 invariants)

The DB-layer triggers in migration 0028 reject DELETE on:
- `audit_outbox`
- `audit_outbox_global_keys`
- `ledger_transactions`
- `ledger_entries`

The retention sweeper UPDATEs (redacts in place) but never
DELETEs. Even SUPERUSER would have to disable triggers
explicitly to remove rows — that action would be visible
in pg_audit logs.

## Gaps (S19-followup)

1. **Retention sweeper service** not yet shipped. The
   schema is in place; the background worker that scans
   audit_outbox + provider_usage_records + applies the
   redaction is the next chunk.
2. **Application-level write-time redaction** (when
   `prompt_retention_days = 0`) needs sidecar +
   webhook_receiver code paths to consult
   `tenant_data_policy` before writing the `data` field.
3. **Export endpoint redaction** (S9) needs to consult
   `tenant_data_policy.export_redaction_field_paths` and
   strip those JSONB paths before the JSONL line goes out.
4. **Application-level tombstone enforcement** — sidecar /
   webhook_receiver / control_plane MUST check
   `tenant_data_policy.tombstoned` and reject writes for
   that tenant. Defense in depth: existing rows stay
   queryable.
