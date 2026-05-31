# Tokenizer Drift Escalation

Alert: `SpendGuardTokenizerDriftEscalation`

## Detection

Prometheus fires when `spendguard_tokenizer_drift_alert_oncall_escalation_total` increases during the last hour.

## Diagnosis

Check tokenizer service logs, encoder version ids, provider/model mapping, and recent SDK default-estimator changes. Compare token counts between centralized tokenizer output and provider-reported usage for the affected model.

## Mitigation

Route affected models to the trusted tokenizer tier or freeze the model mapping that introduced drift. If provider reporting changed, update the estimator only after the calibration report confirms bounded error.

## Rollback

Rollback the tokenizer version registry or SDK estimator config that caused escalation. Confirm new events carry the expected tokenizer_version_id and tokenizer_tier.

## Evidence

Capture escalation graph, model/provider, tokenizer_version_id, sample counts, estimator config hash, and post-recovery calibration output.

## Safety

Do not drop tokenizer version ids from audit rows to hide drift. Keep per-tenant routing and SVID validation unchanged.
