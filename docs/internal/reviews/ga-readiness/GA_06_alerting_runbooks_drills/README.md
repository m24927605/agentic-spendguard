# GA_06 Alerting, Runbooks, and Drills Evidence

Status: implementation evidence captured on 2026-05-31.

## Scope

GA_06 replaces placeholder/stale alert rules with metrics emitted by checked-in services, adds runbooks for every GA-critical alert, and adds an outbox backlog recovery drill.

## Artifacts

| Artifact | Purpose |
|---|---|
| `outbox_lag_recovery.json` | Evidence from the docker-compose drill proving outbox lag alertability and recovery |
| `command-results.md` | Acceptance command log |

## Drill Summary

The drill ran `make demo-up DEMO_MODE=default`, reopened one successfully forwarded runtime audit outbox row by changing only forwarder-state columns, stopped canonical-ingest to produce backlog, observed `spendguard_outbox_pending_oldest_age_seconds > 60` continuously through the alert `for` duration, restarted canonical-ingest, and verified pending rows returned to zero.
