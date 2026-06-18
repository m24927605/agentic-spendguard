# HARDEN 06 Audit Envelope And Signing Notes

## Scope

- Tokenizer drift alert sink already builds a full canonical_ingest `AppendEventsRequest` envelope with `producer_id`, `schema_bundle`, and `route = Observability`.
- Control-plane plugin lifecycle audit rows now have an in-process forwarder that signs each pending outbox row and relays it to canonical_ingest.
- The control-plane forwarder uses per-event signatures as canonical truth; batch signatures remain empty, matching the in-cluster mTLS producer convention used by tokenizer/stats paths.
- canonical_ingest now rejects missing `producer_id`, missing `schema_bundle`, and `ROUTE_UNSPECIFIED` before storage access, so broken producers fail fast without DB side effects.
- Helm and compose both wire the control-plane forwarder through mTLS, the shared schema bundle, and a dedicated control-plane Ed25519 key.
- Production control-plane audit forwarding uses `SPENDGUARD_CONTROL_PLANE_AUDIT_FORWARDER_DATABASE_URL`, which must point at a login role granted the dedicated `control_plane_audit_forwarder_role`; this keeps request-serving RLS separate from outbox forwarding.
- Demo Postgres initialization mounts and applies `services/control_plane/migrations` so a fresh compose stack creates the plugin registry, audit outbox, and forwarder RLS role before control-plane boots.

## Locked Decisions

- Control-plane handlers continue writing `control_plane_audit_outbox` inside the mutation transaction. Forwarding happens asynchronously from pending rows so canonical_ingest downtime preserves audit events.
- `forwarded_at IS NULL` is the durable retry cursor. A successful AppendEvents response updates `cloudevent_payload_signature_hex` and `forwarded_at` in the same DB transaction that locked the row.
- Forwarded plugin lifecycle events use `route = Observability` and retain the `spendguard.audit.plugin_*.v1alpha1` event type family.
- The signer source is `SPENDGUARD_CONTROL_PLANE_*`, matching the shared `spendguard-signing` local/KMS/disabled environment contract.
- The Helm Secret contract now includes `control-plane.{crt,key}` in the TLS Secret and `control-plane.pem` in the signing Secret.
- The Postgres Secret contract now includes `control-plane-audit-forwarder-url` for production. The database role grants are explicit RLS policies on `control_plane_audit_outbox`; no `BYPASSRLS` role is introduced.

## Verification

- `cargo test --manifest-path services/control_plane/Cargo.toml audit_forwarder -- --nocapture`
- `cargo test --manifest-path services/canonical_ingest/Cargo.toml append_events_rejects -- --nocapture`
- `helm template charts/spendguard --set chart.profile=demo`
- `helm template charts/spendguard -f docs/internal/reviews/hardening/HARDEN_02/kind-production-values.example.yaml`
- `rg -n "control_plane_audit_forwarder_role|control_plane_audit_outbox_forwarder" services/control_plane/migrations/0005_control_plane_audit_forwarder_role.sql`
- `docker compose -f deploy/demo/compose.yaml up -d --build postgres pki-init bundles-init canonical-seed-init canonical-ingest control-plane`
- `curl -sS -i -X POST http://localhost:8091/v1/predictor/plugins ...` returned `200 OK` for tenant `00000000-0000-4000-8000-000000000001`.
- `control-plane` logged `control-plane audit outbox forwarded` with `forwarded=1`; `canonical-ingest` logged append with `producer_id=control-plane:demo` and `route=Observability`.
- `canonical_events` contained `spendguard.audit.plugin_registered.v1alpha1` with `storage_class=immutable_audit_log`, `producer_id=control-plane:demo`, 128 hex chars of signature material, and forwarded JSON data containing `subject` plus `actor_subject`.

## Review Focus

- The forwarder must not weaken canonical_ingest envelope validation.
- Production deployments must provide signer and canonical_ingest configuration when the control-plane audit forwarder is enabled.
- Production deployments must provide a distinct audit-forwarder database URL so RLS does not hide pending outbox rows from the worker.
- Duplicate sends rely on HARDEN_05 replay dedup and the outbox row lock/forwarded marker.
