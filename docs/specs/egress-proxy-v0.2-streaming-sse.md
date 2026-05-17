# Egress proxy v0.2 — SSE streaming pass-through

> **Status**: spec v1 — design draft for a single-slice implementation. Codex r0 not yet run.
> **Goal**: close the biggest functional gap in v0.1's "1-env-var auto-instrument" launch claim — `stream:true` chat completions currently return 501, breaking the openai-agents SDK whose `Runner.run` defaults to streaming.
> **Audience**: implementer closing v0.2-streaming + the user verifying the launch claim against real `openai-agents`.
> **Scope**: OpenAI Chat Completions SSE pass-through only. Anthropic / Bedrock / non-Chat-Completions streaming → separate spec(s).

---

## 1. Context — what gap this closes

Per memory `project_overview.md` "Auto-instrument egress proxy v0.1 — slices 1-3 shipped":

> Slice 3: HTTP pass-through `POST /v1/chat/completions` forwarding to `api.openai.com` via reqwest. Byte-identical body + Authorization. `stream:true` → 501 (request-side) + Content-Type=text/event-stream → 502 (response-side).

This was the right v0.1 cut (defer the harder streaming semantics until non-streaming was solid). But every public reference for `openai-agents` SDK (`Runner.run`, `Runner.run_streamed`) uses streaming by default; PyTorch/llama-index/CrewAI similarly. A user setting `OPENAI_BASE_URL=http://localhost:9000/v1` per the launch quickstart and running their agent immediately gets:

```
openai.APIError: Error code: 501 - {"error":{"code":"spendguard_streaming_unsupported"...
```

That's a launch-credibility-killing failure. v0.2 closes it.

## 2. Design

### 2.1 Two semantic shifts vs v0.1

1. **PRE reservation is unchanged**. Reserve before any byte hits OpenAI; STOP returns 429 fail-closed; CONTINUE proceeds. The streaming complexity is on the COMMIT lane only.
2. **POST commit moves from "after `bytes().await`" to "after the SSE stream completes"**. The proxy tees the upstream chunk stream — passing each chunk to the client byte-identical while side-buffering parsed SSE events for the `usage` field. When the stream ends, fire the commit lane with the captured `usage.total_tokens`.

### 2.2 OpenAI streaming usage requires opt-in

OpenAI's `chat.completions` streaming omits the `usage` block UNLESS the request body sets:

```json
{"stream": true, "stream_options": {"include_usage": true}}
```

Without `include_usage`, the proxy has no way to commit real token count. Two options:

- **(A) Require the client to set it** — fail-closed with 400 if client sends `stream:true` without `include_usage:true`.
- **(B) Proxy injects `include_usage:true`** if the client didn't set it.

**Pick B.** Rationale: the launch claim is "no code change". Forcing users to set `stream_options.include_usage=true` violates that. The proxy injecting it is invisible to the client (extra `data: {"usage":{...}}` event at end of stream; openai-python tolerates it; openai-agents tolerates it because it ignores trailing data after `data: [DONE]`).

Risk: a future OpenAI semver bump that breaks `include_usage` injection silently. Mitigation: emit a metric `egress_proxy_usage_injected_total` so anomalous gaps are observable.

### 2.3 Tee stream architecture

```
            ┌──────────────────────────────────────────┐
upstream ──▶│  bytes_stream() Stream<Item=Bytes>       │──▶ axum::Body ──▶ client
            │                                          │
            │  on each Bytes:                          │
            │   1. send Bytes to client (axum stream)  │
            │   2. send Bytes to side parser via mpsc  │
            └──────────────────────────────────────────┘
                          │
                          ▼
            ┌──────────────────────────────────────────┐
            │  side-parser tokio::spawn task           │
            │  accumulates partial events across       │
            │  Bytes boundaries; scans for `usage`     │
            │  field; on stream-end, sends usage to    │
            │  a oneshot channel                       │
            └──────────────────────────────────────────┘
                          │
                          ▼
            ┌──────────────────────────────────────────┐
            │  commit task tokio::spawn                │
            │  awaits oneshot; on receipt fires        │
            │  commit_on_success (existing slice 5);   │
            │  on parser failure / stream error fires  │
            │  release_on_upstream_error               │
            └──────────────────────────────────────────┘
```

Trade-off acknowledged: client sees SUCCESS (last byte delivered) BEFORE ledger commit completes. The reservation TTL (default 60s) protects against orphan commits — if commit fails or proxy crashes mid-flight, the reservation TTL-sweeps to RELEASE without double-billing. This is the same trade-off the SDK wrapper makes (commit is post-call, not synchronous), so behavior parity holds.

### 2.4 SSE event parsing

SSE wire format (RFC 8895, OpenAI follows):

```
data: {"id":"chatcmpl-...","object":"chat.completion.chunk","choices":[{"delta":{"content":"Hello"}}]}

data: {"id":"chatcmpl-...","choices":[{"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":3,"total_tokens":15}}

data: [DONE]

```

- Events separated by `\n\n` (or `\r\n\r\n` per spec; OpenAI uses `\n\n`).
- `data: ` line is the payload; multi-line `data:` accumulated by `\n`.
- `data: [DONE]` is the terminal sentinel.
- Usage is on the FINAL non-DONE event (only when `include_usage:true`).

Parser pseudocode:

```rust
let mut buffer = BytesMut::new();
let mut captured_usage: Option<i64> = None;
while let Some(chunk_result) = rx.recv().await {
    let chunk = chunk_result?;
    buffer.extend_from_slice(&chunk);
    while let Some(event_end) = find_event_boundary(&buffer) {
        let event_bytes = buffer.split_to(event_end);
        if let Some(usage) = parse_usage_from_event(&event_bytes) {
            captured_usage = Some(usage);
        }
        // Drop the \n\n separator after the event.
        buffer.advance(2);
    }
}
// stream ended; signal commit lane
let _ = usage_tx.send(captured_usage);
```

## 3. Components

### 3.1 forward.rs — body inspection (request side)

Replace `StreamingUnsupported` rejection with auto-injection of `stream_options.include_usage:true`:

```rust
let parsed: Value = serde_json::from_slice(&body).map_err(...)?;
let is_streaming = parsed.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

let body_for_upstream = if is_streaming {
    let mut v = parsed.clone();
    let opts = v
        .get_mut("stream_options")
        .and_then(|x| x.as_object_mut())
        .map(|m| m.clone())
        .unwrap_or_default();
    let mut opts = opts;
    let already_set = opts.get("include_usage")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    if !already_set {
        opts.insert("include_usage".to_string(), json!(true));
        v.as_object_mut().unwrap().insert(
            "stream_options".to_string(),
            Value::Object(opts),
        );
        metrics_inc("egress_proxy_usage_injected_total");
    }
    serde_json::to_vec(&v)?
} else {
    body.to_vec()
};
```

Don't mutate `body` (the original `Bytes`) after this point; pass `body_for_upstream` to `req.body(...)`.

### 3.2 forward.rs — response side

After the existing `req.send().await?` returns `resp`:

```rust
let content_type = resp.headers().get(CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .unwrap_or("");

if content_type.starts_with("text/event-stream") {
    // Streaming path (new).
    return forward_streaming(
        app, resp, /* reservation context */,
        is_streaming,  // sanity: must match
    ).await;
}

// Non-streaming path (existing).
let upstream_body = resp.bytes().await?;
// ... existing commit_on_success / release_on_upstream_error logic
```

### 3.3 `forward_streaming` (new)

```rust
async fn forward_streaming(
    app: Arc<AppState>,
    resp: reqwest::Response,
    /* reservation context */,
) -> Result<Response, ForwardError> {
    let upstream_status = resp.status();
    let upstream_headers = resp.headers().clone();
    let upstream_stream = resp.bytes_stream();

    // Channel from tee to parser.
    let (parser_tx, parser_rx) = tokio::sync::mpsc::unbounded_channel::<bytes::Bytes>();
    // Channel from parser to commit lane.
    let (usage_tx, usage_rx) = tokio::sync::oneshot::channel::<Option<i64>>();
    // Signal for error path (client disconnect, upstream error).
    let (err_tx, err_rx) = tokio::sync::oneshot::channel::<()>();

    // Spawn parser task.
    tokio::spawn(parse_usage_from_sse_stream(parser_rx, usage_tx));

    // Spawn commit lane task.
    let app_for_commit = app.clone();
    let /* reservation_id, decision_id, etc */ = ...;
    tokio::spawn(async move {
        tokio::select! {
            usage = usage_rx => {
                match usage {
                    Ok(Some(tokens)) => commit_on_success(&app_for_commit, ..., tokens).await,
                    Ok(None) => release_on_upstream_error(&app_for_commit, ...).await,
                    Err(_) => release_on_upstream_error(&app_for_commit, ...).await,
                }
            }
            _ = err_rx => {
                release_on_upstream_error(&app_for_commit, ...).await;
            }
        }
    });

    // Build the tee'd stream that forwards to client.
    let tee_stream = upstream_stream.map(move |chunk_result| {
        match chunk_result {
            Ok(b) => {
                // Side-buffer for parser.
                let _ = parser_tx.send(b.clone());
                Ok::<_, std::io::Error>(b)
            }
            Err(e) => {
                let _ = err_tx.send(());
                Err(std::io::Error::new(std::io::ErrorKind::Other, e))
            }
        }
    });

    // Forward upstream headers (filtered) + body stream to client.
    let mut response = Response::builder().status(upstream_status);
    for (name, value) in &upstream_headers {
        if should_forward_response_header(name) {
            response = response.header(name, value);
        }
    }
    Ok(response.body(Body::from_stream(tee_stream))?)
}
```

### 3.4 `parse_usage_from_sse_stream` (new helper)

```rust
async fn parse_usage_from_sse_stream(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<bytes::Bytes>,
    tx: tokio::sync::oneshot::Sender<Option<i64>>,
) {
    use bytes::BytesMut;
    let mut buffer = BytesMut::new();
    let mut last_usage: Option<i64> = None;
    while let Some(chunk) = rx.recv().await {
        buffer.extend_from_slice(&chunk);
        while let Some(boundary) = find_event_boundary(&buffer) {
            let event = buffer.split_to(boundary).freeze();
            // Strip trailing \n\n
            if buffer.len() >= 2 { buffer.advance(2); }
            if let Some(usage) = parse_usage_from_event(&event) {
                last_usage = Some(usage);
            }
        }
    }
    let _ = tx.send(last_usage);
}

fn find_event_boundary(buf: &[u8]) -> Option<usize> {
    // Find the first \n\n; tolerate \r\n\r\n if OpenAI ever sends them.
    buf.windows(2).position(|w| w == b"\n\n")
        .or_else(|| buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p))
}

fn parse_usage_from_event(event: &[u8]) -> Option<i64> {
    // Each event is `data: <json>\n` (possibly multi-line).
    // Concatenate the `data:` payload, drop `[DONE]`, parse as JSON,
    // extract usage.total_tokens.
    let s = std::str::from_utf8(event).ok()?;
    let mut payload = String::new();
    for line in s.lines() {
        let l = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:"))?;
        let l = l.trim_start();
        if l == "[DONE]" {
            return None;
        }
        payload.push_str(l);
    }
    let v: serde_json::Value = serde_json::from_str(&payload).ok()?;
    v.get("usage")
        .and_then(|u| u.get("total_tokens"))
        .and_then(|t| t.as_i64())
}
```

### 3.5 `should_forward_response_header` (new)

Whitelist of upstream → client response headers that pass through. Block hop-by-hop headers (`connection`, `transfer-encoding` — axum sets these itself) + sensitive headers (`openai-organization` if leaking is a concern — TBD). Whitelist: `content-type`, `cache-control`, `openai-version`, `openai-organization`, `x-request-id`.

### 3.6 Demo extension

`deploy/demo/proxy_smoke.sh` adds Step 4 (streaming) after the existing CONTINUE + STOP steps:

```bash
log "step 4 (STREAMING): real OpenAI streaming call via proxy..."
STREAM_RESP=$(curl -sS -N --max-time 30 \
    -X POST "${PROXY_URL}/v1/chat/completions" \
    -H "Authorization: Bearer ${OPENAI_API_KEY}" \
    -H "Content-Type: application/json" \
    -d '{"model":"gpt-4o-mini","stream":true,"messages":[...],"max_tokens":15}')

# Should contain SSE events.
echo "$STREAM_RESP" | grep -q "^data: " || fail "no SSE data events"
# Should contain usage.total_tokens (proxy auto-injected include_usage).
echo "$STREAM_RESP" | grep -q '"total_tokens"' || fail "no usage block"
# Should end with [DONE].
echo "$STREAM_RESP" | grep -q "^data: \[DONE\]$" || fail "no [DONE] sentinel"
log "  STREAMING OK; usage block + [DONE] present"
```

## 4. Implementation slices

Single slice. The change is bounded to:
- `services/egress_proxy/Cargo.toml` (`futures-util`, `bytes` deps)
- `services/egress_proxy/src/forward.rs` (request mutation + streaming response path + helpers)
- `deploy/demo/proxy_smoke.sh` (streaming test case)

## 5. Test plan

### Unit (Rust)

- `parse_usage_from_event` — table test with 5 inputs: usage chunk, content chunk (no usage), `[DONE]` chunk, multi-line `data:`, malformed JSON. Expected: Some(total_tokens) only for the usage chunk.
- `find_event_boundary` — table test with 3 inputs: complete `\n\n`, partial (no boundary yet), `\r\n\r\n`. Expected: position or None.
- Request body mutation — given `{"stream":true}` body, assert mutated body contains `"stream_options":{"include_usage":true}`. Given `{"stream":true,"stream_options":{"include_usage":true}}` body, assert unchanged (idempotency).

### Integration (real OpenAI via demo)

- `DEMO_MODE=proxy make demo-up` with `OPENAI_API_KEY` set:
  - Existing non-streaming smoke (Step 1 + Step 2) must PASS (no regression).
  - New Step 4 streaming smoke must PASS: SSE events present, usage block present, `[DONE]` sentinel present.
- Verify in ledger: `commit_estimated` row written with `tokens` matching the streamed usage.

### Adversarial (codex)

- Client disconnects mid-stream — does the parser task leak? Does release fire?
- Upstream sends usage block in middle of stream (not last event) — does parser pick up the LAST occurrence, not the first?
- Upstream stream contains `data: [DONE]\n\n` without trailing newline — does parser still terminate cleanly?
- `stream_options.include_usage` already set to FALSE by client — does proxy override to true? (Decision: yes, override, with metric.)
- Buffer never receives `\n\n` boundary (malformed stream) — does parser handle stream-end gracefully?
- ResvTTL expires mid-stream — what happens? (Reservation TTL is 60s; if stream takes > 60s, commit lane will fail with ReservationExpired → release. Acceptable; rare in practice.)

## 6. Acceptance criteria

- [ ] `Cargo.toml` adds `futures-util` + `bytes`.
- [ ] `forward.rs` no longer returns `StreamingUnsupported`; request-body mutation injects `include_usage:true` when missing.
- [ ] Streaming response path tees upstream `bytes_stream()` to client (via `Body::from_stream`) AND to a side parser that captures `usage.total_tokens`.
- [ ] Commit lane fires AFTER stream-end via the spawned task; uses captured tokens.
- [ ] Stream-error / parser-failure / client-disconnect fires release lane.
- [ ] Demo `proxy_smoke.sh` Step 4 (streaming) added + PASSes against real OpenAI gpt-4o-mini.
- [ ] Existing non-streaming smoke (Step 1 + Step 2) still PASSes (no regression).
- [ ] Ledger has 1 `commit_estimated` row from the streaming run with matching `tokens` from the SSE `usage` block.
- [ ] No new `expose_secret()` call sites; auth handling unchanged.
- [ ] Codex review reaches GREEN within 5 rounds (single-slice complexity is bounded).

## 7. Code review standards (codex prompts)

**r1 adversarial focus**:
- Streaming clone overhead: `parser_tx.send(b.clone())` copies every chunk; is the cost acceptable? (Bytes uses Arc-backed slices internally so clone is cheap; verify.)
- Backpressure: what if the parser task lags behind the network stream? `mpsc::unbounded_channel` will buffer indefinitely (memory growth risk on long/large streams). Switch to bounded channel with `send().await` so backpressure propagates to upstream read.
- TOCTOU on `is_streaming`: the request-body mutation reads `parsed.get("stream")` once; what if the parsed JSON had nested "stream" elsewhere? (Trust the top-level only; gpt-4o-mini schema doesn't nest.)
- Header forwarding: `should_forward_response_header` whitelist — does it correctly handle `Cache-Control` (no-cache for SSE)? Does it strip `Content-Length` (SSE is chunked)?
- Commit lane race: the spawned task may run AFTER the request handler returns; does the `Arc<AppState>` keep the sidecar handle alive long enough? (Yes — `Arc` clone keeps refcount up.)
- Reservation TTL: 60s is the default; does the streaming path bump it for long-running streams? (No; defer to v0.3 if needed.)
- Metrics: `egress_proxy_usage_injected_total` — is the counter wired into the existing Prometheus exporter? Where? (Check `services/egress_proxy/src/main.rs` for the registry; if no existing pattern, defer with a `// TODO` and a log emission.)

**r2-r5 expected patterns**:
- Compile-time errors from `axum::body::Body::from_stream` lifetime (needs `'static` Stream).
- `reqwest::Response::bytes_stream()` returns `impl Stream<Item = Result<Bytes, reqwest::Error>>` — error type mismatch with `io::Error` from axum's stream expectation.
- Missing `Send + Sync` bound on the closure passed to `.map()`.

**Staff escalation triggers** (per `auto-instrument-egress-proxy-spec.md` §14.1):
- r5 RED on backpressure semantics (unbounded vs bounded mpsc choice) — escalate to distributed-systems (memory growth + cancellation).
- r5 RED on commit-lane race against reservation TTL — escalate to ledger-audit.

## 8. Demo verification

Per memory `feedback_demo_quality_gate.md`:

```bash
$ make demo-down -v
$ export OPENAI_API_KEY=...  # source ~/.env
$ DEMO_MODE=proxy make demo-up
[proxy-smoke] step 0: proxy /healthz + /readyz... OK
[proxy-smoke] step 1 (CONTINUE): small OpenAI call via proxy...
[proxy-smoke]   CONTINUE OK
[proxy-smoke] step 2 (STOP): force STOP via huge X-SpendGuard-Estimated-Tokens...
[proxy-smoke]   STOP OK
[proxy-smoke] step 4 (STREAMING): real OpenAI streaming call via proxy...
[proxy-smoke]   STREAMING OK; usage block + [DONE] present
[proxy-smoke] PASS — auto-instrument egress proxy v0.2 (streaming SSE) verified
```

Plus a ledger spot-check:

```sql
SELECT operation_kind, COUNT(*)
FROM ledger_transactions
WHERE recorded_at > now() - interval '5 minutes'
GROUP BY operation_kind;
-- Expected:
--   reserve          | 3   (3 PRE-reservations: continue, stop, streaming)
--   commit_estimated | 2   (continue + streaming committed)
--   denied_decision  | 1   (stop)
```

## 9. Deferred items (NOT shipped in v0.2 streaming)

- Anthropic SSE pass-through (different event shape; messages API not chat.completions).
- AWS Bedrock streaming (different transport entirely — eventstream protocol).
- Reservation TTL bump for long-running streams (>60s). Workaround: client sets `X-SpendGuard-Reservation-TTL` header.
- Streaming usage in `openai-agents` `Runner.run_streamed`'s multi-step flows — each tool call is a separate proxy round trip; no cross-step aggregation. Single-call streaming works.
- Prometheus metric for `egress_proxy_usage_injected_total` — log-only for v0.2 if no existing metrics registry plumbing.
- SSE `event:` field handling (OpenAI doesn't use it; some other providers do).

## 10. References

- Memory `project_overview.md` "Auto-instrument egress proxy v0.1" — original spec + v0.1 slice list
- `docs/specs/auto-instrument-egress-proxy-spec.md` §4.1 / §4.4 — commit + release lanes the streaming path reuses
- `services/egress_proxy/src/forward.rs:130-170` — current `ForwardError::StreamingUnsupported` definition
- `services/egress_proxy/src/forward.rs:533+` — existing `parse_usage_tokens` + `commit_on_success` patterns
- OpenAI Platform docs — `stream_options.include_usage` (added 2024-06)
- W3C SSE spec — event boundary semantics
