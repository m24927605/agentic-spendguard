//! D13 — Subscription-Tier Meter Mode.
//!
//! Detects subscription-plan traffic (Claude Code Pro / Codex on
//! ChatGPT Plus / Pro) and routes it through a meter-only path that
//! does NOT call the ledger. Each request increments a per-tenant
//! `consumed_atomic` counter and evaluates soft-cap / hard-cap
//! thresholds, optionally short-circuiting CONTINUE → DENY with a
//! synthetic 429.
//!
//! Spec:  docs/specs/coverage/D13_subscription_meter/design.md
//! Slices: COV_60 (classifier), COV_61 (proto + branch), COV_62 (Codex
//!         routing + meter_only_estimate), COV_63 (alerts), COV_64
//!         (hard-cap 429), COV_65 (importer stubs), COV_66 (demo).

pub mod alerts;
pub mod classifier;
pub mod estimator;
pub mod hard_cap;
pub mod route;

pub use alerts::{should_emit_alert, AlertDecision, AlertSeverity};
pub use classifier::{classify, ClassifierInput, SubscriptionKind};
pub use estimator::{meter_only_estimate, MeterEstimate};
pub use hard_cap::{
    evaluate_cap, synthetic_429_body, CapDecision, CapEvaluation, HARD_CAP_RETRY_AFTER_MAX_SECONDS,
};
pub use route::route_decision_request;
