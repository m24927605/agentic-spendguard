//! SpendGuard output_predictor service crate.
//!
//! Spec ref [`output-predictor-service-spec-v1alpha1.md`] §2.
//!
//! ## Layout
//!
//!   * [`crate::server`] — `OutputPredictor` gRPC impl orchestrating
//!     parallel Strategy A + B + selector per spec §2.3.
//!   * [`crate::strategy_a`] — pure max_tokens-based ceiling (spec §3).
//!   * [`crate::strategy_b`] — cache lookup + cold-start chain (spec §4 + §7).
//!   * [`crate::classifier`] — 7-class rule-based classifier (spec §8.1).
//!     Server-side fallback when caller did not pre-classify; primary
//!     path is sidecar-runs-classifier (spec §8.3).
//!   * [`crate::fingerprint`] — prompt_class_fingerprint hash (spec §8.2).
//!   * [`crate::selector`] — policy-driven (A | B | C) selector (spec §6).
//!   * [`crate::cache`] — in-memory cache layer on top of SQL lookup (spec §4.3).
//!   * [`crate::context_window`] — model_context_window.toml loader (spec §3.2).
//!   * [`crate::config`] — env-driven configuration (mirrors tokenizer pattern).
//!
//! SLICE_06 ships:
//!
//!   * `Predict` RPC: A always; B when cache hit + sample_size_30d >= 30;
//!     C field declared but unset.
//!   * Cold-start: L1 (hard fallback) + L4 (cache hit) only. L2 returns
//!     None until SLICE_08 lands the TOML. L3 deferred per spec §2.2.
//!   * Server boot: UDS + mTLS gates per SLICE_03 R3 N1+N2 pattern.
//!
//! Hot path invariant: this crate is NOT linked from sidecar/egress_proxy
//! in SLICE_06. SLICE_10 wires the client. Until then the predictor runs
//! standalone in the demo / Helm topology.
//!
//! [`output-predictor-service-spec-v1alpha1.md`]: ../../docs/output-predictor-service-spec-v1alpha1.md

pub mod cache;
pub mod circuit_breaker;
pub mod classifier;
pub mod config;
pub mod context_window;
pub mod endpoint_cache;
pub mod fingerprint;
pub mod plugin_client;
pub mod selector;
pub mod server;
pub mod strategy_a;
pub mod strategy_b;
pub mod strategy_c;

/// Generated protobuf types — `tonic::include_proto!` requires this
/// module path so server / client codegen lands inside the crate's
/// public namespace.
pub mod proto {
    pub mod output_predictor {
        pub mod v1 {
            tonic::include_proto!("spendguard.output_predictor.v1");
        }
    }
    /// SLICE_07: customer-trained Strategy C plugin proto. Client stub
    /// is used by `crate::plugin_client` + `crate::strategy_c` to dial
    /// the customer-hosted Predict + HealthCheck endpoints per
    /// `output-predictor-plugin-contract-v1alpha1.md` §2.1.
    pub mod output_predictor_plugin {
        pub mod v1 {
            tonic::include_proto!("spendguard.output_predictor_plugin.v1");
        }
    }
}
