//! Portkey competitor adapter — stub.
//!
//! Per slice §3 ("closed-source competitors not publicly available")
//! and §10 ("comparison to closed-source competitors not publicly
//! available" pushed to Future): we document Portkey as N/A in
//! RESULTS.md and ship this stub so a reviewer with a Portkey
//! deployment can wire it without further patches.
//!
//! What the stub does:
//!   * If --portkey-url is empty (default), every one_decision returns
//!     an error variant. The harness records each as an error and the
//!     report includes a clear "Portkey: not configured (closed source)"
//!     note in RESULTS.md.
//!   * If --portkey-url is set, we attempt the same shape as LiteLLM
//!     (POST /chat/completions). Portkey's gateway shape is very close
//!     to OpenAI's, so the same request format generally works against
//!     a real Portkey gateway with PORTKEY_API_KEY in the environment.
//!
//! This is consistent with the existing benchmarks/runaway-loop
//! pattern: every competitor that publishes an open spec gets a real
//! adapter; closed-source competitors get a structured "documented
//! N/A" path that doesn't lie about apples-to-apples numbers.

use super::{Competitor, DecisionResult};
use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

pub fn new(base_url: String) -> Box<dyn Competitor> {
    Box::new(PortkeyClient {
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("reqwest client init"),
        base_url,
        api_key: std::env::var("PORTKEY_API_KEY").ok(),
    })
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    max_tokens: u64,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    usage: ChatUsage,
}

#[derive(Deserialize)]
struct ChatUsage {
    total_tokens: u64,
}

const MAX_TOKENS: u64 = 500;

struct PortkeyClient {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl Competitor for PortkeyClient {
    fn one_decision<'a>(&'a self, _idx: usize) -> BoxFuture<'a, Result<DecisionResult>> {
        Box::pin(async move {
            if self.base_url.is_empty() {
                // Documented N/A — surfaced in RESULTS.md via the error
                // count on the burst report.
                return Err(anyhow!(
                    "portkey: documented N/A — closed source; pass --portkey-url + PORTKEY_API_KEY to wire"
                ));
            }
            if self.api_key.is_none() {
                return Err(anyhow!(
                    "portkey: PORTKEY_API_KEY not set; required to call the gateway"
                ));
            }

            let url = format!(
                "{}/v1/chat/completions",
                self.base_url.trim_end_matches('/')
            );
            let req = ChatRequest {
                model: "gpt-4o-mini",
                messages: vec![ChatMessage {
                    role: "user",
                    content: "bench",
                }],
                max_tokens: MAX_TOKENS,
            };
            let r = self
                .client
                .post(&url)
                .bearer_auth(self.api_key.as_ref().unwrap())
                .json(&req)
                .send()
                .await?;

            if r.status().is_client_error() {
                return Ok(DecisionResult {
                    reserved_atomic: 0,
                    actual_atomic: 0,
                    decision_latency_us: None,
                });
            }
            if !r.status().is_success() {
                return Err(anyhow!("portkey: HTTP {}", r.status()));
            }
            let body: ChatResponse = r.json().await?;
            Ok(DecisionResult {
                reserved_atomic: MAX_TOKENS,
                actual_atomic: body.usage.total_tokens,
                decision_latency_us: None,
            })
        })
    }
}
