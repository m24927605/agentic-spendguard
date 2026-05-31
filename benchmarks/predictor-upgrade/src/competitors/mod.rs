//! Per-competitor target adapters. Each adapter implements `Competitor`
//! and is responsible for translating "make one decision" into the
//! competitor's specific wire shape.
//!
//! Why a trait rather than enum:
//!   * Each competitor has different state (TLS client, UDS socket,
//!     base URL) and we want that state encapsulated.
//!   * The harness just calls one_decision(i) and times it; nothing
//!     else leaks through the boundary. Clean dependency direction.
//!
//! Async dispatch is via `BoxFuture<'_, Result<DecisionResult>>` rather
//! than `#[async_trait]`. Keeps the dep tree light (one less crate) and
//! the indirection is fine for benchmarking — the syscall is the cost
//! dominator, not the vtable.

use anyhow::Result;
use futures::future::BoxFuture;

pub mod litellm;
pub mod portkey;
pub mod spendguard;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompetitorName {
    SpendGuard,
    LiteLlm,
    Portkey,
}

impl CompetitorName {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SpendGuard => "spendguard",
            Self::LiteLlm => "litellm",
            Self::Portkey => "portkey",
        }
    }
}

/// Per-call result from a competitor adapter. The harness uses this to
/// compute overshoot % and, when available, decision-only latency.
#[derive(Debug, Clone)]
pub struct DecisionResult {
    /// Reserved units at decision time (atomic — token-equivalent or USD-atomic).
    /// For SpendGuard this is the predictor's reservation.
    /// For LiteLLM proxy this is the max_tokens cap (post-call enforcement → 0).
    /// For Portkey this is whatever the closed-source proxy actually pre-allocates.
    pub reserved_atomic: u64,
    /// Actual usage observed at commit time (post-call).
    pub actual_atomic: u64,
    /// Optional adapter-measured decision latency in microseconds. This
    /// is used when a target has post-call accounting work that must be
    /// awaited for overshoot math but is outside the Contract §14
    /// decision-latency SLO.
    pub decision_latency_us: Option<u64>,
}

/// All competitors implement this. Each `one_decision` is one full
/// reserve+commit roundtrip (or whatever shape the competitor exposes).
/// Adapters may return `decision_latency_us` when post-call accounting
/// must not be counted as pre-call decision latency.
///
/// The `idx` arg lets adapters mint per-call idempotency keys without
/// drag from a shared counter (== avoid lock contention in the burst).
pub trait Competitor: Send + Sync {
    fn one_decision<'a>(&'a self, idx: usize) -> BoxFuture<'a, Result<DecisionResult>>;
}
