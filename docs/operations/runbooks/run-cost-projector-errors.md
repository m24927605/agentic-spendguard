# Run Cost Projector Errors

Alert: `SpendGuardRunCostProjectorErrorRateHigh`

## Detection

Prometheus fires when `spendguard_run_cost_projector_project_total{outcome="err"}` exceeds 1 percent of all project calls for 5 minutes.

## Diagnosis

Check projector logs for database errors, invalid audit replay rows, and schema bundle mismatches. Compare Project and TerminateRun call counters to determine whether errors affect admission only or also run termination.

## Mitigation

Restore database connectivity or rollback the schema/config change causing errors. If admission cannot obtain a trusted projection, keep the fail-closed path active for budget-sensitive runs and page the owner of affected agent workloads.

## Rollback

Rollback the last projector image, migration, or config change. After rollback, run the runaway-loop demo path or equivalent canary before restoring normal traffic.

## Evidence

Record error-rate graphs, representative sanitized logs, failing query or config hash, rollback target, and the canary result.

## Safety

Do not switch projection failures to permissive admission. Preserve the STOP_RUN_PROJECTION behavior for runaway-loop protection.
