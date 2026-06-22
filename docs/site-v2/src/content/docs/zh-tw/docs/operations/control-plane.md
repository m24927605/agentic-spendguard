---
title: "Control plane API"
---

`http://<host>:8091/`(POC)— 負責租戶與 budget 開通的 REST API。

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

建立租戶的 response 會帶一段 `sidecar_config_env` 區塊 —
直接把那些環境變數塞進 sidecar 部署即可。

Auth:POC 階段只用單一 admin bearer token。正式環境則對應到 Entra ID
的 admin scope,並搭配 per-tenant 的資源權限。

來源:`services/control_plane/`。
