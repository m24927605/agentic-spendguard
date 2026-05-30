//! SLICE_11 — Multi-provider routing table.
//!
//! Replaces the SLICE_03 hard-coded `UPSTREAM_URL_CHAT_COMPLETIONS` +
//! `UPSTREAM_URL_RESPONSES` constants at `forward.rs:36-37` with a
//! provider-routing table. Each inbound HTTP path is matched against
//! one [`ProviderConfig`] which dictates:
//!
//!   * the upstream URL template,
//!   * the on-wire request shape ([`RequestShape`]),
//!   * the in-process tokenizer kind ([`EncoderKind`]),
//!   * a per-provider `usage_extractor` function that pulls
//!     `(input_tokens, output_tokens, total_tokens)` out of a non-
//!     streaming JSON response.
//!
//! ## §3.1 cross-cutting (per `tokenizer-service-spec-v1alpha1.md`)
//!
//! Bedrock model dispatch reuses the SLICE_04 narrow patterns +
//! cross-region prefix `(?:[a-z][a-z0-9-]*\.)?` (locked by SLICE_04
//! R2 B1). See [`providers::bedrock::dispatch_tokenizer_kind`].
//!
//! ## §9.10 — unknown inbound path
//!
//! [`route`] returns `None`; the caller maps that to a 404 + the
//! `unknown_inbound_path` metric per spec §9.10. The metric itself is
//! emitted by `forward.rs`.

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use spendguard_tokenizer::EncoderKind;

use crate::providers;

/// The set of providers the proxy can route to. SLICE_11 ships five;
/// SLICE_10's out-of-scope deferral table forbids adding new providers
/// in this slice (see §3 of the slice doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Bedrock,
    Vertex,
    AzureOpenAi,
}

impl ProviderKind {
    /// Stable identifier surfacing in `cloudevent_payload.provider`
    /// per slice doc §6.
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

/// Wire shape of the upstream request. SLICE_11 doesn't transform
/// request bodies (proxy stays byte-identical per spec §3.4), but
/// the proxy DOES need to know the shape to decide:
///
///   * whether `stream_options.include_usage` auto-injection applies
///     (Chat Completions only),
///   * which SSE event-shape parser to use,
///   * which JSON path holds the model id (Bedrock embeds it in the
///     URL, not the body).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestShape {
    /// OpenAI v1 Chat Completions JSON.
    /// Body: `{"model": "...", "messages": [...], "stream": ...}`.
    OpenAiChatCompletions,

    /// OpenAI Responses API.
    /// Body: `{"model": "...", "input": ...}`.
    OpenAiResponses,

    /// Anthropic Messages API.
    /// Body: `{"model": "...", "messages": [...], "max_tokens": ...}`.
    AnthropicMessages,

    /// AWS Bedrock InvokeModel. The model id is in the URL path
    /// (`/model/{model_id}/invoke`), NOT the body. The body shape is
    /// per-vendor: anthropic.claude-* uses the Anthropic Messages
    /// shape, cohere.command-* uses Cohere, meta.llama* uses Llama.
    BedrockInvokeModel,

    /// Vertex AI generateContent.
    /// URL: `/v1/projects/{project}/locations/{loc}/publishers/google/models/{model}:generateContent`.
    VertexGenerateContent,

    /// Azure OpenAI deployments.
    /// URL: `/openai/deployments/{deployment}/chat/completions`. The
    /// "model" is identified by the deployment id (operator-assigned
    /// alias), not by the body's `model` field — body shape matches
    /// OpenAI Chat Completions.
    AzureOpenAiChatCompletions,
}

/// Per-call usage breakdown from an upstream JSON response. Fields
/// are signed because providers occasionally return -1 sentinels for
/// "unknown"; non-negative values are validated by the caller.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageMetrics {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    /// Anthropic-specific (slice doc §6): cache write tokens count as
    /// input but are billed at a different rate. SLICE_11 surfaces them
    /// alongside the headline `input_tokens` so a future audit row
    /// extension can split them out without re-extracting.
    pub cache_creation_input_tokens: i64,
    /// Anthropic-specific (slice doc §6): cache read tokens count as
    /// input but are billed at a different rate. Same surfacing
    /// rationale as `cache_creation_input_tokens`.
    pub cache_read_input_tokens: i64,
}

impl UsageMetrics {
    /// `total_tokens` for the egress_proxy commit-lane. Falls back to
    /// `input + output` when the provider didn't report a total
    /// (e.g. some Bedrock responses).
    pub fn total_for_commit(&self) -> i64 {
        if self.total_tokens > 0 {
            self.total_tokens
        } else {
            self.input_tokens.saturating_add(self.output_tokens)
        }
    }
}

/// One row in the [`ROUTING_TABLE`]. Each row binds:
///
///   * an inbound `path_pattern` regex (anchored),
///   * the upstream URL template (with `{...}` substitutions filled
///     by `upstream_url_for(...)`),
///   * the on-wire request shape,
///   * the in-process tokenizer kind to charge,
///   * a `usage_extractor` for the non-streaming commit path.
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
    /// Render the upstream URL for an inbound path. Most providers use
    /// a static `upstream_url_template`; Bedrock + Vertex + Azure
    /// embed identifiers (model id / project / deployment) in the
    /// path so the proxy substitutes them in.
    ///
    /// SLICE_11 v1 keeps the template renderer narrow: the inbound
    /// `path` is captured by `inbound_path_pattern` and the matched
    /// groups are spliced into the upstream template using `{0}`,
    /// `{1}`, ... placeholders. Providers without dynamic segments
    /// (OpenAI, Anthropic) simply ignore the captures.
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

/// Build the routing table. Compiled once at process start via
/// [`ROUTING_TABLE`].
fn build_routing_table() -> Vec<ProviderConfig> {
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
            usage_extractor: providers::openai::extract_usage,
        },
        // Responses API (v0.3)
        ProviderConfig {
            kind: ProviderKind::OpenAi,
            inbound_path_pattern: Regex::new(r"^/v1/responses$")
                .expect("openai responses path regex"),
            upstream_url_template: "https://api.openai.com/v1/responses",
            request_shape: RequestShape::OpenAiResponses,
            tokenizer_kind: EncoderKind::OpenAi,
            usage_extractor: providers::openai::extract_usage,
        },
        // ─── Anthropic ────────────────────────────────────────────────
        ProviderConfig {
            kind: ProviderKind::Anthropic,
            inbound_path_pattern: Regex::new(r"^/v1/messages$")
                .expect("anthropic messages path regex"),
            upstream_url_template: "https://api.anthropic.com/v1/messages",
            request_shape: RequestShape::AnthropicMessages,
            tokenizer_kind: EncoderKind::Anthropic,
            usage_extractor: providers::anthropic::extract_usage,
        },
        // ─── Bedrock InvokeModel ──────────────────────────────────────
        // Inbound: /model/{model_id}/invoke — the proxy preserves this
        // path on the upstream call. AWS SigV4 is the client's
        // responsibility (proxy forwards the Authorization header
        // byte-identical per spec §3.4). The `{0}` capture is the
        // model_id used by [`providers::bedrock::dispatch_tokenizer_kind`]
        // to pick the right per-vendor BPE encoder.
        ProviderConfig {
            kind: ProviderKind::Bedrock,
            inbound_path_pattern: Regex::new(r"^/model/([^/]+)/invoke$")
                .expect("bedrock invoke path regex"),
            // Bedrock region is in the host, not the path — the proxy
            // forwards verbatim. The `{0}` is the model id; v1 ships a
            // single bedrock host placeholder, operators override the
            // base URL via env (SPENDGUARD_PROXY_BEDROCK_HOST_OVERRIDE)
            // in production for region pinning. v1 default is the
            // us-east-1 host.
            upstream_url_template: "https://bedrock-runtime.us-east-1.amazonaws.com/model/{0}/invoke",
            request_shape: RequestShape::BedrockInvokeModel,
            // Tokenizer kind is dispatched per-model at usage-extract
            // time via `providers::bedrock::dispatch_tokenizer_kind`.
            // The routing-table-level kind is a sentinel (OpenAi)
            // overridden by Phase C's wiring in decision.rs.
            tokenizer_kind: EncoderKind::Anthropic, // most common Bedrock vendor; per-model override below
            usage_extractor: providers::bedrock::extract_usage,
        },
        // ─── Vertex AI generateContent ────────────────────────────────
        // Inbound matches Google's documented endpoint shape:
        //   /v1/projects/{project}/locations/{loc}/publishers/google/models/{model}:generateContent
        // The proxy forwards verbatim; client supplies the GCP
        // bearer token via Authorization header (spec §3.4 byte-identical).
        ProviderConfig {
            kind: ProviderKind::Vertex,
            inbound_path_pattern: Regex::new(
                r"^/v1/projects/([^/]+)/locations/([^/]+)/publishers/google/models/([^:]+):generateContent$",
            )
            .expect("vertex generateContent path regex"),
            upstream_url_template:
                "https://generativelanguage.googleapis.com/v1/projects/{0}/locations/{1}/publishers/google/models/{2}:generateContent",
            request_shape: RequestShape::VertexGenerateContent,
            tokenizer_kind: EncoderKind::Gemini,
            usage_extractor: providers::vertex::extract_usage,
        },
        // ─── Azure OpenAI ─────────────────────────────────────────────
        // Inbound: /openai/deployments/{deployment}/chat/completions
        // Body shape == OpenAI Chat Completions; usage shape ==
        // OpenAI. The "model" identification is via the deployment id
        // alias — the operator maps deployment → model in Azure
        // portal, the proxy treats the deployment string as opaque
        // and charges via the cl100k_base / o200k_base encoder per
        // the model declared in the body (which Azure-hosted
        // deployments echo verbatim).
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
            usage_extractor: providers::azure_openai::extract_usage,
        },
    ]
}

/// Process-global routing table. Compiled once on first access;
/// `Lazy<Vec<_>>` is thread-safe via `once_cell::sync`.
pub static ROUTING_TABLE: Lazy<Vec<ProviderConfig>> = Lazy::new(build_routing_table);

/// First-match lookup. Returns `None` for paths the table doesn't
/// know — caller (`forward.rs`) maps that to 404 + the
/// `unknown_inbound_path` metric per spec §9.10.
pub fn route(path: &str) -> Option<&'static ProviderConfig> {
    ROUTING_TABLE
        .iter()
        .find(|cfg| cfg.inbound_path_pattern.is_match(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_openai_chat_completions() {
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
        let cfg = route("/v1/responses").expect("openai responses must route");
        assert_eq!(cfg.kind, ProviderKind::OpenAi);
        assert_eq!(cfg.request_shape, RequestShape::OpenAiResponses);
    }

    #[test]
    fn routes_anthropic_messages() {
        let cfg = route("/v1/messages").expect("anthropic must route");
        assert_eq!(cfg.kind, ProviderKind::Anthropic);
        assert_eq!(cfg.request_shape, RequestShape::AnthropicMessages);
        assert_eq!(cfg.tokenizer_kind, EncoderKind::Anthropic);
    }

    #[test]
    fn routes_bedrock_invoke() {
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
        // SLICE_04 R2 B1 — cross-region inference profile prefix.
        let path = "/model/us.anthropic.claude-3-5-sonnet-20240620-v1:0/invoke";
        let cfg = route(path).expect("bedrock cross-region must route");
        assert_eq!(cfg.kind, ProviderKind::Bedrock);
        let url = cfg.upstream_url_for(path);
        assert!(url.contains("us.anthropic.claude-3-5-sonnet-20240620-v1:0"));
    }

    #[test]
    fn routes_vertex_generate_content() {
        let path =
            "/v1/projects/my-proj/locations/us-central1/publishers/google/models/gemini-1.5-pro:generateContent";
        let cfg = route(path).expect("vertex must route");
        assert_eq!(cfg.kind, ProviderKind::Vertex);
        assert_eq!(cfg.tokenizer_kind, EncoderKind::Gemini);

        let url = cfg.upstream_url_for(path);
        assert!(url.contains("my-proj"));
        assert!(url.contains("us-central1"));
        assert!(url.contains("gemini-1.5-pro"));
    }

    #[test]
    fn routes_azure_openai_deployments() {
        let path = "/openai/deployments/my-gpt4o/chat/completions";
        let cfg = route(path).expect("azure openai must route");
        assert_eq!(cfg.kind, ProviderKind::AzureOpenAi);
        assert_eq!(cfg.tokenizer_kind, EncoderKind::OpenAi);
        let url = cfg.upstream_url_for(path);
        assert!(url.contains("my-gpt4o"));
    }

    #[test]
    fn unknown_path_returns_none() {
        assert!(route("/unknown/path").is_none());
        assert!(route("/v2/foo").is_none());
        assert!(route("/").is_none());
        // Bedrock InvokeAsync / Converse aren't in the routing table
        // — they fall through to 404 + the unknown_inbound_path metric.
        // SLICE_11 §3 explicitly defers those to "post-launch enhancement".
        assert!(route("/model/foo/converse").is_none());
        assert!(route("/model/foo/invoke-with-response-stream").is_none());
    }

    #[test]
    fn provider_kind_string_stable() {
        // CloudEvent payload schema depends on these strings; SLICE_11
        // contract pins them so future renames break this test loudly.
        assert_eq!(ProviderKind::OpenAi.as_str(), "openai");
        assert_eq!(ProviderKind::Anthropic.as_str(), "anthropic");
        assert_eq!(ProviderKind::Bedrock.as_str(), "bedrock");
        assert_eq!(ProviderKind::Vertex.as_str(), "vertex");
        assert_eq!(ProviderKind::AzureOpenAi.as_str(), "azure_openai");
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
}
