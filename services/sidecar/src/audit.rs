//! Phase 5 GA hardening S6: helper to populate `producer_signature`
//! and `signing_key_id` on outgoing CloudEvent envelopes before the
//! sidecar emits them via Ledger / Canonical Ingest RPCs.
//!
//! Canonical-bytes contract: we sign over the protobuf encoding of the
//! CloudEvent with `producer_signature` cleared and `signing_key_id`
//! populated. Verifiers (S8) strip the signature, re-encode, and check.
//! `time` is included intentionally — the producer attests to it. The
//! hot-path encoder is `prost::Message::encode_to_vec`; it's
//! deterministic for a single producer (proto3 wire format).

use prost::Message;
use spendguard_signing::{SignError, Signer};
use tracing::warn;

use crate::{
    domain::error::DomainError,
    proto::common::v1::CloudEvent,
};

/// Sign `event` in place. Sets `signing_key_id` first (so it's covered
/// by the signature), then computes the canonical encoding with an
/// empty `producer_signature`, signs, and writes the resulting bytes
/// back into `producer_signature`.
///
/// On `SignError::ModeUnavailable` (e.g. KMS stub before S7 lands) we
/// fail the whole operation rather than silently writing an empty
/// signature — operators choose `kms` mode explicitly and need to be
/// blocked at decision time, not at audit-verification time.
pub async fn sign_cloudevent_in_place(
    signer: &dyn Signer,
    event: &mut CloudEvent,
) -> Result<(), DomainError> {
    event.signing_key_id = signer.key_id().to_string();
    event.producer_signature = Vec::new().into();

    let canonical = event.encode_to_vec();

    match signer.sign(&canonical).await {
        Ok(sig) => {
            event.producer_signature = sig.bytes.into();
            Ok(())
        }
        Err(SignError::ModeUnavailable(msg)) => {
            warn!(error = %msg, "signer reports mode unavailable; failing audit emission");
            Err(DomainError::Internal(anyhow::anyhow!(
                "signing mode unavailable: {msg}"
            )))
        }
        Err(other) => {
            warn!(error = ?other, "signer error");
            Err(DomainError::Internal(anyhow::anyhow!(
                "signing failed: {other}"
            )))
        }
    }
}
