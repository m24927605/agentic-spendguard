# Followup #10 — Retention sweeper service (S19)

GitHub issue: https://github.com/m24927605/agentic-flow-cost-evaluation/issues/10

## Goal

Implement the retention background worker that applies
`tenant_data_policy.{prompt,provider_raw}_retention_days` against
`audit_outbox` and `provider_usage_records`. PR #2 S19 shipped the schema
+ DB-layer immutability triggers + tombstone-is-one-way trigger, but the
worker that actually redacts in place is stubbed.

Sequencing: do this **after** issue #11 (per-service metrics) so the new
service exposes its `retention_seconds_total` etc. counters from the
start.

## Files to read first

- `services/ledger/migrations/0028_retention_redaction.sql` — schema:
  `tenant_data_policy`, `retention_sweeper_log`; trigger that rejects
  DELETE on the audit chain and forces redaction-in-place
- `docs/site/docs/operations/data-classification.md` — operator
  playbook + per-table catalog of redaction rules
- `services/ttl_sweeper/src/main.rs` — reference shape (lease + poll loop
  + worker pattern + round-9 `is_leader_now` expiry check)
- `services/leases/src/lib.rs` — `LeaseManager` trait, both
  `PostgresLease` (default) and post-#5 `K8sLease`

## Acceptance criteria

- New crate `services/retention_sweeper/` with `src/main.rs` + `src/lib.rs`
  + `Cargo.toml`
- Polling loop pattern (mirror ttl_sweeper):
  - Acquire lease, gate every iteration on `state.is_leader_now()`
  - Fetch a bounded batch of candidate rows from `audit_outbox` where
    `cloudevent_payload->'data'` exists AND
    `recorded_at < now() - tenant.prompt_retention_days * INTERVAL '1 day'`
    AND not yet redacted
  - For each candidate: redact `cloudevent_payload->'data'` to the
    documented marker (`{"_redacted": true, "redacted_at": "..."}`) +
    copy original SHA-256 to `cloudevent_payload->'_data_sha256_hex'` so
    audit-chain hash continuity holds (this exact pattern is documented
    in `data-classification.md`)
  - Insert one row per pass into `retention_sweeper_log` with
    `(sweep_kind='audit_outbox_prompt', rows_redacted, started_at,
    finished_at)`
  - Same loop for `provider_usage_records.raw_payload` against
    `tenant.provider_raw_retention_days`
- Tombstone enforcement: writes from sidecar / webhook_receiver /
  control_plane must check `tenant_data_policy.tombstoned` and reject for
  tombstoned tenants. **This is application-level enforcement that needs
  to land in those three services, not in the new sweeper.** Add it as
  scope here or split into a separate followup; recommend doing it here
  since the sweeper is the same overall S19 wiring slice
- Helm chart adds `charts/spendguard/templates/retention-sweeper.yaml`
  mirroring ttl-sweeper.yaml structure (deployment + service +
  fail-gates from PR #2 round 5+6)
- `chart.profile=production` fail-gate from PR #2 round 6 already covers
  the env-mapping followup (#3); keep that consistent
- PKI script `deploy/demo/init/pki/generate.sh` adds `retention_sweeper`
  cert with `clientAuth` EKU
- Demo: new mode `DEMO_MODE=retention` (or extend a pseudo-mode in
  Makefile + demo runner) that:
  1. Sets `tenant_data_policy.prompt_retention_days=0` for the demo tenant
  2. Generates a couple of audit rows
  3. Asserts post-sweep that `cloudevent_payload->'data'->>'_redacted'` =
     'true' and `_data_sha256_hex` matches the original hash
- New `verify_step_retention.sql` in `deploy/demo/`
- Per-service metrics endpoint per issue #11 pattern (port 9100)
- 6+ unit tests + 1 integration test on real postgres

## Pattern references

- ttl_sweeper / outbox_forwarder workers — copy structure, especially
  round-9 `is_leader_now()` gating
- data-classification.md — the redaction marker format + which fields
  are class `prompt` / `provider_raw` / never-redact
- Round 4 trigger (migration 0029) shows what's frozen on terminal rows;
  the redaction must NOT trip those triggers (you're updating
  `cloudevent_payload`, not deleting rows)

## Verification

```bash
# Build + cargo check
cargo check -p spendguard-retention-sweeper
cargo test -p spendguard-retention-sweeper

# Demo verify
make demo-down
SIDECAR_TTL_SECONDS=600 make demo-up DEMO_MODE=retention
# expect: redacted markers present + _data_sha256_hex populated
```

## Commit + close

```
feat(s19): retention sweeper service (followup #10)

Closes the S19 worker gap. Background sweeper polls audit_outbox +
provider_usage_records, redacts cloudevent_payload->'data' in place
per tenant_data_policy retention windows, preserves SHA-256 hash
continuity for audit-chain integrity, logs every pass to
retention_sweeper_log.

Tombstone enforcement added at sidecar/webhook_receiver/control_plane
write paths.

Tests: 6 unit + 1 postgres integration; new DEMO_MODE=retention
verifies redaction marker + hash continuity.
```

After merge: `gh issue close 10 --comment "Shipped in <commit-sha>"`.
