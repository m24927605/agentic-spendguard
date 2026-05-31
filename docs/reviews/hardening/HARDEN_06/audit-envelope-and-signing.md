# HARDEN 06 Audit Envelope And Signing Notes

## Scope

- Tokenizer drift alert sink already builds a full canonical_ingest `AppendEventsRequest` envelope with `producer_id`, `schema_bundle`, and `route = Observability`.
- Control-plane plugin lifecycle audit rows now have an in-process forwarder that signs each pending outbox row and relays it to canonical_ingest.
- The control-plane forwarder uses per-event signatures as canonical truth; batch signatures remain empty, matching the in-cluster mTLS producer convention used by tokenizer/stats paths.

## Locked Decisions

- Control-plane handlers continue writing `control_plane_audit_outbox` inside the mutation transaction. Forwarding happens asynchronously from pending rows so canonical_ingest downtime preserves audit events.
- `forwarded_at IS NULL` is the durable retry cursor. A successful AppendEvents response updates `cloudevent_payload_signature_hex` and `forwarded_at` in the same DB transaction that locked the row.
- Forwarded plugin lifecycle events use `route = Observability` and retain the `spendguard.audit.plugin_*.v1alpha1` event type family.
- The signer source is `SPENDGUARD_CONTROL_PLANE_*`, matching the shared `spendguard-signing` local/KMS/disabled environment contract.

## Verification

- `cargo test --manifest-path services/control_plane/Cargo.toml audit_forwarder -- --nocapture`

## Review Focus

- The forwarder must not weaken canonical_ingest envelope validation.
- Production deployments must provide signer and canonical_ingest configuration when the control-plane audit forwarder is enabled.
- Duplicate sends rely on HARDEN_05 replay dedup and the outbox row lock/forwarded marker.
