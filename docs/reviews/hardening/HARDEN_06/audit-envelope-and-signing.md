# HARDEN 06 Audit Envelope And Signing Notes

## Scope

- Tokenizer drift alert sink already builds a full canonical_ingest `AppendEventsRequest` envelope with `producer_id`, `schema_bundle`, and `route = Observability`.
- Control-plane plugin lifecycle audit rows now have an in-process forwarder that signs each pending outbox row and relays it to canonical_ingest.
- The control-plane forwarder uses per-event signatures as canonical truth; batch signatures remain empty, matching the in-cluster mTLS producer convention used by tokenizer/stats paths.
- canonical_ingest now rejects missing `producer_id`, missing `schema_bundle`, and `ROUTE_UNSPECIFIED` before storage access, so broken producers fail fast without DB side effects.
- Helm and compose both wire the control-plane forwarder through mTLS, the shared schema bundle, and a dedicated control-plane Ed25519 key.

## Locked Decisions

- Control-plane handlers continue writing `control_plane_audit_outbox` inside the mutation transaction. Forwarding happens asynchronously from pending rows so canonical_ingest downtime preserves audit events.
- `forwarded_at IS NULL` is the durable retry cursor. A successful AppendEvents response updates `cloudevent_payload_signature_hex` and `forwarded_at` in the same DB transaction that locked the row.
- Forwarded plugin lifecycle events use `route = Observability` and retain the `spendguard.audit.plugin_*.v1alpha1` event type family.
- The signer source is `SPENDGUARD_CONTROL_PLANE_*`, matching the shared `spendguard-signing` local/KMS/disabled environment contract.
- The Helm Secret contract now includes `control-plane.{crt,key}` in the TLS Secret and `control-plane.pem` in the signing Secret.

## Verification

- `cargo test --manifest-path services/control_plane/Cargo.toml audit_forwarder -- --nocapture`
- `cargo test --manifest-path services/canonical_ingest/Cargo.toml append_events_rejects -- --nocapture`
- `helm template charts/spendguard --set chart.profile=demo`
- `helm template charts/spendguard -f docs/reviews/hardening/HARDEN_02/kind-production-values.example.yaml`

## Review Focus

- The forwarder must not weaken canonical_ingest envelope validation.
- Production deployments must provide signer and canonical_ingest configuration when the control-plane audit forwarder is enabled.
- Duplicate sends rely on HARDEN_05 replay dedup and the outbox row lock/forwarded marker.
