//! SpendGuard provider routing — shared crate.
//!
//! Extracted from `services/egress_proxy/src/routing.rs` at COV_01 so
//! both `egress_proxy` and the new `services/envoy_extproc` adapter share
//! one routing table. Per [`docs/specs/coverage/D01_envoy_extproc/implementation.md`]
//! §4 the per-provider response `usage_extractor` functions remain in
//! `services/egress_proxy/src/providers/` and are **registered** here at
//! startup via [`init_extractors`]; the shared crate has no dependency on
//! provider-specific code.
//!
//! ## Behaviour vs the legacy in-tree module
//!
//! The public surface mirrors the legacy module verbatim so existing
//! consumers compile against the re-export shim at
//! `services/egress_proxy/src/routing.rs` with no behaviour change. The
//! routing-table tests live in this crate as the canonical source of
//! truth; `routes_*` tests in egress_proxy were moved here at COV_01.
//!
//! ### Bedrock dispatch
//!
//! Bedrock model dispatch is split from `services/egress_proxy/src/providers/bedrock.rs`
//! into [`bedrock::dispatch_tokenizer_kind`] because the dispatch is pure
//! (regex + `EncoderKind` discriminator) and is needed by both consumers.
//! The per-provider response `extract_usage` shapes stay in egress_proxy.

use std::sync::OnceLock;

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use spendguard_tokenizer::EncoderKind;

// ============================================================================
// Public types (verbatim from services/egress_proxy/src/routing.rs:37-149)
// ============================================================================

/// The set of providers the proxy can route to. SLICE_11 ships five.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Bedrock,
    Vertex,
    AzureOpenAi,
}

impl ProviderKind {
    /// Stable identifier surfacing in `cloudevent_payload.provider`.
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::OpenAi => "openai",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Bedrock => "bedrock",
            ProviderKind::Vertex => "vertex",
            ProviderKind::AzureOpenAi => "azure_openai",
        }
    }
}

/// Wire shape of the upstream request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestShape {
    OpenAiChatCompletions,
    OpenAiResponses,
    AnthropicMessages,
    BedrockInvokeModel,
    VertexGenerateContent,
    AzureOpenAiChatCompletions,
}

/// Per-call usage breakdown from an upstream JSON response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageMetrics {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    /// Anthropic-specific: cache write tokens count as input but are
    /// billed at a different rate.
    pub cache_creation_input_tokens: i64,
    /// Anthropic-specific: cache read tokens count as input but are
    /// billed at a different rate.
    pub cache_read_input_tokens: i64,
}

impl UsageMetrics {
    pub fn total_for_commit(&self) -> i64 {
        if self.total_tokens > 0 {
            self.total_tokens
        } else {
            self.input_tokens.saturating_add(self.output_tokens)
        }
    }
}

/// One row in the [`routing_table`]. Each row binds an inbound path
/// pattern, the upstream URL template, the on-wire request shape, the
/// in-process tokenizer kind, and a per-provider `usage_extractor`
/// function registered by the binary at startup.
#[derive(Debug)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub inbound_path_pattern: Regex,
    pub upstream_url_template: &'static str,
    pub request_shape: RequestShape,
    pub tokenizer_kind: EncoderKind,
    pub usage_extractor: fn(&Value) -> UsageMetrics,
}

impl ProviderConfig {
    pub fn upstream_url_for(&self, path: &str) -> String {
        let captures = match self.inbound_path_pattern.captures(path) {
            Some(c) => c,
            None => return self.upstream_url_template.to_string(),
        };

        let mut url = self.upstream_url_template.to_string();
        for i in 1..captures.len() {
            let placeholder = format!("{{{}}}", i - 1);
            if let Some(m) = captures.get(i) {
                url = url.replace(&placeholder, m.as_str());
            }
        }
        url
    }
}

// ============================================================================
// Extractor registration (COV_01 split)
// ============================================================================

/// Per-provider response `usage_extractor` callbacks. The binary
/// registers its provider implementations exactly once at startup via
/// [`init_extractors`] before any [`route`] call.
#[derive(Debug, Clone, Copy)]
pub struct RoutingExtractors {
    pub openai: fn(&Value) -> UsageMetrics,
    pub anthropic: fn(&Value) -> UsageMetrics,
    pub bedrock: fn(&Value) -> UsageMetrics,
    pub vertex: fn(&Value) -> UsageMetrics,
    pub azure_openai: fn(&Value) -> UsageMetrics,
}

static EXTRACTORS: OnceLock<RoutingExtractors> = OnceLock::new();

/// Register the per-provider response `usage_extractor` callbacks.
///
/// MUST be called exactly once at process start, before [`route`] (or
/// any consumer of [`ProviderConfig::usage_extractor`]). Calling twice
/// returns an error so a misconfiguration is loud rather than racy.
///
/// Test-only: `#[cfg(test)]` callers can use [`init_extractors_for_test`]
/// which silently ignores re-registration.
pub fn init_extractors(extractors: RoutingExtractors) -> Result<(), &'static str> {
    EXTRACTORS
        .set(extractors)
        .map_err(|_| "spendguard_provider_routing::init_extractors called twice")
}

#[doc(hidden)]
pub fn init_extractors_for_test(extractors: RoutingExtractors) {
    let _ = EXTRACTORS.set(extractors);
}

/// Internal accessor — panics with a descriptive message if extractors
/// were never registered. The panic is preferable to a silent zero-token
/// commit because the only way `route()` succeeds without extractors is
/// a binary-startup bug.
fn registered_extractors() -> &'static RoutingExtractors {
    EXTRACTORS.get().expect(
        "spendguard_provider_routing: init_extractors() must be called at startup before route() / ROUTING_TABLE access. \
         See services/egress_proxy/src/main.rs and services/envoy_extproc/src/main.rs.",
    )
}

// ============================================================================
// Routing table (verbatim from services/egress_proxy/src/routing.rs:181-279)
// ============================================================================

fn build_routing_table() -> Vec<ProviderConfig> {
    let extractors = registered_extractors();
    vec![
        // ─── OpenAI ──────────────────────────────────────────────────
        // Chat Completions (v0.1)
        ProviderConfig {
            kind: ProviderKind::OpenAi,
            inbound_path_pattern: Regex::new(r"^/v1/chat/completions$")
                .expect("openai chat path regex"),
            upstream_url_template: "https://api.openai.com/v1/chat/completions",
            request_shape: RequestShape::OpenAiChatCompletions,
            tokenizer_kind: EncoderKind::OpenAi,
            usage_extractor: extractors.openai,
        },
        // Responses API (v0.3)
        ProviderConfig {
            kind: ProviderKind::OpenAi,
            inbound_path_pattern: Regex::new(r"^/v1/responses$")
                .expect("openai responses path regex"),
            upstream_url_template: "https://api.openai.com/v1/responses",
            request_shape: RequestShape::OpenAiResponses,
            tokenizer_kind: EncoderKind::OpenAi,
            usage_extractor: extractors.openai,
        },
        // ─── Anthropic ────────────────────────────────────────────────
        ProviderConfig {
            kind: ProviderKind::Anthropic,
            inbound_path_pattern: Regex::new(r"^/v1/messages$")
                .expect("anthropic messages path regex"),
            upstream_url_template: "https://api.anthropic.com/v1/messages",
            request_shape: RequestShape::AnthropicMessages,
            tokenizer_kind: EncoderKind::Anthropic,
            usage_extractor: extractors.anthropic,
        },
        // ─── Bedrock InvokeModel ──────────────────────────────────────
        ProviderConfig {
            kind: ProviderKind::Bedrock,
            inbound_path_pattern: Regex::new(r"^/model/([^/]+)/invoke$")
                .expect("bedrock invoke path regex"),
            upstream_url_template:
                "https://bedrock-runtime.us-east-1.amazonaws.com/model/{0}/invoke",
            request_shape: RequestShape::BedrockInvokeModel,
            tokenizer_kind: EncoderKind::Anthropic, // most common Bedrock vendor; per-model override
            usage_extractor: extractors.bedrock,
        },
        // ─── Vertex AI generateContent ────────────────────────────────
        ProviderConfig {
            kind: ProviderKind::Vertex,
            inbound_path_pattern: Regex::new(
                r"^/v1/projects/([^/]+)/locations/([^/]+)/publishers/google/models/([^:]+):generateContent$",
            )
            .expect("vertex generateContent path regex"),
            // The inbound pattern is the GCP-token-authenticated Vertex AI form
            // (captures project/location/model). The real Vertex host is
            // region-scoped: {location}-aiplatform.googleapis.com. The public
            // generativelanguage.googleapis.com host is the API-key Gemini route
            // (a separate provider row, added by the D02 Vertex/Gemini forwarding
            // work) and does NOT serve the /v1/projects/.../publishers/google/...
            // path. {1} (location) is interpolated into the host via the
            // whole-string replace in `upstream_url_for`.
            upstream_url_template:
                "https://{1}-aiplatform.googleapis.com/v1/projects/{0}/locations/{1}/publishers/google/models/{2}:generateContent",
            request_shape: RequestShape::VertexGenerateContent,
            tokenizer_kind: EncoderKind::Gemini,
            usage_extractor: extractors.vertex,
        },
        // ─── Azure OpenAI ─────────────────────────────────────────────
        ProviderConfig {
            kind: ProviderKind::AzureOpenAi,
            inbound_path_pattern: Regex::new(
                r"^/openai/deployments/([^/]+)/chat/completions$",
            )
            .expect("azure openai deployments path regex"),
            upstream_url_template:
                "https://AZURE_RESOURCE.openai.azure.com/openai/deployments/{0}/chat/completions",
            request_shape: RequestShape::AzureOpenAiChatCompletions,
            tokenizer_kind: EncoderKind::OpenAi,
            usage_extractor: extractors.azure_openai,
        },
    ]
}

/// Process-global routing table. Compiled once on first access AFTER
/// [`init_extractors`] has registered the per-provider response
/// callbacks.
pub static ROUTING_TABLE: Lazy<Vec<ProviderConfig>> = Lazy::new(build_routing_table);

/// First-match lookup. Returns `None` for paths the table doesn't know.
pub fn route(path: &str) -> Option<&'static ProviderConfig> {
    ROUTING_TABLE
        .iter()
        .find(|cfg| cfg.inbound_path_pattern.is_match(path))
}

/// Resolve the upstream model identifier for a request given the
/// inbound path + parsed body.
pub fn resolve_model_id(cfg: &ProviderConfig, path: &str, body: &Value) -> String {
    match cfg.request_shape {
        RequestShape::BedrockInvokeModel => cfg
            .inbound_path_pattern
            .captures(path)
            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
            .unwrap_or_else(|| "unknown".to_string()),
        _ => body
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
    }
}

/// Resolve the tokenizer kind for a request.
pub fn resolve_tokenizer_kind(
    cfg: &ProviderConfig,
    path: &str,
    body: &Value,
) -> Option<EncoderKind> {
    match cfg.request_shape {
        RequestShape::BedrockInvokeModel => {
            let model_id = resolve_model_id(cfg, path, body);
            bedrock::dispatch_tokenizer_kind(&model_id)
        }
        _ => Some(cfg.tokenizer_kind),
    }
}

// ============================================================================
// Bedrock dispatch (extracted from services/egress_proxy/src/providers/bedrock.rs:53-115)
// ============================================================================

pub mod bedrock {
    //! Bedrock per-model tokenizer dispatch. Pure (no body access);
    //! egress_proxy + envoy_extproc both consume.

    use super::*;
    use tracing::warn;

    /// SLICE_04 R2 B1 — Anthropic Claude 3.5 Bedrock dispatch
    /// (cross-region prefix permitted).
    static BEDROCK_ANTHROPIC_3_5: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"^(?:[a-z][a-z0-9-]*\.)?anthropic\.claude-3-5-(sonnet|haiku|opus)(-\d{8})?-v\d+:\d+$",
        )
        .expect("bedrock anthropic-3-5 regex")
    });
    static BEDROCK_ANTHROPIC_3: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"^(?:[a-z][a-z0-9-]*\.)?anthropic\.claude-3-(sonnet|haiku|opus)(-\d{8})?-v\d+:\d+$",
        )
        .expect("bedrock anthropic-3 regex")
    });
    static BEDROCK_COHERE_COMMAND_R: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^(?:[a-z][a-z0-9-]*\.)?cohere\.command(-r)?(-plus)?-v\d+:\d+$")
            .expect("bedrock cohere regex")
    });
    static BEDROCK_META_LLAMA3: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^(?:[a-z][a-z0-9-]*\.)?meta\.llama3(-\d+)?-\d+b-instruct-v\d+:\d+$")
            .expect("bedrock llama3 regex")
    });

    /// Bedrock-specific model dispatch — picks the correct tokenizer
    /// kind for a Bedrock model id. Returns `None` for unknown vendors;
    /// caller emits the `tokenizer_unknown_model` metric.
    pub fn dispatch_tokenizer_kind(model_id: &str) -> Option<EncoderKind> {
        // 3.5 BEFORE 3 (first-match-wins; SLICE_04 ordering rule).
        if BEDROCK_ANTHROPIC_3_5.is_match(model_id) {
            return Some(EncoderKind::Anthropic);
        }
        if BEDROCK_ANTHROPIC_3.is_match(model_id) {
            return Some(EncoderKind::Anthropic);
        }
        if BEDROCK_COHERE_COMMAND_R.is_match(model_id) {
            return Some(EncoderKind::Cohere);
        }
        if BEDROCK_META_LLAMA3.is_match(model_id) {
            return Some(EncoderKind::Llama);
        }
        warn!(model_id = %model_id, "bedrock unknown model; falling to Tier 3");
        None
    }
}

// ============================================================================
// Tests (ported verbatim from services/egress_proxy/src/routing.rs:351-561 +
//         the dispatch_tokenizer_kind tests from providers/bedrock.rs)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Test extractor — no-op. The routing tests don't probe
    /// `usage_extractor` so a constant is enough. Each test calls
    /// `init_extractors_for_test` so the OnceLock is populated; the
    /// helper deliberately ignores re-registration to match parallel
    /// test execution.
    fn noop_extractor(_: &Value) -> UsageMetrics {
        UsageMetrics::default()
    }

    fn install_test_extractors() {
        init_extractors_for_test(RoutingExtractors {
            openai: noop_extractor,
            anthropic: noop_extractor,
            bedrock: noop_extractor,
            vertex: noop_extractor,
            azure_openai: noop_extractor,
        });
    }

    #[test]
    fn routes_openai_chat_completions() {
        install_test_extractors();
        let cfg = route("/v1/chat/completions").expect("openai chat must route");
        assert_eq!(cfg.kind, ProviderKind::OpenAi);
        assert_eq!(cfg.request_shape, RequestShape::OpenAiChatCompletions);
        assert_eq!(cfg.tokenizer_kind, EncoderKind::OpenAi);
        assert_eq!(
            cfg.upstream_url_for("/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn routes_openai_responses() {
        install_test_extractors();
        let cfg = route("/v1/responses").expect("openai responses must route");
        assert_eq!(cfg.kind, ProviderKind::OpenAi);
        assert_eq!(cfg.request_shape, RequestShape::OpenAiResponses);
    }

    #[test]
    fn routes_anthropic_messages() {
        install_test_extractors();
        let cfg = route("/v1/messages").expect("anthropic must route");
        assert_eq!(cfg.kind, ProviderKind::Anthropic);
        assert_eq!(cfg.request_shape, RequestShape::AnthropicMessages);
        assert_eq!(cfg.tokenizer_kind, EncoderKind::Anthropic);
    }

    #[test]
    fn routes_bedrock_invoke() {
        install_test_extractors();
        let path = "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/invoke";
        let cfg = route(path).expect("bedrock invoke must route");
        assert_eq!(cfg.kind, ProviderKind::Bedrock);
        assert_eq!(cfg.request_shape, RequestShape::BedrockInvokeModel);

        let url = cfg.upstream_url_for(path);
        assert!(
            url.contains("anthropic.claude-3-5-sonnet-20240620-v1:0"),
            "URL should embed model id: {url}"
        );
        assert!(
            url.contains("bedrock-runtime"),
            "URL should be Bedrock host: {url}"
        );
    }

    #[test]
    fn routes_bedrock_with_cross_region_prefix() {
        install_test_extractors();
        let path = "/model/us.anthropic.claude-3-5-sonnet-20240620-v1:0/invoke";
        let cfg = route(path).expect("bedrock cross-region must route");
        assert_eq!(cfg.kind, ProviderKind::Bedrock);
        let url = cfg.upstream_url_for(path);
        assert!(url.contains("us.anthropic.claude-3-5-sonnet-20240620-v1:0"));
    }

    #[test]
    fn routes_vertex_generate_content() {
        install_test_extractors();
        let path =
            "/v1/projects/my-proj/locations/us-central1/publishers/google/models/gemini-1.5-pro:generateContent";
        let cfg = route(path).expect("vertex must route");
        assert_eq!(cfg.kind, ProviderKind::Vertex);
        assert_eq!(cfg.tokenizer_kind, EncoderKind::Gemini);

        let url = cfg.upstream_url_for(path);
        assert!(url.contains("my-proj"));
        assert!(url.contains("us-central1"));
        assert!(url.contains("gemini-1.5-pro"));
        // GCP-token Vertex traffic MUST go to the region-scoped aiplatform host,
        // NOT the public API-key Gemini host (generativelanguage.googleapis.com).
        // The {location} capture is interpolated into the host.
        assert!(
            url.contains("aiplatform"),
            "vertex must resolve to the region-scoped aiplatform host, got {url}"
        );
        assert!(
            url.starts_with("https://us-central1-aiplatform.googleapis.com/"),
            "vertex host must be region-scoped, got {url}"
        );
        assert!(
            !url.contains("generativelanguage.googleapis.com"),
            "GCP-token Vertex must not route to the public Gemini API-key host, got {url}"
        );
    }

    #[test]
    fn routes_azure_openai_deployments() {
        install_test_extractors();
        let path = "/openai/deployments/my-gpt4o/chat/completions";
        let cfg = route(path).expect("azure openai must route");
        assert_eq!(cfg.kind, ProviderKind::AzureOpenAi);
        assert_eq!(cfg.tokenizer_kind, EncoderKind::OpenAi);
        let url = cfg.upstream_url_for(path);
        assert!(url.contains("my-gpt4o"));
    }

    #[test]
    fn unknown_path_returns_none() {
        install_test_extractors();
        assert!(route("/unknown/path").is_none());
        assert!(route("/v2/foo").is_none());
        assert!(route("/").is_none());
        // Bedrock InvokeAsync / Converse aren't in the routing table.
        assert!(route("/model/foo/converse").is_none());
        assert!(route("/model/foo/invoke-with-response-stream").is_none());
    }

    #[test]
    fn provider_kind_string_stable() {
        // CloudEvent payload schema depends on these strings.
        assert_eq!(ProviderKind::OpenAi.as_str(), "openai");
        assert_eq!(ProviderKind::Anthropic.as_str(), "anthropic");
        assert_eq!(ProviderKind::Bedrock.as_str(), "bedrock");
        assert_eq!(ProviderKind::Vertex.as_str(), "vertex");
        assert_eq!(ProviderKind::AzureOpenAi.as_str(), "azure_openai");
    }

    #[test]
    fn resolve_model_id_openai_from_body() {
        install_test_extractors();
        let cfg = route("/v1/chat/completions").unwrap();
        let body = serde_json::json!({"model": "gpt-4o-mini-2024-07-18"});
        assert_eq!(
            resolve_model_id(cfg, "/v1/chat/completions", &body),
            "gpt-4o-mini-2024-07-18"
        );
    }

    #[test]
    fn resolve_model_id_bedrock_from_path() {
        install_test_extractors();
        let path = "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/invoke";
        let cfg = route(path).unwrap();
        let body = serde_json::json!({});
        assert_eq!(
            resolve_model_id(cfg, path, &body),
            "anthropic.claude-3-5-sonnet-20240620-v1:0"
        );
    }

    #[test]
    fn resolve_model_id_bedrock_cross_region() {
        install_test_extractors();
        let path = "/model/us.meta.llama3-1-70b-instruct-v1:0/invoke";
        let cfg = route(path).unwrap();
        let body = serde_json::json!({});
        assert_eq!(
            resolve_model_id(cfg, path, &body),
            "us.meta.llama3-1-70b-instruct-v1:0"
        );
    }

    #[test]
    fn resolve_tokenizer_kind_bedrock_anthropic() {
        install_test_extractors();
        let path = "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/invoke";
        let cfg = route(path).unwrap();
        let body = serde_json::json!({});
        assert_eq!(
            resolve_tokenizer_kind(cfg, path, &body),
            Some(EncoderKind::Anthropic)
        );
    }

    #[test]
    fn resolve_tokenizer_kind_bedrock_llama() {
        install_test_extractors();
        let path = "/model/us.meta.llama3-1-70b-instruct-v1:0/invoke";
        let cfg = route(path).unwrap();
        let body = serde_json::json!({});
        assert_eq!(
            resolve_tokenizer_kind(cfg, path, &body),
            Some(EncoderKind::Llama)
        );
    }

    #[test]
    fn resolve_tokenizer_kind_bedrock_unknown_returns_none() {
        install_test_extractors();
        // Pre-Claude-3 / pre-Llama-3 / amazon.titan-* etc. fall to Tier 3.
        let path = "/model/amazon.titan-text-express-v1/invoke";
        let cfg = route(path).unwrap();
        let body = serde_json::json!({});
        assert_eq!(resolve_tokenizer_kind(cfg, path, &body), None);
    }

    #[test]
    fn resolve_tokenizer_kind_openai_static() {
        install_test_extractors();
        let cfg = route("/v1/chat/completions").unwrap();
        let body = serde_json::json!({"model": "gpt-4o"});
        assert_eq!(
            resolve_tokenizer_kind(cfg, "/v1/chat/completions", &body),
            Some(EncoderKind::OpenAi)
        );
    }

    #[test]
    fn resolve_tokenizer_kind_vertex_static() {
        install_test_extractors();
        let path =
            "/v1/projects/p/locations/us/publishers/google/models/gemini-1.5-pro:generateContent";
        let cfg = route(path).unwrap();
        let body = serde_json::json!({});
        assert_eq!(
            resolve_tokenizer_kind(cfg, path, &body),
            Some(EncoderKind::Gemini)
        );
    }

    #[test]
    fn usage_metrics_total_for_commit_falls_back_to_sum() {
        let u = UsageMetrics {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 0,
            ..Default::default()
        };
        assert_eq!(u.total_for_commit(), 30);

        let u = UsageMetrics {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 50,
            ..Default::default()
        };
        assert_eq!(u.total_for_commit(), 50);
    }

    // ── Bedrock dispatch (moved from providers/bedrock.rs) ──────────

    #[test]
    fn bedrock_dispatch_anthropic_3_5() {
        assert_eq!(
            bedrock::dispatch_tokenizer_kind("anthropic.claude-3-5-sonnet-20240620-v1:0"),
            Some(EncoderKind::Anthropic)
        );
        assert_eq!(
            bedrock::dispatch_tokenizer_kind("anthropic.claude-3-5-haiku-20241022-v1:0"),
            Some(EncoderKind::Anthropic)
        );
    }

    #[test]
    fn bedrock_dispatch_anthropic_3() {
        assert_eq!(
            bedrock::dispatch_tokenizer_kind("anthropic.claude-3-sonnet-20240229-v1:0"),
            Some(EncoderKind::Anthropic)
        );
    }

    #[test]
    fn bedrock_dispatch_llama3() {
        assert_eq!(
            bedrock::dispatch_tokenizer_kind("meta.llama3-1-70b-instruct-v1:0"),
            Some(EncoderKind::Llama)
        );
        assert_eq!(
            bedrock::dispatch_tokenizer_kind("us.meta.llama3-1-70b-instruct-v1:0"),
            Some(EncoderKind::Llama)
        );
    }

    #[test]
    fn bedrock_dispatch_cohere() {
        assert_eq!(
            bedrock::dispatch_tokenizer_kind("cohere.command-r-v1:0"),
            Some(EncoderKind::Cohere)
        );
        assert_eq!(
            bedrock::dispatch_tokenizer_kind("cohere.command-r-plus-v1:0"),
            Some(EncoderKind::Cohere)
        );
    }

    #[test]
    fn bedrock_dispatch_unknown_returns_none() {
        // pre-Claude-3 / pre-Llama-3 / titan / non-AI variants.
        assert_eq!(
            bedrock::dispatch_tokenizer_kind("anthropic.claude-instant-v1"),
            None
        );
        assert_eq!(
            bedrock::dispatch_tokenizer_kind("meta.llama2-70b-chat-v1"),
            None
        );
        assert_eq!(
            bedrock::dispatch_tokenizer_kind("amazon.titan-text-express-v1"),
            None
        );
    }

    #[test]
    fn init_extractors_twice_returns_err() {
        // Note: in the test process this Lazy<EXTRACTORS> may already be
        // set by other tests using init_extractors_for_test. The
        // public init_extractors returns Err when called after Set; that
        // contract is the property under test. We assert the error path
        // by attempting to set twice using the public API on a fresh
        // OnceLock equivalent — here, since the test runner shares
        // process state, we test the documented behaviour: if EXTRACTORS
        // is already set (which it is via install_test_extractors above),
        // init_extractors returns Err.
        install_test_extractors();
        let result = init_extractors(RoutingExtractors {
            openai: noop_extractor,
            anthropic: noop_extractor,
            bedrock: noop_extractor,
            vertex: noop_extractor,
            azure_openai: noop_extractor,
        });
        assert!(
            result.is_err(),
            "second init_extractors must return Err to surface misconfig"
        );
    }
}
