# Control plane API

`http://<host>:8091/` (POC) — REST API for tenant + budget provisioning.

```bash
# Create tenant
curl -X POST http://localhost:8091/v1/tenants \
  -H 'Authorization: Bearer <admin-token>' \
  -H 'Content-Type: application/json' \
  -d '{"name": "acme-corp", "opening_deposit_atomic": "1000"}'

# Get tenant overview
curl http://localhost:8091/v1/tenants/<id> \
  -H 'Authorization: Bearer <admin-token>'

# Tombstone tenant (audit chain remains immutable)
curl -X DELETE http://localhost:8091/v1/tenants/<id> \
  -H 'Authorization: Bearer <admin-token>'
```

The create-tenant response includes a `sidecar_config_env` block —
drop those env vars straight into a sidecar deployment.

Auth: single admin bearer token for POC. Production maps to Entra ID
admin scope with per-tenant resource permissions.

Source: `services/control_plane/`.
