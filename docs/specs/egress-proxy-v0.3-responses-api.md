# Egress proxy v0.3 — POST /v1/responses pass-through

> **Status**: spec v1 — design draft.
> **Goal**: close the openai-agents shorthand gap that PR #66 documented as a v0.3 follow-up. After v0.3, `Agent(model="gpt-4o-mini")` works through the proxy with NO ChatCompletions workaround.
> **Audience**: implementer closing issue #65.
> **Scope**: OpenAI Responses API non-streaming + streaming. Tool calls + multi-step `Runner.run` work transparently because each round-trip is its own `POST /v1/responses`.

---

## 1. Context — what gap this closes

The `openai-agents` SDK default model class is `OpenAIResponsesModel` which hits `POST /v1/responses`. v0.2's proxy only routed `POST /v1/chat/completions`, so the shorthand failed with 404 → workaround required (`OpenAIChatCompletionsModel(openai_client=AsyncOpenAI(base_url=...))` explicit construction). Documented in `README.md`'s auto-instrument quickstart + issue #65.

v0.3 lights up the shorthand. After this:

```python
import os
os.environ["OPENAI_BASE_URL"] = "http://localhost:9000/v1"

from agents import Agent, Runner
agent = Agent(name="demo", instructions="Reply concisely.", model="gpt-4o-mini")
result = await Runner.run(agent, "Say hi")  # → 200, budget gated, audit chain
```

## 2. Design

### 2.1 What's shared with chat_completions

Both APIs follow the same proxy lifecycle:
1. PRE: parse body → resolve identification → call sidecar `RequestDecision` → if not CONTINUE, fail-closed
2. Forward: POST upstream with byte-identical body + Authorization
3. POST: parse usage → call sidecar EmitTraceEvents `LLM_CALL_POST` + ConfirmPublishOutcome(APPLIED)
4. Stream: tee upstream stream to client + side parser captures usage from end event

These steps live in `forward.rs`. v0.3 extracts the shared lifecycle into a private helper and adds Responses-specific delta only.

### 2.2 What's different from chat_completions

| Aspect | Chat Completions | Responses API |
|---|---|---|
| Upstream URL | `/v1/chat/completions` | `/v1/responses` |
| `stream` flag | top-level `stream: true` | same |
| Usage opt-in (streaming) | `stream_options.include_usage: true` (proxy auto-injects) | included by default — no injection needed |
| Usage shape (non-streaming) | `usage.total_tokens` (top-level) | `usage.total_tokens` (top-level) — same |
| Usage shape (streaming) | last data event with `usage.total_tokens` (chunk has top-level `usage`) | `event: response.completed\ndata: {"response": {..., "usage": {"total_tokens": N}}}` — nested under `response.usage` |
| `[DONE]` sentinel | yes | yes |
| Errors | OpenAI error shape | OpenAI error shape — same |

### 2.3 Implementation strategy

**Refactor first**: extract a generic `forward_openai_request(state, headers, body, api_kind)` from the existing `chat_completions` body. `api_kind: ApiKind` enum distinguishes:
- `ApiKind::ChatCompletions` — upstream URL + chat-completions-style SSE usage parser
- `ApiKind::Responses` — upstream URL + responses-style SSE usage parser

Public handlers stay as thin wrappers:
- `pub async fn chat_completions(...)` → `forward_openai_request(..., ApiKind::ChatCompletions)`
- `pub async fn responses(...)` → `forward_openai_request(..., ApiKind::Responses)`

Shared helpers (`commit_on_success`, `release_on_upstream_error`, etc.) are unchanged.

Adds: `parse_usage_from_responses_sse_event` mirrors the chat-completions parser but extracts from `response.usage.total_tokens` nested path.

## 3. Components

### 3.1 forward.rs — refactor + add api_kind

```rust
#[derive(Debug, Clone, Copy)]
enum ApiKind {
    ChatCompletions,
    Responses,
}

impl ApiKind {
    fn upstream_url(self) -> &'static str {
        match self {
            Self::ChatCompletions => "https://api.openai.com/v1/chat/completions",
            Self::Responses => "https://api.openai.com/v1/responses",
        }
    }

    fn needs_include_usage_injection(self) -> bool {
        matches!(self, Self::ChatCompletions)
    }

    fn parse_sse_usage(self, event: &[u8]) -> Option<i64> {
        match self {
            Self::ChatCompletions => parse_usage_from_event(event),
            Self::Responses => parse_usage_from_responses_event(event),
        }
    }
}

async fn forward_openai_request(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
    api_kind: ApiKind,
) -> Result<Response, ForwardError> {
    // ... existing chat_completions body, with:
    //   1. UPSTREAM_URL → api_kind.upstream_url()
    //   2. include_usage injection guarded by api_kind.needs_include_usage_injection()
    //   3. SSE parser → api_kind.parse_sse_usage(event)
}

pub async fn chat_completions(...) -> Result<Response, ForwardError> {
    forward_openai_request(state, headers, body, ApiKind::ChatCompletions).await
}

pub async fn responses(...) -> Result<Response, ForwardError> {
    forward_openai_request(state, headers, body, ApiKind::Responses).await
}
```

### 3.2 New SSE event parser for Responses API

```rust
fn parse_usage_from_responses_event(event: &[u8]) -> Option<i64> {
    let s = std::str::from_utf8(event).ok()?;
    let mut payload = String::new();
    for line in s.lines() {
        // Responses SSE has `event: response.completed\n` header lines we
        // skip; only `data:` lines carry the JSON payload.
        if let Some(l) = line.strip_prefix("data:") {
            let l = l.trim_start();
            if l == "[DONE]" { return None; }
            payload.push_str(l);
        }
    }
    let v: Value = serde_json::from_str(&payload).ok()?;
    // Responses API: usage nested under `response.usage.total_tokens`.
    v.get("response")
        .and_then(|r| r.get("usage"))
        .and_then(|u| u.get("total_tokens"))
        .and_then(|t| t.as_i64())
}
```

### 3.3 Route mounting

`services/egress_proxy/src/main.rs`:

```rust
let app = Router::new()
    .route("/v1/chat/completions", post(forward::chat_completions))
    .route("/v1/responses", post(forward::responses))   // NEW
    ...
```

### 3.4 Demo

- Update `deploy/demo/proxy_smoke.sh` Step 4 to ALSO smoke-test the Responses API path via curl (non-streaming + streaming).
- Update `deploy/demo/demo/run_demo.py` `run_openai_agents_proxy_mode()` to use the SHORTHAND form `Agent(model="gpt-4o-mini")` without the explicit `OpenAIChatCompletionsModel` workaround.

### 3.5 Launch docs

- README `## What works today (v0.2) vs what's coming (v0.3)` section gets updated to mark `openai-agents shorthand` as ✅ verified.
- LangChain docs PR description doesn't need updating (it always claimed Chat Completions).

## 4. Implementation slices

Single slice. Bounded to:
- `services/egress_proxy/src/forward.rs` (refactor + new parser + handler)
- `services/egress_proxy/src/main.rs` (route mount)
- `deploy/demo/demo/run_demo.py` (drop ChatCompletions workaround in openai_agents_proxy mode)
- `deploy/demo/proxy_smoke.sh` (add `/v1/responses` smoke)
- `README.md` (toggle ✅)

## 5. Test plan

### Unit (Rust)

- `parse_usage_from_responses_event` — table test: `event: response.completed\ndata: {...}` with usage → Some(N); `event: response.created\ndata: {...}` without usage → None; multi-line `data:` accumulation; `[DONE]` → None.
- `ApiKind::needs_include_usage_injection` — ChatCompletions=true, Responses=false.

### Integration (real OpenAI via demo)

- `DEMO_MODE=proxy make demo-up`:
  - Existing chat_completions smoke must PASS (no regression).
  - New `/v1/responses` smoke must PASS: non-streaming returns 200 with `usage.total_tokens`; streaming returns SSE events with `response.completed` carrying usage.
- `DEMO_MODE=agent_real_openai_agents_proxy make demo-up`:
  - The demo now uses shorthand `Agent(model="gpt-4o-mini")` (NO explicit ChatCompletions model).
  - Must PASS with the proxy gating the call.

### Adversarial (codex)

- Responses API streaming where `response.completed` arrives in a different event slot than expected.
- Tool-calling response (Responses API encodes tools differently); usage parser must still work.
- Refactor regression: chat_completions hot-path unchanged byte-for-byte; existing PR #63 streaming smoke still PASSes.
- `include_usage` injection: Responses path must NOT inject `stream_options.include_usage=true` (it's not a valid field on the Responses request).

## 6. Acceptance criteria

- [ ] `services/egress_proxy/src/forward.rs` refactored to share lifecycle between `chat_completions` and `responses`; `ApiKind` enum threads behavior differences
- [ ] `services/egress_proxy/src/main.rs` mounts `POST /v1/responses`
- [ ] `deploy/demo/proxy_smoke.sh` smokes both endpoints (non-streaming + streaming)
- [ ] `deploy/demo/demo/run_demo.py` `run_openai_agents_proxy_mode()` uses shorthand `Agent(model="...")` PASSes
- [ ] `README.md`'s `## What works today` section marks `openai-agents shorthand` as ✅
- [ ] No regression in PR #63 streaming smoke (chat_completions path)
- [ ] Codex review reaches GREEN within 5 rounds

## 7. Code review standards (codex prompts)

**r1 adversarial focus**:
- Did the refactor introduce a stream-handling regression in chat_completions? Verify the call sites are byte-equivalent before vs after.
- `include_usage` injection: Responses API request must NOT include `stream_options` (would be ignored, possibly logged as warning by OpenAI). Verify guard.
- Usage parser: Responses streaming usage is nested under `response.usage.total_tokens` (NOT top-level). Test against real OpenAI response shape.
- Tool-calling rounds: `Runner.run` makes multiple `/v1/responses` calls when tools fire. Each must hit the proxy gate. Verify by counting reserve + commit_estimated rows in ledger.
- The Responses API may emit usage on EARLIER events (e.g., `response.output_text.done` rather than `response.completed`). Spec assumes `response.completed` is the canonical usage carrier — verify against real OpenAI traffic.

**Staff escalation triggers**:
- r5 RED on Responses API SSE wire shape — escalate to distributed-systems + ledger-audit (any usage-counting gap leaks into the audit chain).

## 8. Demo verification

```bash
$ DEMO_MODE=proxy make demo-up
[proxy-smoke] step 1 (CONTINUE chat_completions): real OpenAI gpt-4o-mini  OK
[proxy-smoke] step 2 (STOP chat_completions):                              OK
[proxy-smoke] step 4 (streaming chat_completions): SSE events + usage      OK
[proxy-smoke] step 5 (NEW: non-streaming responses):                       OK
[proxy-smoke] step 6 (NEW: streaming responses): SSE events + usage        OK

$ DEMO_MODE=agent_real_openai_agents_proxy make demo-up
[demo] launch-claim verification: pointing openai-agents at http://egress-proxy:9000/v1
[demo]   NO SpendGuard SDK adapter; NO wrapper; NO ChatCompletions workaround.
[demo]   Just OPENAI_BASE_URL + Agent(model="...") shorthand.
[demo] Runner.run via proxy OK; output='Hi!'
[demo] launch-claim verified: openai-agents shorthand WORKS through v0.3 proxy
```

## 9. Deferred items

- Tool calling validation (basic-case works because each tool round-trip is a separate `/v1/responses`; complex tool flows are deferred to v0.4 if real bugs surface)
- Bedrock / Anthropic native streaming (separate spec)
- Prometheus metric for per-API request counters (defer)

## 10. References

- Issue #65 — original gap report
- PR #66 — README scoping that documents the gap
- `docs/specs/egress-proxy-v0.2-streaming-sse.md` — pattern this can mirror
- OpenAI Responses API: https://platform.openai.com/docs/api-reference/responses
- openai-agents source: `agents.models.openai_responses.OpenAIResponsesModel` (default)
