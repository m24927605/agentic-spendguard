# Followup #4 — Approval notification outbox writes

GitHub issue: https://github.com/m24927605/agentic-flow-cost-evaluation/issues/4

## Goal

Make `resolve_approval_request` SP also write into the `approval_notifications`
outbox table when an approval transitions out of `pending`. Today the table
exists (migration 0027) but no INSERT path exists anywhere in the codebase, so
the dispatcher has nothing to forward and external notification (Slack / email
/ webhook) is silently no-op.

This is a closing-the-loop fix for Phase 5 S15.

## Files to read first

- `services/ledger/migrations/0027_approval_notifications.sql` — table schema
  (columns: notification_id, approval_id, dispatch_state, …; expects
  state='pending_dispatch' on insert)
- `services/ledger/migrations/0033_resolve_approval_ttl_atomic.sql` — the
  *current* `resolve_approval_request` SP definition (Codex round 9 version,
  has TTL guard already)
- `services/ledger/migrations/0030_approval_ttl_sweeper_system_actor.sql` —
  earlier system-actor injection version (for context on the `'expired'`
  branch)
- `services/control_plane/src/main.rs:resolve_approval` — caller of the SP

## Acceptance criteria

- New migration `services/ledger/migrations/0035_approval_notification_writes.sql`
  CREATE OR REPLACE the SP. The new body MUST keep all existing behavior
  (round-9 TTL atomic guard, round-5 system-actor injection) plus add an
  `INSERT INTO approval_notifications (...)` for the same transaction whenever
  `transitioned = TRUE` and `p_target_state IN ('approved', 'denied', 'cancelled')`.
- Decision call: also insert for `'expired'` (sweeper-driven). Default = yes,
  for symmetry, but if the schema's NOT NULL columns can't be filled
  meaningfully for the system path, document why in a `-- followup:` comment
  and skip that path.
- The SP body remains a single transactional unit — no nested transactions,
  no commits inside.
- Regenerate the integration smoke test you find in
  `services/ledger/tests/` (or write one if none exists) that asserts:
  approve → 1 row in approval_notifications with `dispatch_state='pending_dispatch'`.

## Pattern references

- Migration 0033 is the current SP body — copy verbatim, add the INSERT after
  the `INSERT INTO approval_events ... RETURNING ... INTO v_event_id;` block,
  before `RETURN QUERY ...`.
- For the SP smoke-test pattern, see how PR #2 round 4 / round 9 verified
  trigger + SP changes via inline `docker run --rm postgres:16-alpine`,
  applying all migrations in `ls -1 *.sql | sort` order, then issuing
  test SQL against the result.

## Verification

```bash
docker run --rm -d --name sg-mig-followup-04 -e POSTGRES_PASSWORD=test -e POSTGRES_DB=test postgres:16-alpine
sleep 4
for f in $(ls -1 services/ledger/migrations/*.sql | sort); do
  docker exec -i sg-mig-followup-04 psql -U postgres -d test -v ON_ERROR_STOP=1 < "$f"
done
# all 35 migrations apply OK

# Insert a pending approval, resolve to approved, assert 1 notification row exists
docker exec -i sg-mig-followup-04 psql -U postgres -d test < /tmp/smoke-followup-04.sql
docker rm -f sg-mig-followup-04
```

## Commit + close

```
fix(s15): wire approval_notifications outbox writes (followup #4)

resolve_approval_request now inserts a pending_dispatch row into
approval_notifications when an approval moves pending → terminal.
The dispatcher worker (separate followup) reads these rows.

Verified: smoke test on postgres:16; approve / deny / cancel path
each lands one notification row. Existing round-5/9 invariants
(system-actor injection, TTL atomic guard) preserved.
```

After merge: `gh issue close 4 --comment "Shipped in <commit-sha>"`.
