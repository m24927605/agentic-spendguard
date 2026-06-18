-- D09 SLICE 5 — Kong Lua fallback handler.
--
-- This is the experimental Lua port of the SpendGuard Kong plugin
-- per `docs/specs/coverage/D09_kong_ai_gateway/design.md` §3.2. It
-- covers `access` (reserve) + `body_filter` (commit) against the
-- same HTTP companion endpoints the Go plugin speaks. The Go plugin
-- remains the supported production path; this fallback exists so
-- operators on constrained OpenResty images (or OSS Kong without
-- go-plugin-server support) still have a SpendGuard option.
--
-- Lifecycle mirrors `plugins/kong/spendguard-go/access.go` +
-- `body_filter.go`:
--
--   1. access:
--      - pull request body (kong.request.get_raw_body)
--      - detect provider shape (openai vs anthropic) by JSON keys +
--        path (review-standards.md §4.2 + §6.3)
--      - POST /v1/tokenize → input_tokens
--      - POST /v1/decision → ALLOW / DENY / DEGRADE
--      - DENY → kong.response.exit(429, SPENDGUARD_DENY body)
--      - ALLOW → ctx.shared["spendguard_reservation_id"] = id
--      - DEGRADE → fail_open ? log + continue : exit(503)
--
--   2. body_filter:
--      - accumulate ngx.arg[1] into ctx.shared until ngx.arg[2] true
--      - parse provider usage (openai prompt_tokens / completion_tokens;
--        anthropic input_tokens / output_tokens)
--      - POST /v1/trace ACCEPTED (or REJECTED on upstream 5xx)
--      - flag committed so a repeat-call short-circuits
--
-- Shared-context keys are deliberately identical to the Go plugin
-- (`spendguard_reservation_id`, `spendguard_provider`,
-- `spendguard_degraded`, `spendguard_committed`,
-- `spendguard_body_buffer`) so a debugging install running both
-- plugins side-by-side does not double-commit.

local cjson = require "cjson.safe"
local client = require "kong.plugins.spendguard.sidecar_client"

local SpendGuardHandler = {
  -- review-standards.md §6.3: 950 fires BEFORE ai-proxy (770) so the
  -- reserve happens upstream of upstream auth. Identical to the Go
  -- plugin's `Priority` constant.
  PRIORITY = 950,
  VERSION = "1.0.0",
}

-- ────────────────────────────────────────────────────────────────────
-- Shared-context keys (mirrors plugins/kong/spendguard-go/access.go
-- + body_filter.go constants — DO NOT RENAME without bumping major).
-- ────────────────────────────────────────────────────────────────────

local CTX_RESERVATION_ID = "spendguard_reservation_id"
local CTX_PROVIDER       = "spendguard_provider"
local CTX_DEGRADED       = "spendguard_degraded"
local CTX_COMMITTED      = "spendguard_committed"
local CTX_BODY_BUFFER    = "spendguard_body_buffer"

-- ────────────────────────────────────────────────────────────────────
-- Helpers.
-- ────────────────────────────────────────────────────────────────────

--- Detect provider shape from the request path + body. Mirrors the
--- Go implementation in `plugins/kong/spendguard-go/provider_route.go`.
--- Returns `(provider, model, prompt, nil)` on success or `(nil,
--- nil, nil, err)` on a body we cannot recognise.
local function _detect_provider(path, body_str)
  local body, derr = cjson.decode(body_str or "")
  if not body or type(body) ~= "table" then
    return nil, nil, nil, "decode body: " .. tostring(derr)
  end
  -- Anthropic /v1/messages → presence of `messages` + `max_tokens`.
  if path and path:find("/v1/messages", 1, true) and body.messages then
    local model = body.model or "claude-3-haiku-20240307"
    local prompt = ""
    for _, m in ipairs(body.messages or {}) do
      if type(m.content) == "string" then
        prompt = prompt .. m.content .. "\n"
      end
    end
    return "anthropic", model, prompt
  end
  -- OpenAI /v1/chat/completions → `messages` + `model` + `usage`.
  if body.messages and body.model then
    local prompt = ""
    for _, m in ipairs(body.messages or {}) do
      if type(m.content) == "string" then
        prompt = prompt .. m.content .. "\n"
      end
    end
    return "openai", body.model, prompt
  end
  -- OpenAI /v1/completions legacy shape — `prompt` + `model`.
  if body.prompt and body.model then
    local prompt = body.prompt
    if type(prompt) == "table" then prompt = table.concat(prompt, "\n") end
    return "openai", body.model, prompt or ""
  end
  return nil, nil, nil, "unrecognised provider shape (need messages+model)"
end

--- Translate a sidecar error into either DENY or DEGRADE per
--- review-standards.md §4.4 + §4.7. Returns true if the handler
--- should short-circuit, false to continue.
local function _fail_open_or_deny(conf, status, code, msg)
  if conf.fail_open then
    kong.log.warn("spendguard degraded fail-open: ", code, " ", msg)
    kong.ctx.shared[CTX_DEGRADED] = "1"
    return false
  end
  kong.response.exit(status, {
    error = msg,
    code = code,
  })
  return true
end

--- Hash request body for the auto-derived idempotency key. We do not
--- need cryptographic strength — Kong-side retries within the same
--- attempt benefit from a deterministic key, that's all. SHA1 of the
--- body is enough.
local function _auto_idempotency_key(body_str)
  local resty_sha1 = require "resty.sha1"
  local sha1 = resty_sha1:new()
  sha1:update(body_str or "")
  local digest = sha1:final()
  -- ngx.encode_base64 + truncation keeps it short for log lines.
  return "kong-lua-auto-" .. ngx.encode_base64(digest):sub(1, 22)
end

-- ────────────────────────────────────────────────────────────────────
-- access phase.
-- ────────────────────────────────────────────────────────────────────

function SpendGuardHandler:access(conf)
  -- (1) Pull body once. Kong's `request_buffering: true` must be set
  --     on the route per design §3.3; otherwise the body is nil.
  local body_str = kong.request.get_raw_body() or ""
  if #body_str == 0 then
    -- Empty bodies are a client error, not a degrade path
    -- (review-standards §4.6).
    kong.response.exit(400, {
      error = "empty request body",
      code = "SPENDGUARD_EMPTY_BODY",
    })
    return
  end

  -- (2) Detect provider shape.
  local path = kong.request.get_path()
  local provider, model, prompt, derr = _detect_provider(path, body_str)
  if not provider then
    kong.response.exit(400, {
      error = derr,
      code = "SPENDGUARD_UNRECOGNISED_REQUEST",
    })
    return
  end

  -- (3) Idempotency key — prefer header, fall back to body hash.
  local idem_key = kong.request.get_header("Idempotency-Key")
  if not idem_key or #idem_key == 0 then
    idem_key = _auto_idempotency_key(body_str)
  end

  -- (4) Tokenize.
  local tok_resp, terr = client.tokenize(conf, provider, model, prompt)
  if not tok_resp then
    if _fail_open_or_deny(conf, 503, "SPENDGUARD_TOKENIZE_UNREACHABLE", terr) then return end
    return
  end

  -- (5) Decision.
  local dec_resp, derr2, dstatus = client.decision(conf, {
    claim_estimate_atomic = tok_resp.input_tokens,
    model_class = provider .. "/" .. model,
    idempotency_key = idem_key,
  })
  if not dec_resp then
    -- 409 IdempotencyConflict is a client error per review-standards
    -- §4.6 — surface it directly so a misbehaving caller sees it.
    if dstatus == 409 then
      kong.response.exit(409, {
        error = derr2,
        code = "SPENDGUARD_IDEMPOTENCY_CONFLICT",
      })
      return
    end
    if _fail_open_or_deny(conf, 503, "SPENDGUARD_DECISION_UNREACHABLE", derr2) then return end
    return
  end

  -- (6) Verdict branch.
  local verdict = dec_resp.verdict
  if verdict == "ALLOW" then
    -- Sidecar contract: ALLOW must carry a reservation_id. An empty
    -- one means the companion is mis-wired; fail closed (mirrors the
    -- Go plugin's access.go:207-213). The unconditional `return`
    -- after `_fail_open_or_deny` is required: even in the fail-open
    -- branch we must NOT fall through and stash an empty
    -- reservation_id (which body_filter would then silently skip on
    -- the `#reservation_id == 0` marker check, dropping the commit).
    if not dec_resp.reservation_id or #dec_resp.reservation_id == 0 then
      _fail_open_or_deny(conf, 503, "SPENDGUARD_RESERVATION_MISSING", "ALLOW without reservation_id")
      return
    end
    kong.ctx.shared[CTX_RESERVATION_ID] = dec_resp.reservation_id
    kong.ctx.shared[CTX_PROVIDER] = provider
  elseif verdict == "DENY" then
    kong.response.exit(429, {
      error = "budget exceeded",
      code = "SPENDGUARD_DENY",
      reason_codes = dec_resp.reason_codes or {},
      decision_id = dec_resp.decision_id,
    })
  elseif verdict == "DEGRADE" then
    if conf.fail_open then
      kong.log.warn("spendguard degrade fail-open: ", table.concat(dec_resp.reason_codes or {}, ","))
      kong.ctx.shared[CTX_DEGRADED] = "1"
    else
      kong.response.exit(503, {
        error = "guardrail degraded",
        code = "SPENDGUARD_DEGRADE",
      })
    end
  else
    -- Unknown verdict — fail-closed by default.
    if not conf.fail_open then
      kong.response.exit(500, {
        error = "unknown verdict: " .. tostring(verdict),
        code = "SPENDGUARD_UNKNOWN_VERDICT",
      })
    end
  end
end

-- ────────────────────────────────────────────────────────────────────
-- body_filter phase — accumulate response chunks, commit on EOF.
-- ────────────────────────────────────────────────────────────────────

--- Parse the provider's usage block from a fully-buffered response.
--- Returns `(input_tokens, output_tokens, provider_event_id)` or nil
--- on parse failure (the handler treats nil as RUN_ABORTED per
--- review-standards §5.5).
local function _parse_usage(provider, body_str)
  local body, _ = cjson.decode(body_str or "")
  if not body or type(body) ~= "table" then return nil end
  if provider == "openai" then
    local usage = body.usage
    if not usage then return nil end
    return usage.prompt_tokens, usage.completion_tokens, body.id
  end
  if provider == "anthropic" then
    local usage = body.usage
    if not usage then return nil end
    return usage.input_tokens, usage.output_tokens, body.id
  end
  return nil
end

function SpendGuardHandler:body_filter(conf)
  -- (0) Plugin-side dedup (review-standards §5.2).
  if kong.ctx.shared[CTX_COMMITTED] == "1" then return end

  -- (1) Access ALLOW marker.
  local reservation_id = kong.ctx.shared[CTX_RESERVATION_ID]
  if not reservation_id or #reservation_id == 0 then return end

  -- (2) Accumulate. `ngx.arg[1]` is the chunk; `ngx.arg[2]` is the
  --     end-of-body flag. Mirrors the Go plugin's `isFinalChunk`
  --     check.
  local chunk = ngx.arg[1] or ""
  local eof = ngx.arg[2]

  local buffer = kong.ctx.shared[CTX_BODY_BUFFER] or ""
  if chunk and #chunk > 0 then
    buffer = buffer .. chunk
    kong.ctx.shared[CTX_BODY_BUFFER] = buffer
  end
  if not eof then return end

  -- (3) End-of-body: parse usage + emit trace.
  --     Mark committed FIRST so a stray repeat call is a no-op even
  --     if the trace POST below times out (review-standards §5.2).
  kong.ctx.shared[CTX_COMMITTED] = "1"

  local provider = kong.ctx.shared[CTX_PROVIDER] or "openai"
  local upstream_status = kong.service.response.get_status() or 200

  local outcome = "ACCEPTED"
  if upstream_status >= 500 then outcome = "REJECTED" end

  local input_tokens, output_tokens, provider_event_id
  if outcome == "ACCEPTED" then
    input_tokens, output_tokens, provider_event_id = _parse_usage(provider, buffer)
    if not input_tokens then
      -- Could not parse — treat as REJECTED so the reservation is
      -- released, not committed.
      outcome = "REJECTED"
    end
  end

  -- The sidecar requires actual_amount_atomic on ACCEPTED; an empty
  -- field forces the estimated-amount commit lane (handlers.rs) and
  -- mis-counts realized spend. Mirror body_filter.go:201-202 by
  -- surfacing the total token count as the realized amount. The
  -- sidecar validates this field as a decimal-integer STRING
  -- (service.rs is_ascii_digit), so we tostring() it — a Lua number
  -- would encode as a JSON number and trip a 400. Only set it on
  -- ACCEPTED; the REJECTED release lane does not read it.
  local actual_amount_atomic
  if outcome == "ACCEPTED" then
    actual_amount_atomic = tostring((input_tokens or 0) + (output_tokens or 0))
  end

  local _, terr = client.trace(conf, {
    reservation_id = reservation_id,
    outcome = outcome,
    provider_event_id = provider_event_id,
    input_tokens = input_tokens,
    output_tokens = output_tokens,
    actual_amount_atomic = actual_amount_atomic,
  })
  if terr then
    -- review-standards §5.6: commit-lane timeouts log but do not
    -- short-circuit — the upstream response is already in flight.
    kong.log.err("spendguard body_filter trace: ", terr)
  end
end

-- Exposed for spec tests to drive the pure helpers without a full
-- Kong request lifecycle.
SpendGuardHandler._test = {
  detect_provider = _detect_provider,
  parse_usage = _parse_usage,
  auto_idempotency_key = _auto_idempotency_key,
  ctx_keys = {
    reservation_id = CTX_RESERVATION_ID,
    provider = CTX_PROVIDER,
    degraded = CTX_DEGRADED,
    committed = CTX_COMMITTED,
    body_buffer = CTX_BODY_BUFFER,
  },
}

return SpendGuardHandler
