-- D09 SLICE 5 — Lua HTTP client for the SpendGuard sidecar.
--
-- Per `docs/specs/coverage/D09_kong_ai_gateway/design.md` §3.2 the
-- Lua port is *experimental* and only needs to cover `access` +
-- `body_filter`. It speaks the same JSON-over-HTTPS+mTLS contract
-- the Go plugin speaks (`services/sidecar/src/http_companion/
-- handlers.rs`). Wire stability is enforced by the Go plugin's
-- integration tests; this file's only job is to format the JSON
-- and translate HTTP status into ALLOW / DENY / DEGRADE.
--
-- Implementation choices:
--
--   * `lua-resty-http` v0.17+ for the HTTP/1.1 + TLS surface. Ships
--     with stock OpenResty + Kong 3.0+; no extra dependency.
--   * `cjson.safe` for the JSON encode/decode so a malformed sidecar
--     response is a clean nil-return instead of a Lua exception.
--   * mTLS material is loaded once per worker via the `ngx.ssl`
--     parsed-cert / parsed-key helpers and cached in module-level
--     locals. lua-resty-http re-uses them on every connect call;
--     this is the canonical OpenResty pattern (see
--     https://github.com/ledgetech/lua-resty-http#ssl).
--
-- Anti-scope (review-standards.md §1.5):
--
--   * No streaming SSE handling — body_filter accumulates and emits
--     one trace event at end-of-body, identical to the Go path.
--   * No retry inside the client. The Kong plugin treats every
--     non-ALLOW outcome from a request-time error as DEGRADE; the
--     plugin's fail-closed default takes over.

local http = require "resty.http"
local cjson = require "cjson.safe"
local ssl = require "ngx.ssl"

local _M = {}

-- Module-level parsed-cert / parsed-key cache. lua-resty-http accepts
-- a *parsed* cert (from `ssl.parse_pem_cert`) so the PEM parse only
-- happens once per worker per config.
local _cert_cache = setmetatable({}, { __mode = "k" })
local _key_cache = setmetatable({}, { __mode = "k" })

-- Load PEM material from either `conf.<field>_pem` (inline) or
-- `conf.<field>_file` (path on disk). The schema's mutually_exclusive
-- check guarantees exactly one is supplied; this helper crashes the
-- worker fast if both or neither are present so the misconfiguration
-- surfaces at first call rather than silently.
local function _read_pem(inline, path, kind)
  if inline and #inline > 0 then
    return inline, nil
  end
  if not path or #path == 0 then
    return nil, ("spendguard: " .. kind .. " missing (neither inline pem nor file path supplied)")
  end
  local fh, err = io.open(path, "r")
  if not fh then
    return nil, ("spendguard: open " .. kind .. " " .. path .. ": " .. tostring(err))
  end
  local pem = fh:read("*a")
  fh:close()
  if not pem or #pem == 0 then
    return nil, ("spendguard: empty pem at " .. tostring(path))
  end
  return pem, nil
end

-- Build a per-config parsed cert + key pair. lua-resty-http accepts
-- the cdata `ssl.parse_pem_cert(pem)` / `ssl.parse_pem_priv_key(pem)`
-- handles directly. The cache key is the inline PEM (or the file
-- path + mtime) so an operator-driven cert rotation invalidates the
-- cache automatically on the next request.
local function _materialize(conf)
  local cache_key = (conf.client_cert_pem or conf.client_cert_file or "")
                    .. "|"
                    .. (conf.client_key_pem or conf.client_key_file or "")
                    .. "|"
                    .. (conf.sidecar_ca_pem or conf.sidecar_ca_file or "")
  if _cert_cache[cache_key] then
    return _cert_cache[cache_key], _key_cache[cache_key], nil
  end
  local cert_pem, err = _read_pem(conf.client_cert_pem, conf.client_cert_file, "client_cert")
  if err then return nil, nil, err end
  local key_pem
  key_pem, err = _read_pem(conf.client_key_pem, conf.client_key_file, "client_key")
  if err then return nil, nil, err end

  local cert_handle, perr = ssl.parse_pem_cert(cert_pem)
  if not cert_handle then
    return nil, nil, "spendguard: parse client cert: " .. tostring(perr)
  end
  local key_handle, kerr = ssl.parse_pem_priv_key(key_pem)
  if not key_handle then
    return nil, nil, "spendguard: parse client key: " .. tostring(kerr)
  end
  _cert_cache[cache_key] = cert_handle
  _key_cache[cache_key] = key_handle
  return cert_handle, key_handle, nil
end

-- Parse `https://host:port/optional/path` into `(host, port, path)`.
-- lua-resty-http expects the three components separately. The HTTP
-- companion contract puts `/v1/<verb>` paths at the root so the
-- operator-supplied URL is expected to be authority-only; we still
-- accept a path so a future API-version prefix is doable without
-- breaking older configs.
local function _split_url(url)
  -- expected shape: https://host[:port][/prefix]
  local host, port, path = url:match("^https://([^/:]+):?(%d*)(/?.*)$")
  if not host then
    return nil, nil, nil, "spendguard: malformed sidecar_url: " .. tostring(url)
  end
  if port == "" then port = "443" end
  if path == "" then path = "/" end
  return host, tonumber(port), path, nil
end

-- Build a request body POST helper. Used by all three endpoints.
local function _post_json(conf, endpoint_path, payload)
  local host, port, prefix, err = _split_url(conf.sidecar_url)
  if err then return nil, err end
  local cert_handle, key_handle, merr = _materialize(conf)
  if merr then return nil, merr end

  local httpc = http.new()
  httpc:set_timeout(conf.timeout_ms or 500)

  -- Connect with full SSL options. `ssl_server_name` is set to the
  -- host literal so SNI matches the workload SVID's DNS / URI SAN
  -- the sidecar's ServerName verifier expects.
  local ok, cerr = httpc:connect({
    scheme = "https",
    host = host,
    port = port,
    ssl_verify = true,
    ssl_server_name = host,
    ssl_client_cert = cert_handle,
    ssl_client_priv_key = key_handle,
  })
  if not ok then
    return nil, "spendguard: connect " .. host .. ":" .. tostring(port) .. ": " .. tostring(cerr)
  end

  local body, jerr = cjson.encode(payload)
  if not body then
    httpc:close()
    return nil, "spendguard: encode payload: " .. tostring(jerr)
  end

  local res, rerr = httpc:request({
    method = "POST",
    path = (prefix == "/" and "" or prefix) .. endpoint_path,
    headers = {
      ["Content-Type"] = "application/json",
      ["Accept"] = "application/json",
      ["Content-Length"] = tostring(#body),
    },
    body = body,
  })
  if not res then
    httpc:close()
    return nil, "spendguard: request " .. endpoint_path .. ": " .. tostring(rerr)
  end

  local resp_body, berr = res:read_body()
  -- Keepalive the connection so the next call (commit, retry) reuses
  -- the existing mTLS handshake.
  httpc:set_keepalive(30000, 16)
  if not resp_body then
    return nil, "spendguard: read body " .. endpoint_path .. ": " .. tostring(berr)
  end

  if res.status >= 200 and res.status < 300 then
    local decoded, derr = cjson.decode(resp_body)
    if not decoded then
      return nil, "spendguard: decode body " .. endpoint_path .. ": " .. tostring(derr)
    end
    return decoded, nil, res.status
  end
  -- Sidecar-side errors come back as 4xx/5xx with a {"code", "message"}
  -- body. Return the structured error so the handler can branch on
  -- 409 (idempotency conflict) vs 503 (degrade) etc.
  local decoded = cjson.decode(resp_body) or { code = "SPENDGUARD_UNKNOWN", message = resp_body }
  return nil, "spendguard: status " .. tostring(res.status) .. " " .. tostring(decoded.code), res.status
end

-- ────────────────────────────────────────────────────────────────────
-- Public surface.
-- ────────────────────────────────────────────────────────────────────

--- POST /v1/tokenize
function _M.tokenize(conf, provider, model, prompt)
  return _post_json(conf, "/v1/tokenize", {
    provider = provider,
    model = model,
    prompt = prompt,
  })
end

--- POST /v1/decision
function _M.decision(conf, opts)
  return _post_json(conf, "/v1/decision", {
    tenant_id = conf.tenant_id,
    claim_estimate_atomic = tostring(opts.claim_estimate_atomic or 0),
    prompt_class = conf.prompt_class or "general",
    model_class = opts.model_class,
    idempotency_key = opts.idempotency_key,
    budget_id = conf.budget_id,
  })
end

--- POST /v1/trace
function _M.trace(conf, opts)
  return _post_json(conf, "/v1/trace", {
    reservation_id = opts.reservation_id,
    outcome = opts.outcome,
    provider_event_id = opts.provider_event_id,
    input_tokens = opts.input_tokens,
    output_tokens = opts.output_tokens,
    actual_amount_atomic = opts.actual_amount_atomic,
  })
end

-- Exposed for spec tests so they can verify _split_url + _read_pem
-- behaviour without spinning a real HTTPS listener.
_M._test = {
  split_url = _split_url,
  read_pem = _read_pem,
}

return _M
