-- D09 SLICE 5 — Lua fallback plugin schema.
--
-- This file is the Kong control-plane wire contract for the Lua port
-- of the SpendGuard plugin. It mirrors `plugins/kong/spendguard-go/
-- config.go` field-for-field so the two distributions accept the
-- same `KongPlugin` CRD without surprises.
--
-- Per `docs/specs/coverage/D09_kong_ai_gateway/design.md` §3.2 the
-- Lua port is **experimental** and only covers `access` + `body_filter`
-- via lua-resty-http against the same HTTP companion endpoints. The
-- Go plugin is the supported production path; operators reach for the
-- Lua port when they cannot run a `go-plugin-server` subprocess
-- alongside the Kong worker (constrained OpenResty images, OSS Kong
-- 3.0–3.5 deployments where the plugin-server protocol is locked
-- behind Enterprise, etc).
--
-- Schema invariants per review-standards.md §1.6 + §3.4:
--   * fail-closed default: `fail_open = false`.
--   * `timeout_ms` defaults to 500, matching the Go default.
--   * `tenant_id` is mandatory in production. The schema marks it
--     required so Kong's declarative-config validation rejects an
--     install missing it.
--   * `sidecar_url` is mandatory and MUST start with `https://`.
--     The handler re-asserts this at plugin-init time (defense in
--     depth — Kong's `match` validator does not run on every
--     request, the handler does).
--
-- Compatibility:
--   * Kong 3.0+ (PRIORITY semantics + `kong.ctx.shared` + go-pdk
--     interop API). The plugin priority MUST equal the Go plugin's
--     priority (950) so installing both side-by-side in a debugging
--     deployment yields deterministic ordering.

local typedefs = require "kong.db.schema.typedefs"

return {
  name = "spendguard",
  fields = {
    { consumer = typedefs.no_consumer },
    { protocols = typedefs.protocols_http },
    {
      config = {
        type = "record",
        fields = {
          -- ── Sidecar transport ────────────────────────────────────
          {
            sidecar_url = {
              type = "string",
              required = true,
              -- design §3.1: mTLS-only, refuse plaintext URLs.
              match = "^https://",
              referenceable = true,
            },
          },
          {
            sidecar_ca_pem = {
              type = "string",
              required = false,
              referenceable = true,
            },
          },
          {
            sidecar_ca_file = {
              type = "string",
              required = false,
            },
          },
          -- ── Workload identity (client mTLS) ─────────────────────
          {
            client_cert_pem = {
              type = "string",
              required = false,
              referenceable = true,
            },
          },
          {
            client_cert_file = {
              type = "string",
              required = false,
            },
          },
          {
            client_key_pem = {
              type = "string",
              required = false,
              referenceable = true,
              encrypted = true,
            },
          },
          {
            client_key_file = {
              type = "string",
              required = false,
            },
          },
          -- ── Tenant assertion ────────────────────────────────────
          {
            tenant_id = {
              type = "string",
              required = true,
              -- UUID match identical to the Go plugin's runtime
              -- validation. Operator-supplied tenant IDs that are
              -- not UUIDs fail at schema-load time.
              match = "^[0-9a-fA-F]+%-[0-9a-fA-F]+%-[0-9a-fA-F]+%-[0-9a-fA-F]+%-[0-9a-fA-F]+$",
            },
          },
          -- ── Fail-closed default (review-standards §1.6) ─────────
          {
            fail_open = {
              type = "boolean",
              default = false,
              required = true,
            },
          },
          -- ── Per-request budget ──────────────────────────────────
          {
            timeout_ms = {
              type = "integer",
              default = 500,
              between = { 50, 30000 },
              required = true,
            },
          },
          -- ── Optional explicit budget binding (multi-budget tenants) ─
          {
            budget_id = {
              type = "string",
              required = false,
              match = "^[0-9a-fA-F]+%-[0-9a-fA-F]+%-[0-9a-fA-F]+%-[0-9a-fA-F]+%-[0-9a-fA-F]+$",
            },
          },
          {
            prompt_class = {
              type = "string",
              default = "general",
              required = true,
            },
          },
        },
        -- Cross-field validation. Lua schema runs this at install +
        -- declarative-reload time. The Go plugin enforces equivalents
        -- inside `newSidecarClient(cfg)`.
        entity_checks = {
          {
            at_least_one_of = { "sidecar_ca_pem", "sidecar_ca_file" },
          },
          {
            at_least_one_of = { "client_cert_pem", "client_cert_file" },
          },
          {
            at_least_one_of = { "client_key_pem", "client_key_file" },
          },
          -- Cannot supply both inline + file forms of the same
          -- material — operator intent is ambiguous.
          {
            mutually_exclusive = { "sidecar_ca_pem", "sidecar_ca_file" },
          },
          {
            mutually_exclusive = { "client_cert_pem", "client_cert_file" },
          },
          {
            mutually_exclusive = { "client_key_pem", "client_key_file" },
          },
        },
      },
    },
  },
}
