# SpendGuard + Kong AI Gateway — composite example

End-to-end manifests that wire the SpendGuard plugin (Go or Lua) into
a Kong AI Gateway install. The plugin runs in Kong's `access` +
`body_filter` phases and gates upstream LLM API calls before any
provider receives them.

Spec: [`docs/specs/coverage/D09_kong_ai_gateway/design.md`](../../docs/specs/coverage/D09_kong_ai_gateway/design.md)

## Topology

```
client
   |
   v
Kong DataPlane :8000
   |    spendguard plugin (Go .so / Lua plugin-server)
   |       |
   |       v
   |    HTTPS+mTLS to spendguard-kong-companion:8443
   |       |
   |       +-> /v1/tokenize   /v1/decision   /v1/trace
   |
   +-> ai-proxy plugin -> OpenAI / Anthropic
```

## Files

| File | Purpose |
|------|---------|
| `kong-plugin-crd.yaml` | Reference `KongPlugin` CRDs for the Go (production) and Lua (experimental) distributions. |
| `kong-conf.yaml` | Reference `kong.conf` snippet for declarative-config installs (DB-less Kong). |
| `go-build.sh` | Build the Go plugin `.so` and bake it into a Kong DataPlane image (no Konnect, no Konnect control plane). |
| `README.md` | This file. |

## Bring-up sequence

```bash
# 1. Install SpendGuard with the Kong companion enabled.
helm install spendguard ./charts/spendguard \
  --set kongPlugin.enabled=true \
  --set kongPlugin.tenantId=00000000-0000-4000-8000-000000000001 \
  --set kongPlugin.svid.secretName=kong-companion-svid

# 2. Apply the reference KongPlugin CRD (pick Go OR Lua).
kubectl apply -f examples/kong-gateway-composite/kong-plugin-crd.yaml

# 3. Bind the plugin to the target Service via annotation:
kubectl annotate svc my-llm-upstream \
  konghq.com/plugins=spendguard-go --overwrite

# 4. Smoke-test through the gateway.
curl https://kong-proxy/v1/chat/completions \
  -H 'Authorization: Bearer xxx' \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}'
```

## Production checklist

- [ ] `chart.profile=production` set — fail-closed gates active.
- [ ] `kongPlugin.tenantId` is a real UUID, not a demo seed.
- [ ] `kongPlugin.svid.secretName` points at a cert-manager-issued
      Secret holding tls.crt + tls.key + ca.crt.
- [ ] Kong DataPlane pods labeled `app.kubernetes.io/name=kong` so
      the chart's NetworkPolicy ingress rule matches them.
- [ ] `fail_open: false` (default) — only flip after operator review.
- [ ] Service annotation `konghq.com/plugins` references exactly ONE
      distribution (Go OR Lua, not both).
- [ ] `request_buffering: true` on routes the plugin gates (design §3.3).

## Lua fallback

The `spendguard-lua` plugin is published as `luarocks install
spendguard` (rockspec at `plugins/kong/spendguard-lua/spendguard-1.0.0-1.rockspec`).
The Lua port is **experimental** per design §3.2; it has no conformance
gate. Use it only on Kong OSS 3.0–3.5 deployments that cannot run a
`go-plugin-server` subprocess.

## Related

- [`plugins/kong/spendguard-go/README.md`](../../plugins/kong/spendguard-go/README.md) — Go plugin
- [`plugins/kong/spendguard-lua/README.md`](../../plugins/kong/spendguard-lua/README.md) — Lua port (experimental)
- [`docs/specs/coverage/D09_kong_ai_gateway/design.md`](../../docs/specs/coverage/D09_kong_ai_gateway/design.md) — full spec
