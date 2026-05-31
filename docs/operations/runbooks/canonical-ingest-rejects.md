# Canonical Ingest Signature Rejects

Alert: `SpendGuardCanonicalIngestRejecting`

## Detection

Prometheus fires when `spendguard_ingest_events_rejected_invalid_signature_total` has a nonzero rate for 5 minutes.

## Diagnosis

Identify the route label, producer identity, signing key id, and schema bundle hash from canonical-ingest logs. Compare the producer trust-store material with the mounted canonical-ingest trust store. Check for recent key rotation, schema bundle changes, and clock skew.

## Mitigation

Restore the correct trust-store contents or rollback the producer signing configuration. Keep rejecting invalid events while trusted producers are corrected. If only one producer is affected, isolate that producer and leave other routes running.

## Rollback

Rollback the producer image/config or trust-store secret that introduced signature failures. After rollback, verify accepts resume and reject rate returns to zero.

## Evidence

Save reject-rate graphs, route label, producer id, key id, schema bundle hash, trust-store secret version, and post-rollback acceptance evidence.

## Safety

Do not relax strict signature verification to clear this alert. Invalid events must remain outside the canonical audit chain.
