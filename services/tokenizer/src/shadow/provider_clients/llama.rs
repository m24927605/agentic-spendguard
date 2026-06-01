//! Meta Llama Tier 1 count client via Amazon Bedrock Runtime CountTokens.
//!
//! Locked tokenizer spec §3.1/§3.4 routes Llama only for Bedrock
//! `meta.llama3-*-instruct-v1:0` model IDs. Use the official Bedrock
//! Runtime CountTokens API instead of hand-rolled SigV4:
//! <https://docs.aws.amazon.com/bedrock/latest/APIReference/API_runtime_CountTokens.html>
//!
//! Test fixtures use the HTTP-compatible backend so normal unit tests do
//! not require AWS credentials. Production boot uses the AWS SDK backend
//! when `SPENDGUARD_TOKENIZER_LLAMA_BEDROCK_REGION` is set.

use std::time::{Duration, Instant};

use aws_config::BehaviorVersion;
use aws_sdk_bedrockruntime::{
    config::{timeout::TimeoutConfig, Region},
    error::SdkError,
    operation::count_tokens::CountTokensError,
    types::{ContentBlock, ConversationRole, ConverseTokensRequest, CountTokensInput, Message},
    Client as BedrockClient,
};
use reqwest::{Client, StatusCode};
use serde_json::json;

use super::{ProviderCount, ProviderError};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct LlamaClient {
    backend: LlamaBackend,
}

#[derive(Clone)]
enum LlamaBackend {
    Bedrock {
        client: BedrockClient,
    },
    HttpCompat {
        http: Client,
        base_url: String,
        api_key: String,
    },
}

impl std::fmt::Debug for LlamaClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let backend = match &self.backend {
            LlamaBackend::Bedrock { .. } => "bedrock-sdk",
            LlamaBackend::HttpCompat { .. } => "http-compat",
        };
        f.debug_struct("LlamaClient")
            .field("backend", &backend)
            .finish()
    }
}

impl LlamaClient {
    pub async fn new_bedrock(
        region: impl Into<String>,
        endpoint_override: Option<String>,
    ) -> Result<Self, ProviderError> {
        let region = region.into();
        if region.trim().is_empty() {
            return Err(ProviderError::Auth("llama bedrock region empty".into()));
        }

        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(region))
            .load()
            .await;
        let mut builder = aws_sdk_bedrockruntime::config::Builder::from(&sdk_config)
            .timeout_config(
                TimeoutConfig::builder()
                    .operation_timeout(REQUEST_TIMEOUT)
                    .operation_attempt_timeout(REQUEST_TIMEOUT)
                    .build(),
            );
        if let Some(endpoint) = endpoint_override {
            if !endpoint.trim().is_empty() {
                builder = builder.endpoint_url(endpoint);
            }
        }

        Ok(Self {
            backend: LlamaBackend::Bedrock {
                client: BedrockClient::from_conf(builder.build()),
            },
        })
    }

    /// Test / private gateway backend. It speaks the Bedrock CountTokens
    /// wire shape but authenticates with a bearer token instead of SigV4,
    /// which keeps unit tests and signed internal gateways simple.
    pub fn with_base_url(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(ProviderError::Auth("llama api key empty".into()));
        }
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(Duration::from_secs(2))
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .user_agent(concat!(
                "spendguard-tokenizer/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/spendguard)"
            ))
            .build()
            .map_err(|e| ProviderError::Other(format!("build client: {e}")))?;
        Ok(Self {
            backend: LlamaBackend::HttpCompat {
                http,
                base_url: base_url.into(),
                api_key,
            },
        })
    }

    pub async fn count_tokens(
        &self,
        model: &str,
        text: &str,
    ) -> Result<ProviderCount, ProviderError> {
        match &self.backend {
            LlamaBackend::Bedrock { client } => count_tokens_bedrock(client, model, text).await,
            LlamaBackend::HttpCompat {
                http,
                base_url,
                api_key,
            } => count_tokens_http_compat(http, base_url, api_key, model, text).await,
        }
    }
}

async fn count_tokens_bedrock(
    client: &BedrockClient,
    model: &str,
    text: &str,
) -> Result<ProviderCount, ProviderError> {
    let message = Message::builder()
        .role(ConversationRole::User)
        .content(ContentBlock::Text(text.to_owned()))
        .build()
        .map_err(|e| ProviderError::Schema(format!("build bedrock converse message: {e}")))?;
    let input =
        CountTokensInput::Converse(ConverseTokensRequest::builder().messages(message).build());

    let start = Instant::now();
    let resp = client
        .count_tokens()
        .model_id(model)
        .input(input)
        .send()
        .await
        .map_err(map_bedrock_error)?;
    let latency = start.elapsed();
    let input_tokens = resp.input_tokens();
    if input_tokens < 0 {
        return Err(ProviderError::Schema(format!(
            "bedrock returned negative inputTokens: {input_tokens}"
        )));
    }
    Ok(ProviderCount {
        input_tokens: input_tokens as u64,
        request_id: None,
        latency,
    })
}

async fn count_tokens_http_compat(
    http: &Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    text: &str,
) -> Result<ProviderCount, ProviderError> {
    let url = format!(
        "{}/model/{}/count-tokens",
        base_url.trim_end_matches('/'),
        model
    );
    let body = json!({
        "input": {
            "converse": {
                "messages": [{
                    "role": "user",
                    "content": [{ "text": text }]
                }]
            }
        }
    });

    let start = Instant::now();
    let resp = http
        .post(&url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(map_send_error)?;
    let latency = start.elapsed();

    let status = resp.status();
    let request_id = resp
        .headers()
        .get("x-amzn-requestid")
        .or_else(|| resp.headers().get("x-request-id"))
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_owned());

    if status.is_success() {
        let parsed: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Schema(format!("read body: {e}")))?;
        let count = parsed
            .get("inputTokens")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ProviderError::Schema("missing or non-u64 `inputTokens`".into()))?;
        return Ok(ProviderCount {
            input_tokens: count,
            request_id,
            latency,
        });
    }

    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(30));
        return Err(ProviderError::RateLimit { retry_after });
    }
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(ProviderError::Auth(format!(
            "llama {} {}",
            status,
            body_summary(resp.text().await.unwrap_or_default())
        )));
    }
    if status == StatusCode::BAD_REQUEST {
        return Err(ProviderError::Schema(format!(
            "llama {} {}",
            status,
            body_summary(resp.text().await.unwrap_or_default())
        )));
    }
    Err(ProviderError::Other(format!(
        "llama {} {}",
        status,
        body_summary(resp.text().await.unwrap_or_default())
    )))
}

fn map_bedrock_error(err: SdkError<CountTokensError>) -> ProviderError {
    match err {
        SdkError::TimeoutError(_) => ProviderError::Timeout,
        SdkError::ServiceError(service) => {
            let detail = format!("{:?}", service.err());
            match service.err() {
                CountTokensError::AccessDeniedException(_)
                | CountTokensError::ResourceNotFoundException(_) => {
                    ProviderError::Auth(format!("bedrock llama {detail}"))
                }
                CountTokensError::ThrottlingException(_) => ProviderError::RateLimit {
                    retry_after: Duration::from_secs(30),
                },
                CountTokensError::ValidationException(_) => {
                    ProviderError::Schema(format!("bedrock llama {detail}"))
                }
                CountTokensError::InternalServerException(_)
                | CountTokensError::ServiceUnavailableException(_) => {
                    ProviderError::Other(format!("bedrock llama {detail}"))
                }
                _ => ProviderError::Other(format!("bedrock llama {detail}")),
            }
        }
        other => ProviderError::Other(format!("bedrock llama {other}")),
    }
}

fn map_send_error(e: reqwest::Error) -> ProviderError {
    if e.is_timeout() {
        ProviderError::Timeout
    } else {
        let safe = e.without_url();
        ProviderError::Other(format!("llama send: {safe}"))
    }
}

fn body_summary(body: String) -> String {
    format!("<provider body redacted: {}B>", body.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const MODEL: &str = "meta.llama3-1-8b-instruct-v1:0";

    async fn client_for_server(server: &MockServer) -> LlamaClient {
        LlamaClient::with_base_url("test-key", server.uri()).expect("client builds")
    }

    #[tokio::test]
    async fn count_tokens_success_returns_input_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/model/meta.llama3-1-8b-instruct-v1:0/count-tokens"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_json(json!({
                "input": {
                    "converse": {
                        "messages": [{
                            "role": "user",
                            "content": [{ "text": "hello llama" }]
                        }]
                    }
                }
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-amzn-requestid", "bedrock_req_123")
                    .set_body_json(json!({ "inputTokens": 11 })),
            )
            .mount(&server)
            .await;

        let c = client_for_server(&server).await;
        let resp = c.count_tokens(MODEL, "hello llama").await.expect("ok");
        assert_eq!(resp.input_tokens, 11);
        assert_eq!(resp.request_id.as_deref(), Some("bedrock_req_123"));
    }

    #[tokio::test]
    async fn schema_drift_missing_input_tokens_is_schema_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/model/meta.llama3-1-8b-instruct-v1:0/count-tokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "tokens": 11 })))
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let err = c.count_tokens(MODEL, "hello").await.expect_err("schema");
        assert!(matches!(err, ProviderError::Schema(_)));
        assert!(!err.counts_as_breaker_failure());
    }

    #[tokio::test]
    async fn access_denied_maps_to_auth_variant() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/model/meta.llama3-1-8b-instruct-v1:0/count-tokens"))
            .respond_with(ResponseTemplate::new(403).set_body_string("not entitled"))
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let err = c.count_tokens(MODEL, "hello").await.expect_err("auth");
        assert!(matches!(err, ProviderError::Auth(_)));
        assert!(!err.counts_as_breaker_failure());
    }

    #[tokio::test]
    async fn throttling_maps_to_rate_limit() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/model/meta.llama3-1-8b-instruct-v1:0/count-tokens"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "9"))
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let err = c.count_tokens(MODEL, "hello").await.expect_err("rate");
        match err {
            ProviderError::RateLimit { retry_after } => {
                assert_eq!(retry_after, Duration::from_secs(9));
            }
            other => panic!("expected rate limit, got {other:?}"),
        }
    }
}
