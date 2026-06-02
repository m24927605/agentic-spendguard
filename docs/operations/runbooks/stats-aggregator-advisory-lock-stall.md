# Stats Aggregator Advisory Lock Stall

Alert: sustained `stats_aggregator_skipped_lock_held_total` growth with no successful aggregation cycle, or `StatsAggregatorCycleStale` when only one replica should be active.

## Detection

Check whether the singleton advisory lock is held by a stale Postgres session:

```sql
SELECT
  a.pid,
  a.usename,
  a.application_name,
  a.client_addr,
  a.state,
  a.backend_start,
  a.query_start,
  a.wait_event_type,
  a.wait_event,
  a.query
FROM pg_locks l
JOIN pg_stat_activity a ON a.pid = l.pid
WHERE l.locktype = 'advisory'
  AND l.granted
  AND l.classid = ((6003373350444290643::bigint >> 32) & 4294967295)::oid
  AND l.objid = (6003373350444290643::bigint & 4294967295)::oid;
```

The lock id is `6003373350444290643` (`0x5350444147475253`, `SPDAGGRS`) from `services/stats_aggregator/src/aggregation.rs`.

## Diagnosis

Confirm the expected replica count and compare the lock holder PID to live pods. If the holder pod no longer exists or has lost network connectivity, Postgres may keep the session until TCP keepalive or the load balancer idle timeout closes it.

Check database keepalive settings:

```sql
SHOW tcp_keepalives_idle;
SHOW tcp_keepalives_interval;
SHOW tcp_keepalives_count;
```

For production, tune Postgres or the managed database parameter group so the keepalive failure window is shorter than the stats freshness alert window. A common target is idle <= 60s, interval <= 30s, count <= 5, adjusted to provider limits.

## Mitigation

If the holder pod is healthy, leave it alone and inspect why cycles are slow. If the holder is stale, terminate only that backend:

```sql
SELECT pg_terminate_backend(<pid>);
```

Then watch one stats-aggregator pod acquire the lock and complete a cycle. Do not scale multiple manual jobs while the old backend is still present.

## Rollback

Rollback the deployment or DB parameter change that introduced half-open sessions. If keepalive tuning causes false disconnects, revert the parameter group and keep one stats-aggregator replica until a safer network timeout is available.

## Evidence

Record the alert time, skipped-lock metric, last successful cycle age, `pg_locks`/`pg_stat_activity` output, keepalive settings, the backend PID terminated, and the first successful cycle after recovery.

## Safety

Advisory lock termination only stops the singleton aggregation cycle. It does not mutate immutable audit rows. Do not terminate unrelated ledger, canonical_ingest, or customer workload sessions.
