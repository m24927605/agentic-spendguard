-- D09 SLICE 5 — schema invariants.
--
-- These specs run via `busted spec/` against the Kong plugin schema
-- typedefs. They cover the fail-closed defaults + cross-field
-- validation invariants from `docs/specs/coverage/D09_kong_ai_gateway/
-- review-standards.md` §1.6 + §3.4.

local schema_def = require "kong.plugins.spendguard.schema"
local v = require("spec.helpers").validate_plugin_config_schema

describe("spendguard schema", function()
  it("accepts a full valid PEM-inline config", function()
    local ok, err = v({
      sidecar_url = "https://spendguard-companion.svc:8443",
      sidecar_ca_pem = "-----BEGIN CERTIFICATE-----\nfake\n-----END CERTIFICATE-----\n",
      client_cert_pem = "-----BEGIN CERTIFICATE-----\nfake\n-----END CERTIFICATE-----\n",
      client_key_pem = "-----BEGIN PRIVATE KEY-----\nfake\n-----END PRIVATE KEY-----\n",
      tenant_id = "00000000-0000-4000-8000-000000000001",
    }, schema_def)
    assert.is_nil(err)
    assert.is_truthy(ok)
  end)

  it("rejects a plaintext sidecar_url", function()
    -- design §3.1: mTLS-only, refuse http://.
    local _, err = v({
      sidecar_url = "http://spendguard-companion.svc:8443",
      sidecar_ca_pem = "FAKE",
      client_cert_pem = "FAKE",
      client_key_pem = "FAKE",
      tenant_id = "00000000-0000-4000-8000-000000000001",
    }, schema_def)
    assert.is_not_nil(err)
  end)

  it("rejects a non-UUID tenant_id", function()
    local _, err = v({
      sidecar_url = "https://spendguard-companion.svc:8443",
      sidecar_ca_pem = "FAKE",
      client_cert_pem = "FAKE",
      client_key_pem = "FAKE",
      tenant_id = "tenant-name",
    }, schema_def)
    assert.is_not_nil(err)
  end)

  it("defaults fail_open=false (review-standards §1.6)", function()
    local ok, _ = v({
      sidecar_url = "https://spendguard-companion.svc:8443",
      sidecar_ca_pem = "FAKE",
      client_cert_pem = "FAKE",
      client_key_pem = "FAKE",
      tenant_id = "00000000-0000-4000-8000-000000000001",
    }, schema_def)
    assert.is_truthy(ok)
    assert.is_false(ok.config.fail_open)
  end)

  it("defaults timeout_ms=500", function()
    local ok, _ = v({
      sidecar_url = "https://spendguard-companion.svc:8443",
      sidecar_ca_pem = "FAKE",
      client_cert_pem = "FAKE",
      client_key_pem = "FAKE",
      tenant_id = "00000000-0000-4000-8000-000000000001",
    }, schema_def)
    assert.is_truthy(ok)
    assert.equals(500, ok.config.timeout_ms)
  end)

  it("rejects when neither inline nor file PEM is supplied", function()
    -- at_least_one_of cross-field check from schema.lua.
    local _, err = v({
      sidecar_url = "https://spendguard-companion.svc:8443",
      tenant_id = "00000000-0000-4000-8000-000000000001",
    }, schema_def)
    assert.is_not_nil(err)
  end)

  it("rejects supplying both inline pem and file path", function()
    -- mutually_exclusive cross-field check.
    local _, err = v({
      sidecar_url = "https://spendguard-companion.svc:8443",
      sidecar_ca_pem = "FAKE",
      sidecar_ca_file = "/etc/sg/ca.pem",
      client_cert_pem = "FAKE",
      client_key_pem = "FAKE",
      tenant_id = "00000000-0000-4000-8000-000000000001",
    }, schema_def)
    assert.is_not_nil(err)
  end)
end)
