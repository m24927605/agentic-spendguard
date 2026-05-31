# Prediction Drift Alerts

Alert: `SpendGuardPredictionDriftAlerting`

## Detection

Prometheus fires when `spendguard_stats_aggregator_drift_alerts_total` increases during the last hour.

## Diagnosis

Review the stats-aggregator drift alert payload, affected strategy, tokenizer tier, model provider, and calibration window. Compare recent benchmark data with production audit_outbox and canonical_events mirrors to identify whether drift is model-specific or systemic.

## Mitigation

Pause rollout of the affected predictor strategy or route affected tenants to the better-calibrated fallback. Refresh cold-start baselines or calibration windows only after confirming the source data and tokenizer version are correct.

## Rollback

Rollback the predictor policy, cold-start table, or calibration config that introduced drift. Restore traffic gradually and watch the drift counter for a full calibration window.

## Evidence

Store the drift alert id, affected strategy/provider/tokenizer tier, calibration report, policy version, and post-rollback metric snapshot.

## Safety

Do not overwrite historical calibration evidence. Predictor fallback changes must keep audit-chain mirrors populated.
