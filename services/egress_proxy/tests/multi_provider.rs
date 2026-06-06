//! SLICE_11 Phase E — Multi-provider integration tests.
//!
//! These tests exercise the SLICE_11 routing + per-provider extractor
//! contracts end-to-end at the binary-crate boundary. They drive:
//!
//!   * inbound path → ProviderConfig lookup,
//!   * upstream URL templating (Bedrock / Vertex / Azure substitution),
//!   * per-provider tokenizer kind selection,
//!   * per-provider usage extraction round-trip from real vendor
//!     response samples.
//!
//! Real-network integration tests against live OpenAI / Anthropic /
//! Bedrock / Vertex / Azure endpoints are gated by environment
//! variables and run only in operator-controlled CI environments
//! (slice §8.2 "Real provider request → forward correctly"). The
//! tests here cover the deterministic paths that don't require
//! provider keys, exercising the same code paths as production would.
//!
//! Note: this file lives under `tests/` so it loads via the
//! egress_proxy crate's INTERNAL `bin` target — i.e. it tests the
//! library surface that `tests/decision_tests.rs` already uses
//! (placeholder pattern documented there). For modules that need
//! crate-internal access we use `#[path]` to inline-mirror the
//! routing.rs + providers/*.rs surface. The duplication is acceptable
//! because cargo refuses to link a `[[bin]]` crate from an external
//! test target.

#[path = "../src/routing.rs"]
#[allow(dead_code)]
mod routing;

#[path = "../src/providers/mod.rs"]
#[allow(dead_code)]
mod providers;

use serde_json::json;

// ─── SLICE_11 §8.2 integration coverage ──────────────────────────────

#[test]
fn openai_chat_completions_round_trip() {
    // Per slice §8.2 — "Real OpenAI request → forward correctly + audit
    // row provider=openai". This test exercises the deterministic part
    // of that flow: inbound path routes to OpenAI, upstream URL is the
    // OpenAI host, usage extraction works on a real response sample.
    let cfg = routing::route("/v1/chat/completions").expect("OpenAI must route");
    assert_eq!(cfg.kind, routing::ProviderKind::OpenAi);
    assert_eq!(
        cfg.upstream_url_for("/v1/chat/completions"),
        "https://api.openai.com/v1/chat/completions"
    );
    // Real-shape OpenAI chat completion response (from openai-python
    // test fixtures).
    let resp = json!({
        "id": "chatcmpl-9abc",
        "object": "chat.completion",
        "model": "gpt-4o-mini-2024-07-18",
        "choices": [{"message": {"role": "assistant", "content": "Hi!"}}],
        "usage": {"prompt_tokens": 14, "completion_tokens": 3, "total_tokens": 17}
    });
    let usage = (cfg.usage_extractor)(&resp);
    assert_eq!(usage.total_tokens, 17);
    assert_eq!(usage.input_tokens, 14);
    assert_eq!(usage.output_tokens, 3);
}

#[test]
fn anthropic_messages_round_trip_with_anthropic_tokenizer() {
    // Per slice §8.2 — "Real Anthropic Messages → forward + audit row
    // provider=anthropic + Anthropic BPE tokenizer".
    let cfg = routing::route("/v1/messages").expect("Anthropic must route");
    assert_eq!(cfg.kind, routing::ProviderKind::Anthropic);
    assert_eq!(
        cfg.tokenizer_kind,
        spendguard_tokenizer::EncoderKind::Anthropic
    );
    assert_eq!(
        cfg.upstream_url_for("/v1/messages"),
        "https://api.anthropic.com/v1/messages"
    );
    let resp = json!({
        "id": "msg_01EhPAB",
        "type": "message",
        "model": "claude-3-5-sonnet-20241022",
        "usage": {
            "input_tokens": 18,
            "output_tokens": 10,
            "cache_creation_input_tokens": 250,
            "cache_read_input_tokens": 100,
        }
    });
    let usage = (cfg.usage_extractor)(&resp);
    assert_eq!(usage.input_tokens, 18);
    assert_eq!(usage.output_tokens, 10);
    // Slice §6 — cache tokens surface separately for billing differentiation.
    assert_eq!(usage.cache_creation_input_tokens, 250);
    assert_eq!(usage.cache_read_input_tokens, 100);
}

#[test]
fn bedrock_claude_3_5_sonnet_routes_to_anthropic_tokenizer() {
    // Per slice §8.2 — "Real Bedrock anthropic.claude-3-5-sonnet-* →
    // forward + Anthropic BPE".
    let path = "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/invoke";
    let cfg = routing::route(path).expect("Bedrock must route");
    assert_eq!(cfg.kind, routing::ProviderKind::Bedrock);
    assert_eq!(cfg.request_shape, routing::RequestShape::BedrockInvokeModel);

    // Per-model tokenizer dispatch (Phase C wiring).
    let body = json!({});
    assert_eq!(
        routing::resolve_tokenizer_kind(cfg, path, &body),
        Some(spendguard_tokenizer::EncoderKind::Anthropic)
    );

    // URL templating: model id substituted in.
    let url = cfg.upstream_url_for(path);
    assert!(url.contains("anthropic.claude-3-5-sonnet-20240620-v1:0"));
    assert!(url.contains("bedrock-runtime"));

    // Usage extraction — Bedrock passes Anthropic Messages shape.
    let resp = json!({
        "id": "msg_bdrock_1",
        "model": "claude-3-5-sonnet-20240620",
        "usage": {"input_tokens": 30, "output_tokens": 45}
    });
    let usage = (cfg.usage_extractor)(&resp);
    assert_eq!(usage.input_tokens, 30);
    assert_eq!(usage.output_tokens, 45);
}

#[test]
fn bedrock_meta_llama3_routes_to_llama_tokenizer() {
    let path = "/model/us.meta.llama3-1-70b-instruct-v1:0/invoke";
    let cfg = routing::route(path).expect("Bedrock llama must route");
    let body = json!({});
    assert_eq!(
        routing::resolve_tokenizer_kind(cfg, path, &body),
        Some(spendguard_tokenizer::EncoderKind::Llama)
    );
}

#[test]
fn vertex_generate_content_routes_to_gemini_tokenizer() {
    // Per slice §8.2 — "Real Vertex generateContent → forward + Gemini BPE".
    let path =
        "/v1/projects/my-proj/locations/us-central1/publishers/google/models/gemini-1.5-pro:generateContent";
    let cfg = routing::route(path).expect("Vertex must route");
    assert_eq!(cfg.kind, routing::ProviderKind::Vertex);
    assert_eq!(
        cfg.tokenizer_kind,
        spendguard_tokenizer::EncoderKind::Gemini
    );
    let url = cfg.upstream_url_for(path);
    assert!(url.contains("my-proj"));
    assert!(url.contains("us-central1"));
    assert!(url.contains("gemini-1.5-pro"));

    let resp = json!({
        "candidates": [{"content": {"parts": [{"text": "ok"}]}}],
        "usageMetadata": {
            "promptTokenCount": 10,
            "candidatesTokenCount": 5,
            "totalTokenCount": 15,
        }
    });
    let usage = (cfg.usage_extractor)(&resp);
    assert_eq!(usage.input_tokens, 10);
    assert_eq!(usage.output_tokens, 5);
    assert_eq!(usage.total_tokens, 15);
}

#[test]
fn azure_openai_deployment_routes_with_deployment_id() {
    // Per slice §9.8 review question 8 — "Azure OpenAI: deployment_id
    // routing handled?".
    let path = "/openai/deployments/my-gpt4o-deployment/chat/completions";
    let cfg = routing::route(path).expect("Azure must route");
    assert_eq!(cfg.kind, routing::ProviderKind::AzureOpenAi);
    let url = cfg.upstream_url_for(path);
    assert!(url.contains("my-gpt4o-deployment"));
}

#[test]
fn cross_tenant_routing_isolation() {
    // Per slice §8.2 — "Cross-tenant routing: tenant A's openai key
    // cannot be used for tenant B's call".
    //
    // The routing table is shared at process-scope (Lazy<Vec<_>>), but
    // tenant attribution is gated upstream of routing by forward.rs's
    // TenantSpoofing check (X-SpendGuard-Tenant-Id ≠ proxy startup
    // tenant → 403). This test pins that the routing table itself is
    // tenant-agnostic (so the per-tenant key bind is the right
    // enforcement layer — not routing) and that ProviderConfig
    // references are read-only.
    let cfg_a = routing::route("/v1/chat/completions").unwrap();
    let cfg_b = routing::route("/v1/chat/completions").unwrap();
    assert!(
        std::ptr::eq(cfg_a as *const _, cfg_b as *const _),
        "ROUTING_TABLE must be process-scoped (same reference for both lookups)"
    );
}

#[test]
fn unknown_inbound_path_returns_none() {
    // Per slice §7 "Unknown inbound path | 404 + unknown_inbound_path metric".
    assert!(routing::route("/v1/some/unknown/path").is_none());
    // The metric is emitted by forward.rs; this test pins the
    // routing-table-level contract.
}

#[test]
fn backward_compat_openai_default_paths_unchanged() {
    // Per slice §8.5 — "Existing OpenAI / Chat Completions / Responses
    // API requests unaffected by behavior".
    //
    // Before SLICE_11 the proxy hard-coded
    // https://api.openai.com/v1/chat/completions and
    // https://api.openai.com/v1/responses. The routing table's first
    // two rows MUST resolve to identical URLs so the v0.1 + v0.3 demo
    // paths see zero behavior change.
    let cfg = routing::route("/v1/chat/completions").unwrap();
    assert_eq!(
        cfg.upstream_url_for("/v1/chat/completions"),
        "https://api.openai.com/v1/chat/completions"
    );
    let cfg = routing::route("/v1/responses").unwrap();
    assert_eq!(
        cfg.upstream_url_for("/v1/responses"),
        "https://api.openai.com/v1/responses"
    );
}

#[test]
fn five_providers_all_route() {
    // Sanity test: SLICE_11 covers exactly 5 providers (OpenAI,
    // Anthropic, Bedrock, Vertex, Azure OpenAI). The routing table
    // includes the OpenAI Responses API endpoint as a separate row
    // (same provider, different request shape) so the table has 6+ rows.
    assert!(routing::route("/v1/chat/completions").is_some());
    assert!(routing::route("/v1/responses").is_some());
    assert!(routing::route("/v1/messages").is_some());
    assert!(routing::route("/model/anthropic.claude-3-5-sonnet-20240620-v1:0/invoke").is_some());
    assert!(routing::route(
        "/v1/projects/p/locations/us/publishers/google/models/gemini-1.5-pro:generateContent"
    )
    .is_some());
    assert!(routing::route("/openai/deployments/foo/chat/completions").is_some());
}

#[test]
fn concurrent_routing_lookups_thread_safe() {
    // Per slice §9.9 review question 9 — "Concurrent requests for
    // different providers: routing table thread-safe?".
    //
    // ROUTING_TABLE is Lazy<Vec<_>> + immutable after init →
    // shareable across threads without sync primitives. Pin this
    // with a 50-thread storm.
    use std::sync::Arc;
    use std::thread;

    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut handles = Vec::new();
    for i in 0..50 {
        let c = counter.clone();
        handles.push(thread::spawn(move || {
            // Mix of paths across all providers.
            let paths = [
                "/v1/chat/completions",
                "/v1/messages",
                "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/invoke",
                "/openai/deployments/x/chat/completions",
            ];
            let p = paths[i % paths.len()];
            if routing::route(p).is_some() {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 50);
}

// ─── SLICE_11 §8.4 demo-mode regression ──────────────────────────────

#[test]
fn multi_provider_demo_routing_works_for_all_five() {
    // Per slice §8.4 — "`make demo-up DEMO_MODE=multi_provider_usd`
    // shows correct provider-routed handling".
    //
    // The integration here verifies that one router can produce 5
    // distinct provider configs in a single process, all returning
    // different ProviderKind enums but sharing the same routing table.
    // (The demo-side runner is in deploy/demo; that wiring is
    // separately exercised when the makefile runs.)
    let configs = vec![
        ("/v1/chat/completions", routing::ProviderKind::OpenAi),
        ("/v1/messages", routing::ProviderKind::Anthropic),
        (
            "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/invoke",
            routing::ProviderKind::Bedrock,
        ),
        (
            "/v1/projects/p/locations/us/publishers/google/models/gemini-1.5-pro:generateContent",
            routing::ProviderKind::Vertex,
        ),
        (
            "/openai/deployments/d/chat/completions",
            routing::ProviderKind::AzureOpenAi,
        ),
    ];
    for (path, expected_kind) in configs {
        let cfg = routing::route(path).unwrap_or_else(|| panic!("path must route: {path}"));
        assert_eq!(cfg.kind, expected_kind, "wrong provider for path: {path}");
    }
}

#[test]
fn anthropic_cache_tokens_split_per_spec_section_6() {
    // Per slice §9.6 review question 6 — "Anthropic
    // cache_creation_input_tokens / cache_read_input_tokens: treated
    // separately in audit?".
    let cfg = routing::route("/v1/messages").unwrap();
    let resp = json!({
        "usage": {
            "input_tokens": 50,
            "output_tokens": 100,
            "cache_creation_input_tokens": 1000,
            "cache_read_input_tokens": 500,
        }
    });
    let u = (cfg.usage_extractor)(&resp);
    // Per slice §6 — surfaced separately so audit row can bill cache
    // tokens at the right rate.
    assert_eq!(u.cache_creation_input_tokens, 1000);
    assert_eq!(u.cache_read_input_tokens, 500);
    // total_for_commit is input + output only (cache tokens billed
    // separately in audit).
    assert_eq!(u.total_for_commit(), 150);
}
