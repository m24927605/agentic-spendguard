# D09 — Kong AI Gateway Plugin — Implementation

**Companion to:** [`design.md`](design.md)
**Code layout owner:** Backend Architect

---

## §1. Module layout

```
plugins/kong/
├── spendguard-go/                            # SLICE 2-4 primary
│   ├── go.mod
│   ├── go.sum
│   ├── main.go                               # plugin-server entry, registers Config + Access + BodyFilter
│   ├── config.go                             # plugin Config struct (sidecar URL, SVID paths, fail_open)
│   ├── access.go                             # SLICE 3 reserve flow
│   ├── body_filter.go                        # SLICE 4 commit flow
│   ├── sidecar_client.go                     # HTTP+mTLS client of /v1/tokenize and /v1/decision
│   ├── provider_route.go                     # OpenAI/Anthropic body shape detection + ProviderKind
│   ├── tls.go                                # rustls-compatible cert load (Go crypto/tls + SVID PEM)
│   ├── metrics.go                            # Prometheus /metrics (port-forwarded via plugin-server)
│   └── internal/
│       ├── proto/                            # generated from sidecar adapter common.v1 (for trace event shapes)
│       └── testutil/                         # sidecar stub for unit tests
├── spendguard-lua/                           # SLICE 5 fallback
│   ├── kong/
│   │   └── plugins/
│   │       └── spendguard/
│   │           ├── handler.lua               # access + body_filter
│   │           ├── schema.lua                # plugin schema (sidecar_url, fail_open)
│   │           └── sidecar_client.lua        # lua-resty-http HTTPS client
│   ├── spendguard-1.0.0-1.rockspec
│   └── README.md                             # "experimental" disclaimer
└── README.md                                 # plugin-distribution index

services/sidecar/                             # SLICE 1
└── src/
    └── server/
        └── http_companion.rs                 # axum 0.7 listener; POST /v1/tokenize, POST /v1/decision; mTLS via rustls
        # wire into sidecar/src/main.rs alongside adapter_uds

charts/spendguard/templates/                  # SLICE 6
├── kong_plugin_sidecar.yaml                  # SpendGuard side: sidecar Deployment with HTTP companion enabled
├── kong_plugin_networkpolicy.yaml            # Ingress from app.kubernetes.io/name=kong namespace selector
└── kong_plugin_servicemonitor.yaml           # Prometheus ServiceMonitor for plugin /metrics

examples/kong-gateway-composite/              # SLICE 6
├── kong-plugin-crd.yaml                      # KongPlugin + KongClusterPlugin reference
├── kong-conf.yaml                            # kong.conf snippet for non-CRD installs
├── go-build.sh                               # build .so + push image (Kong DataPlane custom image recipe)
└── README.md

deploy/demo/                                  # SLICE 7
├── Makefile                                  # +DEMO_MODE=kong_gateway_real target
├── compose.kong.yaml                         # kong + postgres + spendguard sidecar + real OpenAI
├── kong_demo/
│   ├── kong.yml                              # declarative DBless config: ai-proxy + spendguard plugins
│   ├── plugin_bin/                           # mounted into Kong container; holds spendguard.so
│   └── client.sh                             # curl-driven smoke (ALLOW + DENY + COMMIT verify)
└── verify_step_kong_gateway_real.sql         # SLICE 7 audit-chain assertion

docs/site/docs/integrations/                  # SLICE 7
└── kong-ai-gateway.md                        # public docs page

crates/spendguard-provider-routing/           # if not already shipped by D01 SLICE 1
└── src/lib.rs                                # ProviderKind + resolve_route + body-shape detection
```

## §2. Go plugin `plugins/kong/spendguard-go/go.mod`

```go
module github.com/spendguard/kong-plugin-spendguard

go 1.22

require (
    github.com/Kong/go-pdk v0.11.0
    github.com/Kong/go-plugin-server v0.6.0
    github.com/prometheus/client_golang v1.20.0
    google.golang.org/protobuf v1.34.0
)
```

## §3. Go plugin entry `plugins/kong/spendguard-go/main.go` (skeleton)

```go
package main

import (
    "github.com/Kong/go-pdk"
    "github.com/Kong/go-pdk/server"
)

type Config struct {
    SidecarURL    string `json:"sidecar_url"`
    SidecarCAPem  string `json:"sidecar_ca_pem"`
    ClientCertPem string `json:"client_cert_pem"`
    ClientKeyPem  string `json:"client_key_pem"`
    TenantID      string `json:"tenant_id"`
    FailOpen      bool   `json:"fail_open"`
    TimeoutMS     int    `json:"timeout_ms"`
}

func New() interface{} { return &Config{TimeoutMS: 500} }

func main() {
    _ = server.StartServer(New, "1.0.0", 0)
}

func (c *Config) Access(kong *pdk.PDK) {
    // SLICE 3: parse body → tokenize → decision → ALLOW/DENY/DEGRADE
    runAccess(kong, c)
}

func (c *Config) BodyFilter(kong *pdk.PDK) {
    // SLICE 4: buffer response → parse usage → EmitTraceEvents
    runBodyFilter(kong, c)
}
```

## §4. Sidecar HTTP companion `services/sidecar/src/server/http_companion.rs` (skeleton — SLICE 1)

```rust
//! HTTP companion listener for out-of-process plugins that cannot speak
//! the adapter UDS gRPC contract (Kong Go plugin, Coze plugin daemon,
//! Botpress integration). Mirrors the gRPC adapter handler semantics
//! 1:1 but speaks JSON over HTTP/1.1+mTLS.
//!
//! Endpoints:
//!   POST /v1/tokenize  → thin wrapper over spendguard_tokenizer::encode
//!   POST /v1/decision  → thin wrapper over decision::transaction::run
//!   POST /v1/trace     → thin wrapper over EmitTraceEvents single-event
//!
//! Transport: HTTPS only; mTLS required; loopback bind by default.
//! Auth: workload SVID cert SAN URI matched against expected_tenant_id.

use axum::{routing::post, Router, Json};
use crate::decision::transaction;
use crate::domain::state::SidecarState;

pub fn router(state: SidecarState) -> Router {
    Router::new()
        .route("/v1/tokenize", post(tokenize_handler))
        .route("/v1/decision", post(decision_handler))
        .route("/v1/trace", post(trace_handler))
        .with_state(state)
}

async fn tokenize_handler(/* ... */) -> Json<TokenizeResponse> { /* ... */ }
async fn decision_handler(/* ... */) -> Json<DecisionResponse> { /* ... */ }
async fn trace_handler(/* ... */) -> Json<TraceAckResponse> { /* ... */ }
```

## §5. Access flow `plugins/kong/spendguard-go/access.go` (skeleton — SLICE 3)

```go
package main

import (
    "encoding/json"
    "github.com/Kong/go-pdk"
)

func runAccess(kong *pdk.PDK, cfg *Config) {
    body, err := kong.Request.GetRawBody()
    if err != nil { failOpenOrDeny(kong, cfg, 502, "body read failed"); return }

    provider, err := detectProvider(kong, body)
    if err != nil { failOpenOrDeny(kong, cfg, 400, "unrecognized provider shape"); return }

    client, err := newSidecarClient(cfg)
    if err != nil { failOpenOrDeny(kong, cfg, 503, "sidecar client init"); return }

    tokenResp, err := client.Tokenize(provider, body)
    if err != nil { failOpenOrDeny(kong, cfg, 503, "tokenize unreachable"); return }

    decision, err := client.Decision(cfg.TenantID, provider, tokenResp.InputTokens)
    if err != nil { failOpenOrDeny(kong, cfg, 503, "decision unreachable"); return }

    switch decision.Decision {
    case "ALLOW":
        kong.Ctx.SetShared("spendguard_reservation_id", decision.ReservationID)
        kong.Ctx.SetShared("spendguard_provider", string(provider))
    case "DENY":
        _ = kong.Response.Exit(429, []byte(`{"error":"budget exceeded","code":"SPENDGUARD_DENY"}`), nil)
    case "DEGRADE":
        if cfg.FailOpen {
            kong.Ctx.SetShared("spendguard_degraded", "1")
        } else {
            _ = kong.Response.Exit(503, []byte(`{"error":"guardrail degraded","code":"SPENDGUARD_DEGRADE"}`), nil)
        }
    }
}

func failOpenOrDeny(kong *pdk.PDK, cfg *Config, status int, msg string) {
    if cfg.FailOpen {
        kong.Log.Warn("spendguard degraded fail-open: " + msg)
        return
    }
    _ = kong.Response.Exit(status, []byte(`{"error":"`+msg+`","code":"SPENDGUARD_FAIL_CLOSED"}`), nil)
}
```

## §6. BodyFilter flow `plugins/kong/spendguard-go/body_filter.go` (skeleton — SLICE 4)

```go
package main

import "github.com/Kong/go-pdk"

func runBodyFilter(kong *pdk.PDK, cfg *Config) {
    reservationID, err := kong.Ctx.GetSharedString("spendguard_reservation_id")
    if err != nil || reservationID == "" { return }  // upstream short-circuited or never reserved

    chunk, err := kong.ServiceResponse.GetRawBody()
    if err != nil { return }

    // Buffer until end-of-body. Kong's body_filter is called repeatedly;
    // we accumulate into kong.ctx.shared until the final empty-chunk marker.
    if !isFinalChunk(chunk) {
        appendBufferedChunk(kong, chunk)
        return
    }
    full := readBufferedBody(kong)

    provider, _ := kong.Ctx.GetSharedString("spendguard_provider")
    usage, err := parseProviderUsage(provider, full)
    if err != nil {
        client, _ := newSidecarClient(cfg)
        _ = client.Trace(reservationID, "RUN_ABORTED", nil)
        return
    }
    client, err := newSidecarClient(cfg)
    if err != nil { return }
    _ = client.Trace(reservationID, "LLM_CALL_POST.SUCCESS", &usage)
}
```

## §7. Lua fallback `plugins/kong/spendguard-lua/kong/plugins/spendguard/handler.lua` (skeleton — SLICE 5)

```lua
local http = require "resty.http"
local cjson = require "cjson.safe"

local SpendGuardHandler = { PRIORITY = 950, VERSION = "1.0.0" }

function SpendGuardHandler:access(conf)
  local body = kong.request.get_raw_body() or ""
  local httpc = http.new()
  httpc:set_timeout(conf.timeout_ms or 500)
  local ok, err = httpc:connect{
    scheme = "https", host = conf.sidecar_host, port = conf.sidecar_port,
    ssl_verify = true, ssl_client_cert = conf.client_cert, ssl_client_priv_key = conf.client_key,
  }
  if not ok then return self:fail(conf, 503, "sidecar connect: " .. tostring(err)) end
  -- POST /v1/tokenize + /v1/decision; map decision to kong.response.exit/ctx
  -- (full impl in SLICE 5)
end

function SpendGuardHandler:body_filter(conf)
  -- accumulate ngx.arg[1]; on final chunk POST /v1/trace with LLM_CALL_POST.SUCCESS
end

return SpendGuardHandler
```

## §8. Helm chart `charts/spendguard/templates/kong_plugin_sidecar.yaml` (skeleton — SLICE 6)

```yaml
{{- if .Values.kongPlugin.enabled }}
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "spendguard.fullname" . }}-kong-companion
  labels: {{- include "spendguard.labels" . | nindent 4 }}
spec:
  replicas: {{ .Values.kongPlugin.replicas | default 2 }}
  template:
    spec:
      containers:
        - name: sidecar
          image: "{{ .Values.image.repository }}:{{ .Values.image.tag }}"
          args: ["--http-companion-bind", "0.0.0.0:8443", "--http-companion-svid", "/run/svid/cert.pem"]
          ports:
            - name: http-companion
              containerPort: 8443
            - name: metrics
              containerPort: 9090
          volumeMounts:
            - name: svid
              mountPath: /run/svid
              readOnly: true
      volumes:
        - name: svid
          csi:
            driver: csi.cert-manager.io
            volumeAttributes:
              csi.cert-manager.io/issuer-name: {{ .Values.kongPlugin.svidIssuer }}
              csi.cert-manager.io/common-name: "spendguard-kong-companion.{{ .Release.Namespace }}.svc"
{{- end }}
```

## §9. Demo topology `deploy/demo/compose.kong.yaml` (skeleton — SLICE 7)

```yaml
services:
  postgres:
    image: postgres:16
    # ... ledger DB
  sidecar:
    image: spendguard-sidecar:dev
    command: ["--http-companion-bind", "127.0.0.1:8443", "--uds-path", "/run/sg/uds.sock"]
    network_mode: "service:kong"   # share network ns so 127.0.0.1 is mutual
  kong:
    image: spendguard-kong-demo:dev    # base kong:3.7 + plugin .so + go-plugin-server
    environment:
      - KONG_PLUGINS=bundled,spendguard
      - KONG_PLUGINSERVER_NAMES=spendguard
      - KONG_PLUGINSERVER_SPENDGUARD_START_CMD=/usr/local/bin/spendguard
      - KONG_PLUGINSERVER_SPENDGUARD_QUERY_CMD=/usr/local/bin/spendguard -dump
      - KONG_DATABASE=off
      - KONG_DECLARATIVE_CONFIG=/kong/declarative/kong.yml
    ports: ["8000:8000"]
  client:
    image: curlimages/curl:8.10.0
    command: ["/bin/sh", "/demo/client.sh"]
    environment:
      - OPENAI_API_KEY=${OPENAI_API_KEY}
```

## §10. Build automation

```makefile
# Top-level Makefile additions
build-kong-plugin:
	cd plugins/kong/spendguard-go && go build -o ../../../target/kong/spendguard ./...

publish-kong-plugin-image:
	docker build -t spendguard-kong-demo:dev -f deploy/demo/kong_demo/Dockerfile .
```

## §11. Wire diagram

```
┌─────────┐  HTTP  ┌──────────────────┐  HTTPS+mTLS  ┌─────────────────┐
│ Client  │ ─────▶ │ Kong DataPlane   │ ───────────▶ │ SpendGuard      │
│  curl   │        │  + spendguard.so │              │ sidecar         │
└─────────┘        │  (Go plugin)     │              │ /v1/tokenize    │
                   │                  │              │ /v1/decision    │
                   │                  │              │ /v1/trace       │
                   │                  │              └─────────────────┘
                   │                  │  HTTPS       ┌─────────────────┐
                   │                  │ ───────────▶ │ OpenAI / Anth.  │
                   └──────────────────┘              └─────────────────┘
```

---

*Code skeletons in §3, §4, §5, §6, §7, §8 are SLICE-defining. Slice impl PRs do not refactor these signatures; deviations require design.md amendment.*
