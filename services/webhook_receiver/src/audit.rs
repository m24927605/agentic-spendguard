//! Phase 5 GA hardening S6: helper to populate `producer_signature`
//! and `signing_key_id` on outgoing CloudEvents before the webhook
//! receiver emits them via Ledger RPCs. Mirror of
//! services/sidecar/src/audit.rs (same canonical-bytes contract).

use prost::Message;
use spendguard_signing::{SignError, Signer};
use tracing::warn;

use crate::{domain::error::ReceiverError, proto::common::v1::CloudEvent};

pub async fn sign_cloudevent_in_place(
    signer: &dyn Signer,
    event: &mut CloudEvent,
) -> Result<(), ReceiverError> {
    event.signing_key_id = signer.key_id().to_string();
    event.producer_signature = Vec::new().into();
    let canonical = event.encode_to_vec();

    match signer.sign(&canonical).await {
        Ok(sig) => {
            event.producer_signature = sig.bytes.into();
            Ok(())
        }
        Err(SignError::ModeUnavailable(msg)) => {
            warn!(error = %msg, "signer reports mode unavailable; failing webhook emission");
            Err(ReceiverError::Internal(anyhow::anyhow!(
                "signing mode unavailable: {msg}"
            )))
        }
        Err(other) => {
            warn!(error = ?other, "signer error");
            Err(ReceiverError::Internal(anyhow::anyhow!(
                "signing failed: {other}"
            )))
        }
    }
}
