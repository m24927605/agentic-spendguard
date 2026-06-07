//! # spendguard-cursor-codec — Cursor IDE MITM codec (EXPERIMENTAL, SOW only)
//!
//! Per [`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md) §1:
//!
//! > **DO NOT SHIP AS A GA FEATURE.** Gated to Enterprise SOW customers who
//! > signed the maintenance addendum. Codec breaks whenever Cursor changes
//! > the wire protocol. Customer accepts break-window risk.
//!
//! This crate reverse-engineers the Cursor IDE Agent's outbound wire
//! format toward `api.cursor.sh` so SpendGuard can interpose budget
//! reservation / commit / release on a Cursor session. The framing
//! layer is public Connect-RPC; the envelope is reconstructed from
//! black-box capture (no vendor source).
//!
//! ## Crate scope (D17 SLICE 1-4)
//!
//! * [`framing`] — Connect-RPC length-prefixed frame reader.
//! * [`cursor_proto`] — prost-generated types for the observed envelope.
//! * [`envelope`] — typed decode helpers + `DecodeError`.
//!
//! Translator (SLICE 5), reserve/commit pipeline (SLICE 6), response
//! re-encode (SLICE 7), and the egress-proxy hook (SLICE 4 in
//! [`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md)
//! §7) all land in later slices.
//!
//! ## Feature gates
//!
//! The crate exposes a `mitm` feature flag. With the workspace-level
//! `cursor-mitm-experimental` feature on (set in `services/egress_proxy`
//! and `services/cli`), the consumer enables `mitm` here. Without it,
//! the public API still compiles (the framing parser is pure-Rust and
//! has no transitive deps that need gating), but downstream wiring is
//! `cfg`-gated out. SLICE 4 introduces the egress-proxy hook under that
//! cfg.
//!
//! ## Legal posture
//!
//! See [`README.md`](../README.md) for the full customer-facing
//! disclaimer. In brief: this is reverse-engineered interop, not
//! vendor-endorsed. Customers who enable the codec do so under an
//! Enterprise SOW that acknowledges (a) the codec breaks on vendor
//! release, (b) the customer is responsible for confirming their own
//! Cursor terms permit on-host MITM.
//!
//! ## Loud experimental banner
//!
//! Per [`review-standards.md`](../../docs/specs/coverage/D17_cursor_mitm/review-standards.md)
//! §1 (`E2`): every public entry point in this crate calls
//! [`assert_experimental_banner_emitted`] on first use per process. SLICE
//! 1-4 ship the function; SLICE 5+ wire the call sites at the pipeline
//! entry point in `egress_proxy`.

#![deny(missing_docs)]

use std::sync::OnceLock;

pub mod cursor_proto;
pub mod envelope;
pub mod framing;

pub use envelope::{decode_chat_request, decode_chat_response_chunk, DecodeError};
pub use framing::{
    ConnectRpcReader, Frame, DEFAULT_MAX_FRAME_LEN, FLAG_COMPRESSED, FLAG_END_OF_STREAM,
};

/// Emit the experimental banner to stderr on first call per process.
///
/// Safe to call from every public entry point — [`OnceLock`] guarantees
/// the message lands exactly once even under concurrent callers. The
/// banner is the second of the three "loud experimental" markers per
/// [`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md)
/// §6 (the first is the Cargo `[package.metadata.experimental]` marker;
/// the third is the SOW addendum doc).
pub fn assert_experimental_banner_emitted() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        eprintln!(
            "[EXPERIMENTAL] cursor-mitm codec active. \
             Codec break SLA: docs/customer/sow-cursor-mitm.md. \
             DO NOT SHIP IN GA CONFIG."
        );
    });
}

#[cfg(test)]
mod lib_tests {
    use super::*;

    #[test]
    fn banner_does_not_panic_on_repeated_calls() {
        // OnceLock guarantees the print runs once, but the function
        // itself must be re-callable from anywhere without panicking.
        assert_experimental_banner_emitted();
        assert_experimental_banner_emitted();
        assert_experimental_banner_emitted();
    }
}
