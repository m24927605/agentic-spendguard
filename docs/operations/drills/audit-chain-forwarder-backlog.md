# Audit Chain Forwarder Backlog Drill

## Objective

Reproduce an outbox backlog without fabricating audit payloads, verify `SpendGuardOutboxLagHigh` has a real metric source, and prove recovery drains the backlog into canonical-ingest.

## Scope

This drill exercises the demo Postgres, ledger, canonical-ingest, sidecar, and outbox-forwarder topology. It does not alter immutable audit payload columns.

## Preconditions

- Docker Compose is available.
- No production kube-context is targeted.
- The operator is working from the repository root.

## Commands

```bash
tests/e2e/outbox_lag_recovery.sh
```

The script resets the demo stack, runs `make demo-up DEMO_MODE=default`, stops canonical-ingest, reopens one real successfully forwarded runtime outbox row by changing only forwarder-state columns, waits until the outbox lag metric is strictly above the alert threshold, holds that predicate through the alert `for` duration, restarts canonical-ingest, and waits for the backlog to drain.

## Expected Alert

`SpendGuardOutboxLagHigh` should be satisfiable because `spendguard_outbox_pending_oldest_age_seconds` is emitted by outbox-forwarder and should stay above 60 seconds for 5 minutes while canonical-ingest is unavailable.

## Recovery

Canonical-ingest restart is the primary recovery action. The outbox-forwarder should resend the pending event, canonical-ingest should dedupe or accept it idempotently, and `audit_outbox.pending_forward` should return to zero pending rows.

## Evidence

The drill writes `docs/internal/reviews/ga-readiness/GA_06_alerting_runbooks_drills/outbox_lag_recovery.json` with pending counts, lag metrics, and scrape excerpts.

## Safety

The drill only changes forwarder-state columns on a real audit_outbox row. It does not remove audit rows, alter CloudEvent payloads, or loosen signature verification.
