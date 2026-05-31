# Signature Quarantine Spike

Alert: `SpendGuardSignatureQuarantineSpike`

## Detection

Prometheus fires when `spendguard_ingest_events_quarantined_total` has a nonzero rate for 10 minutes.

## Diagnosis

Inspect canonical-ingest logs for quarantine reason values and producer identity. Distinguish expected quarantine from rollout drift, schema mismatch, or malformed event envelopes. Confirm route labels and bundle versions.

## Mitigation

Fix the producer envelope or trust configuration and replay only through the supported signed producer path. Keep quarantined events available for forensic review and do not promote them into canonical storage manually.

## Rollback

Rollback the producer or bundle version that started quarantine. Confirm the quarantine counter stops increasing and canonical-ingest accepts new signed events.

## Evidence

Capture quarantine-rate graphs, reason values, producer id, bundle hash, rollout version, and a sanitized sample of the rejected envelope shape.

## Safety

Do not hand-edit quarantined events into accepted canonical events. Do not weaken signature or schema validation during incident response.
