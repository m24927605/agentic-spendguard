//! # spendguard-importer-genspark — Genspark Billing Importer (D16)
//!
//! Per [`design.md`](../../../docs/specs/coverage/D16_genspark_importer/design.md) §1:
//!
//! > Genspark Super Agent runs inside Genspark's cloud VM. Operators
//! > buy a subscription (Plus $19.99/mo, Pro $24.99/mo, Premium
//! > $249.99/mo); every action draws credits and Genspark settles
//! > dollars internally. The client sees a task result + an aggregate
//! > credit number — never a per-LLM-call payload, never a tokenized
//! > prompt.
//!
//! This crate is **reconciliation only** — SpendGuard cannot gate
//! Genspark sessions. The importer pulls post-hoc credit consumption,
//! converts credits → USD via a vendored price table, and emits
//! `spendguard.audit.import.genspark_credit` CloudEvents tagged with
//! `reservation_source = subscription_meter` /
//! `import_source = genspark_team_api`.
//!
//! ## Slice scope (D16 COV_84-88)
//!
//! * [`credit_price_table`] — embedded `genspark_credit_prices.json`
//!   loader + `credits_to_micro_usd` conversion (COV_85).
//! * [`import_record`] — `ImportRecord` shape + pure
//!   `import_record_to_audit_row` (COV_85).
//! * [`fixture_loader`] — fixture-mode reader; default merge gate
//!   (COV_86).
//! * [`cloudevent_envelope`] — CloudEvent 1.0 envelope builder for the
//!   `spendguard.audit.import.genspark_credit` schema (COV_86).
//! * [`live`] — feature-gated `GensparkClient` (COV_87); `cargo tree
//!   -e=normal` MUST NOT show `reqwest` / `hyper-tls` under default
//!   features (acceptance A2.4 + review-standards T3-T4).
//!
//! ## Locked invariants
//!
//! * `data.reservation_source == "subscription_meter"` — never `byok`.
//!   D13 §4.3 fork: importer rows MUST NOT charge the BYOK ledger.
//! * `data.import_source == "genspark_team_api"` — matches mig 0061
//!   CHECK widening (which lands additively on top of D14's mig 0059).
//! * Default `cargo tree -e=normal` is HTTP-client-free. Reviewer-grep
//!   confirms `live`-only deps carry `optional = true`.
//! * `import_record_to_audit_row` is **pure**: no I/O, no clock read,
//!   no `unsafe`. Same `(ImportRecord, CreditPriceTable)` ↦ same
//!   `AuditRowDraft`. Trivially fuzzable / property-testable.
//! * Idempotency key `(workspace_id, task_id, window_end)` produces a
//!   deterministic UUIDv5 `event.id` so re-running the same window
//!   does not double-emit (canonical_ingest dedups via
//!   `event_replay_dedup`).
//! * Unknown plan slug ↦ `amount_micro_usd = 0` +
//!   `reason_code = "genspark_plan_unknown"`. BOTH fields set;
//!   dashboard distinguishes "unknown rate" from "zero spend".

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod cloudevent_envelope;
pub mod credit_price_table;
pub mod fixture_loader;
pub mod import_record;

#[cfg(feature = "live")]
pub mod live;

pub use cloudevent_envelope::{
    CloudEventData, CloudEventEnvelope, EnvelopeBuildError, EVENT_SOURCE, EVENT_TYPE,
    SCHEMA_VERSION,
};
pub use credit_price_table::{
    credits_to_micro_usd, ConversionError, CreditPriceTable, CreditRate, PriceLookupError,
};
pub use fixture_loader::{FixtureLoadError, FixtureLoader};
pub use import_record::{
    import_record_to_audit_row, AuditRowDraft, ImportRecord, IngestionMode,
    IMPORT_SOURCE_GENSPARK_TEAM_API, REASON_CODE_GENSPARK_PLAN_UNKNOWN,
    RESERVATION_SOURCE_SUBSCRIPTION_METER,
};
