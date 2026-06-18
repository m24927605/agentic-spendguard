# GA Soak Runbook

## Objective

Run a sustained SpendGuard predictor stack and collect periodic proof that audit forwarding, stats aggregation, plugin SVID behavior, and container health remain stable.

## Local Acceptance

```bash
scripts/soak/ga-soak.sh --duration 30m --profile local
```

The command resets the demo stack, runs `make demo-up DEMO_MODE=default`, starts the GA predictor and operations services, waits for a stats aggregation cycle, and records snapshots until the duration elapses.

## Release Gate

```bash
scripts/soak/ga-soak.sh --duration 24h --interval 5m --profile local
```

The release gate should run from a clean checkout with no unrelated compose stack using the same service names.

## Failure Signals

- Any nonzero `audit_outbox.pending_forward` count.
- Outbox lag above the configured `MAX_OUTBOX_LAG_SECONDS`.
- Outbox leader count different from one.
- Zero canonical audit rows or canonical row-count regression after the initial demo traffic.
- No stats aggregation cycle, stale last-cycle timestamp, or cycle errors.
- Failed `verify-chain` / audit-column probe.
- Failed predictor-client SVID subject probe.
- Any required container not running or unhealthy.
- Container RSS growth above `MAX_MEMORY_GROWTH_BYTES` from the first snapshot.

## Evidence

The harness writes:

- `docs/internal/reviews/ga-readiness/GA_07_soak_harness/ga_soak_snapshots.jsonl`
- `docs/internal/reviews/ga-readiness/GA_07_soak_harness/ga_soak_summary.json`
- `docs/internal/reviews/ga-readiness/GA_07_soak_harness/ga_soak_baseline.json`

Attach all three files to release evidence. The JSONL file is intentionally append-only during the run so a failed soak still leaves the last successful snapshot.

## Safety

The harness targets the demo compose stack only. It does not mutate immutable audit payload columns, relax signature verification, or disable RLS. It may reset local demo volumes unless `--no-reset` is supplied.
