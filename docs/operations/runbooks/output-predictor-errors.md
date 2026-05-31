# Output Predictor Errors

Alert: `SpendGuardOutputPredictorErrorRateHigh`

## Detection

Prometheus fires when `spendguard_output_predictor_predict_total{outcome="err"}` exceeds 1 percent of all predict calls for 5 minutes.

## Diagnosis

Check output-predictor logs for validation failures, plugin TLS errors, and cache load failures. Compare error counts with `customer_predictor_failure_mode_total` to separate plugin failures from local predictor failures. Confirm the schema bundle hash and tenant SVID material match the active deployment.

## Mitigation

Fail affected tenants to Strategy B through the configured breaker path when errors are plugin-related. Restart unhealthy replicas only after confirming the error is process-local. If errors come from bad config, revert that config before scaling.

## Rollback

Rollback the image, schema bundle, or routing config that introduced the error spike. Keep the fallback route active until the error rate remains below 0.1 percent for 15 minutes.

## Evidence

Save the error-rate graph, representative sanitized log lines, breaker decisions, config version, image digest, and a post-recovery metric snapshot.

## Safety

Do not bypass SVID checks, tenant isolation, or signed bundle validation to make calls succeed. Preserve audit-chain writes for all decisions.
