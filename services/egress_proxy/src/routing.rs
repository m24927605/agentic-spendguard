//! Routing surface for the egress proxy.
//!
//! COV_01 (`docs/internal/slices/COV_01_envoy_extproc_skeleton.md`) extracted
//! [`ProviderKind`], [`RequestShape`], [`UsageMetrics`], [`ProviderConfig`],
//! [`ROUTING_TABLE`], `route()`, [`resolve_model_id`], and
//! [`resolve_tokenizer_kind`] into the shared `spendguard-provider-routing`
//! crate so `services/envoy_extproc` (D01 Envoy AI Gateway adapter) can
//! consume the same table.
//!
//! This module is now a thin shim. It:
//!   * re-exports the moved types,
//!   * provides a wrapper [`route`] that **eagerly registers** the
//!     egress_proxy's per-provider extractors via a `Lazy` so the
//!     pre-COV_01 call sites (forward.rs, decision.rs, integration tests
//!     that mount `#[path = "../src/routing.rs"]`) keep working with no
//!     ordering assumption,
//!   * exposes [`install_extractors`] for explicit registration from
//!     `main` — preferred because it surfaces double-init as an error.
//!
//! The original (pre-COV_01) test suite lives in the shared crate.

use once_cell::sync::Lazy;

#[allow(unused_imports)]
pub use spendguard_provider_routing::{
    bedrock as bedrock_dispatch, init_extractors, resolve_model_id, resolve_tokenizer_kind,
    ProviderConfig, ProviderKind, RequestShape, RoutingExtractors, UsageMetrics, ROUTING_TABLE,
};

/// Eagerly register the egress_proxy's per-provider extractors on first
/// access. `Lazy` runs at most once per process; if `install_extractors`
/// was already called from `main`, the second register returns Err
/// which we discard (the registered set is byte-identical anyway).
///
/// INVARIANT: the lazy self-install path uses `default_extractors()`
/// which MUST be byte-identical to the `install_extractors()` call from
/// `main`. Diverging the two paths silently miscount: a `main`-installed
/// process would route response usage through one extractor set while a
/// test process bypassing `main` (e.g. integration tests that `#[path]`-
/// mount this file) would route through another, producing
/// non-reproducible token counts across the same code paths. The single
/// source of truth is `default_extractors()` below; any future
/// per-provider extractor added must be appended there AND nowhere else.
static EAGER_INSTALL: Lazy<()> = Lazy::new(|| {
    let _ = spendguard_provider_routing::init_extractors(default_extractors());
});

fn default_extractors() -> RoutingExtractors {
    RoutingExtractors {
        openai: crate::providers::openai::extract_usage,
        anthropic: crate::providers::anthropic::extract_usage,
        bedrock: crate::providers::bedrock::extract_usage,
        vertex: crate::providers::vertex::extract_usage,
        azure_openai: crate::providers::azure_openai::extract_usage,
    }
}

/// Register the egress_proxy's per-provider extractors with the shared
/// crate. `main` calls this so a duplicate registration surfaces as an
/// `Err` — the `EAGER_INSTALL` Lazy fallback only kicks in if the
/// process bypasses `main` (e.g. integration tests that `#[path]`-mount
/// `routing.rs`).
pub fn install_extractors() -> Result<(), &'static str> {
    init_extractors(default_extractors())
}

/// COV_01 wrapper that mirrors the pre-extraction `routing::route`
/// signature. The eager-install Lazy fires on first access; subsequent
/// calls go straight to the shared crate's [`spendguard_provider_routing::route`].
pub fn route(path: &str) -> Option<&'static ProviderConfig> {
    Lazy::force(&EAGER_INSTALL);
    spendguard_provider_routing::route(path)
}
