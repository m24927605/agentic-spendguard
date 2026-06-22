---
title: "Control plane API"
---

`http://<host>:8091/`(POC)——用于租户与预算开通的 REST API。

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

create-tenant 的响应里带有一个 `sidecar_config_env` 块——把这些环境变量直接塞进 sidecar 部署即可。

Auth:POC 阶段用单一 admin bearer token。生产环境映射到 Entra ID 的 admin scope,并带有按租户划分的资源权限。

来源:`services/control_plane/`。
