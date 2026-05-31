# Run Cost Projector Latency

Alert: `SpendGuardRunCostProjectorLatencyHigh`

## Detection

Prometheus fires when the 5 minute p99 for `spendguard_run_cost_projector_project_latency_seconds_bucket` stays above 500 ms for 10 minutes.

## Diagnosis

Check run-cost-projector CPU, database connection pool saturation, and audit replay query latency. Confirm recent workload changes in long-running agents and inspect whether cold cache rebuilds are increasing replay volume.

## Mitigation

Scale the service if process saturation is clear. If replay pressure is the driver, warm the run-length cache from existing audit_outbox data and reduce concurrent expensive agent runs through admission policy. Keep fail-closed STOP behavior intact for over-budget projections.

## Rollback

Rollback the latest projector image or config change if latency began after deployment. Revert admission-policy changes after projector p99 and queue depth return to baseline.

## Evidence

Capture p99 latency, database query timings, replay counts, active run counts, image digest, and the exact mitigation used.

## Safety

Do not disable projection enforcement or widen run budgets to hide latency. Do not alter immutable audit event content during replay diagnosis.
