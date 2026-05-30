//! LiteLLM proxy competitor adapter — calls the proxy's /chat/completions
//! endpoint and post-call accounts the budget.
//!
//! Why we benchmark against LiteLLM:
//!   * Open-source, popular reference implementation
//!   * Existing benchmarks/runaway-loop already wires it; same wire shape
//!   * Demonstrates the structural difference: LiteLLM enforces POST-call
//!     (one call past the budget can sneak through), SpendGuard enforces
//!     PRE-call (the call is denied before it leaves the runner)
//!
//! Per slice §9 review item #2 (competitor version):
//!   Pinned by Docker image tag `ghcr.io/berriai/litellm:main-stable`
//!   (mirrors the version used in examples/litellm-proxy-composite/).

use super::{Competitor, DecisionResult};
use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

/// max_tokens cap matches the SpendGuard reservation amount so the
/// two competitors are on identical structural footing (each gets one
/// chance to gate the same N tokens-of-budget per call).
const MAX_TOKENS: u64 = 500;

pub fn new(base_url: String) -> Box<dyn Competitor> {
    Box::new(LiteLlmClient {
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("reqwest client init"),
        base_url,
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

struct LiteLlmClient {
    client: reqwest::Client,
    base_url: String,
}

impl Competitor for LiteLlmClient {
    fn one_decision<'a>(&'a self, idx: usize) -> BoxFuture<'a, Result<DecisionResult>> {
        Box::pin(async move {
            // The proxy's /chat/completions endpoint is the canonical
            // entrypoint. Per benchmarks/runaway-loop semantics, the
            // benchmark sends through the LiteLLM router with a deterministic
            // mock backend (configured via proxy_config.yaml in the demo
            // example). If the proxy denies the call we still record the
            // wall latency (the deny path is part of the SLO surface).
            let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
            let req = ChatRequest {
                model: "gpt-4o-mini",
                messages: vec![ChatMessage {
                    role: "user",
                    content: "bench",
                }],
                max_tokens: MAX_TOKENS,
            };
            let r = self.client.post(&url).json(&req).send().await?;

            // LiteLLM proxy's hard-block path returns 4xx with a specific
            // error code; we treat all 4xx as "denied" (reserved=0, actual=0).
            if r.status().is_client_error() {
                return Ok(DecisionResult {
                    reserved_atomic: 0,
                    actual_atomic: 0,
                });
            }
            if !r.status().is_success() {
                return Err(anyhow!("litellm: HTTP {}", r.status()));
            }
            let body: ChatResponse = r.json().await?;

            // LiteLLM doesn't pre-reserve — its enforcement is post-call.
            // From the budget perspective the "reservation" at decision
            // time is the max_tokens cap; the actual is what the response
            // reports. That's the apples-to-apples shape we compare to
            // SpendGuard's pre-reservation.
            //
            // The KEY DIFFERENCE: LiteLLM only learns the call was over
            // budget AFTER total_tokens is in hand. Calls in flight at
            // budget-exceed time complete; SpendGuard would have refused
            // them. That's the +8% overshoot agentbudget demonstrates in
            // benchmarks/runaway-loop/RESULTS.md.
            //
            // Side note (per slice §9 #2): LiteLLM proxy upstream version
            // is captured at compose-time, not at bench-time. RESULTS.md
            // pins the exact image SHA via `docker image inspect`.
            let _ = idx;
            Ok(DecisionResult {
                reserved_atomic: MAX_TOKENS,
                actual_atomic: body.usage.total_tokens,
            })
        })
    }
}
