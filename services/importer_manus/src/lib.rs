//! # spendguard-importer-manus — Manus Billing Importer (D15)
//!
//! Per [`design.md`](../../../docs/specs/coverage/D15_manus_importer/design.md) §1:
//!
//! > Manus (Butterfly Effect, Meta-acquired 2026) runs each agent task
//! > inside a vendor-managed VM. Clients see a task ID and a **credit**
//! > counter — never per-LLM-call payloads, never a model split. No
//! > proxy hook, no callback bus, no base-URL swap. Archetype IV is
//! > architecturally unreachable for predictive gating; the only legible
//! > signal is the post-hoc Team+ admin REST surface gated by an API
//! > token.
//!
//! This crate is **reconciliation only** — SpendGuard cannot gate Manus
//! sessions. The importer pulls post-hoc credit usage, converts
//! credit -> USD via a vendored TOML price table, and emits
//! `spendguard.audit.import.manus_credit` CloudEvents tagged with
//! `reservation_source = import_manus` /
//! `import_source = manus_admin_usage`.
//!
//! ## Slice scope (D15 COV_70-74)
//!
//! * [`pricing`] — embedded `assets/price_table.toml` loader +
//!   `credit_to_usd_micros` integer-saturating conversion (COV_71).
//! * [`record`] — `UsageRecord` / `ImportRecord` wire+internal shapes,
//!   `Tier` + `SessionStatus` enums (COV_70).
//! * [`audit`] — `import_record_to_audit_row` pure transform +
//!   `RESERVATION_SOURCE` / `IMPORT_SOURCE` / `CLOUDEVENT_TYPE`
//!   constants (COV_72).
//! * [`fixture`] — fixture-mode loader; default merge gate (COV_72).
//! * [`cloudevent_envelope`] — CloudEvent 1.0 envelope builder for the
//!   `spendguard.audit.import.manus_credit` schema (COV_74).
//! * [`error`] — `ImporterError` / `MeterError` typed errors (COV_70).
//! * [`live`] — feature-gated `ManusClient` (COV_73); `cargo tree
//!   -e=normal` MUST NOT show `reqwest` / `hyper-tls` under default
//!   features (review-standards T3 / T4).
//!
//! ## Locked invariants (design §5)
//!
//! * `reservation_source == "subscription_meter"` — D14/D16 family-aligned
//!   (deviation from D15 design §5 #1; see `audit::RESERVATION_SOURCE`
//!   doc-comment).
//! * `import_source == "manus_team_api"` — D14/D16 `<vendor>_team_api`
//!   family pattern; matches mig 0060 CHECK widening.
//! * Default `cargo tree -e=normal` is HTTP-client-free. Reviewer-grep
//!   confirms `live`-only deps carry `optional = true`.
//! * `import_record_to_audit_row` is **pure**: no I/O, no clock read,
//!   no `unsafe`. Same `(ImportRecord, PriceTable)` ↦ same
//!   `AuditRowDraft`. Trivially fuzzable / property-testable.
//! * Idempotency key `(workspace_id, session_id, window_end)` produces
//!   a deterministic UUIDv5 `event.id` and `dedupe_key = "manus:{sid}"`
//!   so re-running the same window does not double-emit
//!   (canonical_ingest dedups via `event_replay_dedup`).
//! * Synthetic `model = "manus.session/credit"`, `input_tokens =
//!   output_tokens = 0`. Honest zero beats guessed tokens (design §5
//!   #10).
//! * Integer micro-USD only in the conversion hot path; `saturating_mul`
//!   guards i64 overflow (design §5 #12, review-standards T13).

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod audit;
pub mod cloudevent_envelope;
pub mod error;
pub mod fixture;
pub mod pricing;
pub mod record;

#[cfg(feature = "live")]
pub mod live;

pub use audit::{
    import_record_to_audit_row, AuditRowDraft, CLOUDEVENT_TYPE, IMPORT_SOURCE, RESERVATION_SOURCE,
};
pub use cloudevent_envelope::{
    CloudEventData, CloudEventEnvelope, EnvelopeBuildError, EVENT_SOURCE, EVENT_TYPE,
    SCHEMA_VERSION,
};
pub use error::{ImporterError, MeterError};
pub use fixture::{FixtureLoadError, FixtureLoader};
pub use pricing::{credit_to_usd_micros, PriceTable, TierPricing};
pub use record::{ImportRecord, IngestionMode, SessionStatus, Tier, UsageRecord};
