//! SLICE_11 — Per-provider implementations.
//!
//! Each submodule exports an `extract_usage(&serde_json::Value) ->
//! UsageMetrics` function that the routing table's [`ProviderConfig`]
//! wires up. Phase B fills in real extractors; Phase C wires Bedrock
//! per-model dispatch back into `decision.rs` so estimate_call_cost
//! uses the correct tokenizer kind.
//!
//! [`ProviderConfig`]: crate::routing::ProviderConfig

pub mod anthropic;
pub mod azure_openai;
pub mod bedrock;
pub mod openai;
pub mod vertex;
