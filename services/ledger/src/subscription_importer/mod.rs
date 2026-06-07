//! D13 COV_65 — Subscription usage importer stubs.
//!
//! These crates ship empty contracts for Day-2 reconciliation: when
//! Anthropic ships the Console Usage Admin API, when OpenAI's Admin
//! Usage API lands, and when Devin / Manus / Genspark each publish
//! their own CSV export, this module's `ImportRecord` shape is the
//! drop-in contract every implementation must satisfy.
//!
//! D13 ships the **stub** only — see D14 (Devin), D15 (Manus), D16
//! (Genspark) for live importers; Anthropic + OpenAI live importers
//! are blocked on their respective Admin APIs (legal review on
//! Anthropic Console scraping; OpenAI's Admin API still gated to
//! select customers as of 2026-06).
//!
//! Spec: docs/specs/coverage/D13_subscription_meter/design.md §5

pub mod stub;

pub use stub::{AuditRowDraft, ImportRecord, ImporterKind, MIRROR_AUDIT_COLUMN_RESERVATION_SOURCE};
