# Output Predictor Latency

Alert: `SpendGuardOutputPredictorLatencyHigh`

## Detection

Prometheus fires when the 5 minute p99 for `spendguard_output_predictor_predict_latency_seconds_bucket` stays above 500 ms for 10 minutes.

## Diagnosis

Check the output-predictor pod CPU, memory, and restart counters. Compare `customer_predictor_call_total` with `spendguard_output_predictor_cache_lookup_total` and `spendguard_output_predictor_cache_hit_total` to determine whether Strategy C plugin traffic or cache misses are driving latency. Inspect recent deploys, plugin endpoint health, and database latency before changing routing.

## Mitigation

Scale output-predictor replicas if saturation is visible. If plugin latency is the driver, use the existing Strategy C breaker controls to fall back to Strategy B for the affected tenant. Keep tenant isolation checks enabled and confirm calls continue to return bounded predictions.

## Rollback

Rollback the most recent output-predictor image or config change through the deployment controller. Re-enable Strategy C only after p99 latency is below 500 ms for at least 15 minutes and plugin health checks pass.

## Evidence

Record the alert start and clear time, affected tenants if known, p99 graph, cache hit ratio, breaker state, and the deployment or config version before and after mitigation.

## Safety

Do not mutate audit payload columns or remove audit rows while responding. Do not relax tenant SVID validation to reduce plugin latency.
