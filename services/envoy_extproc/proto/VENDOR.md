# Vendored Envoy ExternalProcessor proto subset

Source-of-truth: [envoyproxy/envoy](https://github.com/envoyproxy/envoy) at
the `main` branch (Envoy 1.30+ / Envoy AI Gateway v0.6 wire-compatible).

| Vendored file | Upstream path |
|---|---|
| `envoy/service/ext_proc/v3/external_processor.proto` | `api/envoy/service/ext_proc/v3/external_processor.proto` |
| `envoy/config/core/v3/base.proto` | `api/envoy/config/core/v3/base.proto` (subset: `HeaderMap`, `HeaderValue`, `HeaderValueOption`, `Metadata`) |
| `envoy/type/v3/http_status.proto` | `api/envoy/type/v3/http_status.proto` (full `StatusCode` enum) |
| `envoy/extensions/filters/http/ext_proc/v3/processing_mode.proto` | `api/envoy/extensions/filters/http/ext_proc/v3/processing_mode.proto` (subset: `HeaderSendMode`, `BodySendMode`, `ProcessingMode`) |

## What was stripped, and why

The upstream tree pulls in `xds.annotations.v3`, `udpa.annotations`,
`validate`, and `envoy.annotations`. These are:

* `(validate.rules).*` — server-side field validators (only affect
  generated validators, not the wire format).
* `(udpa.annotations.versioning).*` — migration metadata across api
  versions, not on the wire.
* `(envoy.annotations.deprecated_at_minor_version)` — deprecation
  metadata.
* `(xds.annotations.v3.field_status).work_in_progress` — stability
  hints.

Removing these leaves the wire-compatible message shapes intact (proto3
field numbers + types match upstream byte-for-byte). The vendored stubs
are a **client/server protocol** subset, suitable for a SpendGuard
ExtProc adapter that the Envoy AI Gateway can dial via the standard
`ExternalProcessor` filter.

## Refresh procedure

1. Fetch upstream:
   ```sh
   for p in service/ext_proc/v3/external_processor.proto \
            config/core/v3/base.proto \
            type/v3/http_status.proto \
            extensions/filters/http/ext_proc/v3/processing_mode.proto; do
     curl -s -o /tmp/upstream.proto \
       "https://raw.githubusercontent.com/envoyproxy/envoy/main/api/envoy/$p"
     diff -u "/tmp/upstream.proto" "services/envoy_extproc/proto/envoy/$p" || true
   done
   ```
2. If any field number / type changed in upstream: refresh the local
   stub, re-run `cargo build -p spendguard-envoy-extproc`, and bump the
   adapter version.
3. If upstream renamed a message (rare; the v3 surface is stable since
   Envoy 1.27): file a follow-up issue — D01 spec §3.5 pins v3.
