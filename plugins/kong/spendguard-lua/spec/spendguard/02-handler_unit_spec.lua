-- D09 SLICE 5 — handler unit specs (no Kong runtime needed).
--
-- These specs run via `busted spec/` against the exported `_test`
-- helpers in `kong/plugins/spendguard/handler.lua`. They cover the
-- pure-Lua helpers (provider detection, usage parsing, idempotency
-- key derivation) without needing a live Kong worker.
--
-- The full mTLS-against-sidecar matrix runs as the Go plugin's
-- integration tests (per design §3.2 — the Lua port does not get
-- the conformance-test guarantee).

local handler = require "kong.plugins.spendguard.handler"
local detect_provider = handler._test.detect_provider
local parse_usage = handler._test.parse_usage
local auto_idem = handler._test.auto_idempotency_key

describe("provider detection", function()
  it("recognises OpenAI /v1/chat/completions", function()
    local body = [[{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}]]
    local provider, model, prompt = detect_provider("/v1/chat/completions", body)
    assert.equals("openai", provider)
    assert.equals("gpt-4o-mini", model)
    assert.is_truthy(prompt:find("hi"))
  end)

  it("recognises Anthropic /v1/messages", function()
    local body = [[{"model":"claude-3-haiku-20240307","messages":[{"role":"user","content":"hi"}]}]]
    local provider, model = detect_provider("/v1/messages", body)
    assert.equals("anthropic", provider)
    assert.equals("claude-3-haiku-20240307", model)
  end)

  it("returns an error on a body we can't recognise", function()
    local _, _, _, err = detect_provider("/v1/foo", [[{"bar":"baz"}]])
    assert.is_string(err)
  end)

  it("returns an error on malformed JSON", function()
    local _, _, _, err = detect_provider("/v1/chat/completions", "not json")
    assert.is_string(err)
  end)
end)

describe("usage parsing", function()
  it("extracts OpenAI usage block", function()
    local body = [[{
      "id":"chatcmpl-abc",
      "usage":{"prompt_tokens":5,"completion_tokens":7,"total_tokens":12}
    }]]
    local input, output, eid = parse_usage("openai", body)
    assert.equals(5, input)
    assert.equals(7, output)
    assert.equals("chatcmpl-abc", eid)
  end)

  it("extracts Anthropic usage block", function()
    local body = [[{
      "id":"msg_abc",
      "usage":{"input_tokens":11,"output_tokens":13}
    }]]
    local input, output, eid = parse_usage("anthropic", body)
    assert.equals(11, input)
    assert.equals(13, output)
    assert.equals("msg_abc", eid)
  end)

  it("returns nil when usage block is missing", function()
    local body = [[{"id":"chatcmpl-no-usage","choices":[]}]]
    local input = parse_usage("openai", body)
    assert.is_nil(input)
  end)
end)

describe("idempotency key derivation", function()
  it("returns a deterministic value for the same body", function()
    local a = auto_idem("body-A")
    local b = auto_idem("body-A")
    assert.equals(a, b)
  end)

  it("returns distinct values for different bodies", function()
    local a = auto_idem("body-A")
    local b = auto_idem("body-B")
    assert.not_equals(a, b)
  end)

  it("prefixes the value so logs can grep for it", function()
    local k = auto_idem("body")
    assert.is_truthy(k:find("^kong%-lua%-auto%-"))
  end)
end)

describe("shared context keys (review-standards §5.2)", function()
  it("matches the Go plugin's constant names", function()
    -- The Go plugin's `CtxKeyReservationID` constant is
    -- "spendguard_reservation_id" — see plugins/kong/spendguard-go/
    -- access.go. The Lua port MUST use the same literal string so a
    -- side-by-side debug install doesn't double-commit.
    assert.equals("spendguard_reservation_id", handler._test.ctx_keys.reservation_id)
    assert.equals("spendguard_provider", handler._test.ctx_keys.provider)
    assert.equals("spendguard_degraded", handler._test.ctx_keys.degraded)
    assert.equals("spendguard_committed", handler._test.ctx_keys.committed)
    assert.equals("spendguard_body_buffer", handler._test.ctx_keys.body_buffer)
  end)

  it("PRIORITY mirrors the Go plugin's 950", function()
    assert.equals(950, handler.PRIORITY)
  end)
end)
