# Audit Outbox Lag

Alert: `SpendGuardOutboxLagHigh`

## Detection

Prometheus fires when `spendguard_outbox_pending_oldest_age_seconds` stays above 60 seconds for 5 minutes.

## Diagnosis

Check the outbox-forwarder logs, leader metric, canonical-ingest health, and database connectivity. Query pending count and oldest pending age from `audit_outbox` using read-only SELECT statements. Confirm whether canonical-ingest rejects signatures, is unreachable, or is draining slowly.

## Mitigation

Restore canonical-ingest availability first. If one forwarder is leader and canonical-ingest is healthy, restart the leader pod to clear process-local transport state. If backlog is large, temporarily increase outbox-forwarder batch size within tested limits and monitor canonical-ingest reject and quarantine counters.

## Rollback

Return batch size and replica count to the previous values after pending count is zero and canonical_events has received the forwarded events. Rollback any image or certificate change that caused forwarding failures.

## Evidence

Capture pending count, oldest pending age, outbox-forwarder leader count, canonical-ingest reject/quarantine metrics, and the recovery command sequence. Attach the `outbox_lag_recovery.json` drill artifact when the incident is simulated.

## Safety

Do not delete immutable audit rows, edit CloudEvent payloads, or suppress signature verification. Only forwarder-state columns may change during a controlled drill.
