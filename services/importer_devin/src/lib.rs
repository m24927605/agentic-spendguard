//! # spendguard-importer-devin — Devin Billing Importer (D14)
//!
//! Per [`design.md`](../../../docs/specs/coverage/D14_devin_importer/design.md) §1:
//!
//! > Devin (Cognition Labs) runs the agent loop entirely inside Cognition's
//! > cloud VM. Customer network never carries the per-LLM-call payload;
//! > egress proxy + SDK adapters intercept nothing. The only telemetry
//! > exposed is post-hoc — Devin Team API usage in **ACU (Agent Compute
//! > Unit, ≈ $2.25/ACU)**.
//!
//! This crate is **reconciliation only** — SpendGuard cannot gate Devin
//! sessions. The importer pulls post-hoc usage, converts ACU → USD via
//! a vendored price table, and emits
//! `spendguard.audit.import.devin_acu` CloudEvents tagged with
//! `reservation_source = subscription_meter` /
//! `import_source = devin_team_api`.
//!
//! ## Slice scope (D14 COV_67-72)
//!
//! * [`acu_price_table`] — embedded `devin_acu_prices.json` loader +
//!   `acu_to_micro_usd` conversion (COV_68).
//! * [`import_record`] — `ImportRecord` shape + pure
//!   `import_record_to_audit_row` (COV_69).
//! * [`fixture_loader`] — fixture-mode reader; default merge gate
//!   (COV_69).
//! * [`cloudevent_envelope`] — CloudEvent 1.0 envelope builder for the
//!   `spendguard.audit.import.devin_acu` schema (COV_70).
//! * [`live`] — feature-gated `DevinClient` (COV_71); `cargo tree
//!   -e=normal` MUST NOT show `reqwest` / `hyper-tls` under default
//!   features (acceptance A2.4 + review-standards T2-T4).
//!
//! ## Locked invariants
//!
//! * `data.reservation_source == "subscription_meter"` — never `byok`.
//!   D13 §4.3 fork: importer rows MUST NOT charge the BYOK ledger.
//! * `data.import_source == "devin_team_api"` — matches mig 0059
//!   CHECK widening.
//! * Default `cargo tree -e=normal` is HTTP-client-free. Reviewer-grep
//!   confirms `live`-only deps carry `optional = true`.
//! * `import_record_to_audit_row` is **pure**: no I/O, no clock read,
//!   no `unsafe`. Same `(ImportRecord, AcuPriceTable)` ↦ same
//!   `AuditRowDraft`. Trivially fuzzable / property-testable.
//! * Idempotency key `(devin_team_id, devin_session_id, window_end)`
//!   produces a deterministic UUIDv5 `event.id` so re-running the same
//!   window does not double-emit (canonical_ingest dedups via
//!   `event_replay_dedup`).

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod acu_price_table;
pub mod cloudevent_envelope;
pub mod fixture_loader;
pub mod import_record;

#[cfg(feature = "live")]
pub mod live;

pub use acu_price_table::{
    acu_to_micro_usd, AcuPriceTable, AcuRate, ConversionError, PriceLookupError,
};
pub use cloudevent_envelope::{
    CloudEventData, CloudEventEnvelope, EnvelopeBuildError, EVENT_SOURCE, EVENT_TYPE,
    SCHEMA_VERSION,
};
pub use fixture_loader::{FixtureLoadError, FixtureLoader};
pub use import_record::{
    import_record_to_audit_row, AuditRowDraft, ImportRecord, IngestionMode,
    IMPORT_SOURCE_DEVIN_TEAM_API, REASON_CODE_ENTERPRISE_NEGOTIATED,
    RESERVATION_SOURCE_SUBSCRIPTION_METER,
};
