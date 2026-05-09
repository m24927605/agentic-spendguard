//! Phase 5 GA hardening S8: helpers that bridge the canonical_ingest
//! handler to `spendguard_signing`'s Verifier trait.
//!
//! Two canonical-bytes encodings are supported, mirroring the producer
//! S6 implementation:
//!
//!   * **proto canonical** — used by sidecar / webhook_receiver /
//!     ttl_sweeper. The signed bytes are
//!     `prost::Message::encode_to_vec` of the CloudEvent with
//!     `producer_signature` cleared and `signing_key_id` populated.
//!   * **JSON canonical** — used by the ledger's server-minted
//!     decision row in `InvoiceReconcile`. The signed bytes are
//!     `serde_json::to_vec(&decision_payload)` (the payload object the
//!     ledger writes into `cloudevent_payload`).
//!
//! `producer_id.starts_with("ledger:")` is the tell. Until S7 adds a
//! richer per-event canonical_form metadata, this prefix is the
//! contract.

use prost::Message;
use serde_json::json;
use spendguard_signing::{Verifier, VerifyFailure};

use crate::proto::common::v1::CloudEvent;

/// Verify a CloudEvent's signature using the supplied verifier. Returns
/// Ok(()) on match. Returns Err(VerifyFailure) for typed failure modes.
/// Caller decides how to react (strict-mode reject vs quarantine vs
/// admit-with-metric).
pub fn verify_cloudevent(
    verifier: &dyn Verifier,
    evt: &CloudEvent,
) -> Result<(), VerifyFailure> {
    let canonical = canonical_bytes(evt);
    verifier.verify(
        &evt.signing_key_id,
        &canonical,
        &evt.producer_signature,
    )
}

/// Compute the canonical bytes for a CloudEvent. Producers and
/// verifiers MUST use the same logic — keep these two functions in
/// lock-step (the producer copies live in
/// services/{sidecar,webhook_receiver,ttl_sweeper}/src/audit.rs and
/// services/ledger/src/handlers/invoice_reconcile.rs).
pub fn canonical_bytes(evt: &CloudEvent) -> Vec<u8> {
    if evt.producer_id.starts_with("ledger:") {
        canonical_bytes_json(evt)
    } else {
        canonical_bytes_proto(evt)
    }
}

fn canonical_bytes_proto(evt: &CloudEvent) -> Vec<u8> {
    let mut copy = evt.clone();
    copy.producer_signature = Vec::new().into();
    copy.encode_to_vec()
}

fn canonical_bytes_json(evt: &CloudEvent) -> Vec<u8> {
    // Mirror of services/ledger/src/handlers/invoice_reconcile.rs
    // decision_payload JSON keys. Critical: serde_json::to_vec orders
    // keys deterministically per insertion (we use the same order as
    // the producer).
    use base64::Engine as _;
    let data_b64 = base64::engine::general_purpose::STANDARD.encode(&evt.data);
    let payload = json!({
        "specversion":     evt.specversion,
        "type":            evt.r#type,
        "source":          evt.source,
        "id":              evt.id,
        "time_seconds":    evt.time.as_ref().map(|t| t.seconds).unwrap_or_default(),
        "time_nanos":      evt.time.as_ref().map(|t| t.nanos).unwrap_or_default(),
        "datacontenttype": evt.datacontenttype,
        "data_b64":        data_b64,
        "tenantid":        evt.tenant_id,
        "runid":           evt.run_id,
        "decisionid":      evt.decision_id,
        "schema_bundle_id": evt.schema_bundle_id,
        "producer_id":     evt.producer_id,
        "producer_sequence": evt.producer_sequence,
        "signing_key_id":  evt.signing_key_id,
    });
    serde_json::to_vec(&payload).expect("canonical JSON serialization is infallible")
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost_types::Timestamp;
    use spendguard_signing::{LocalEd25519Signer, LocalEd25519Verifier, Signer};
    use std::collections::HashMap;

    fn make_event_proto_form() -> CloudEvent {
        CloudEvent {
            specversion: "1.0".into(),
            r#type: "spendguard.audit.decision".into(),
            source: "sidecar://demo/wl-1".into(),
            id: "01999d4f-1234-7000-8000-000000000001".into(),
            time: Some(Timestamp { seconds: 1_700_000_000, nanos: 0 }),
            datacontenttype: "application/json".into(),
            data: b"{}".to_vec().into(),
            tenant_id: "00000000-0000-4000-8000-000000000001".into(),
            run_id: String::new(),
            decision_id: "00000000-0000-7000-8000-000000000002".into(),
            schema_bundle_id: String::new(),
            producer_id: "sidecar:wl-1".into(),
            producer_sequence: 42,
            producer_signature: Vec::new().into(),
            signing_key_id: String::new(),
        }
    }

    fn make_event_json_form() -> CloudEvent {
        let mut evt = make_event_proto_form();
        evt.producer_id = "ledger:server-mint".into();
        evt
    }

    #[tokio::test]
    async fn proto_canonical_roundtrips_through_signer_verifier() {
        let mut rng = rand::rngs::OsRng;
        let signing_key = ed25519_dalek::SigningKey::generate(&mut rng);
        let verifying_key = signing_key.verifying_key();
        let signer = LocalEd25519Signer::from_key(signing_key, "sidecar:test".into());

        let mut evt = make_event_proto_form();
        evt.signing_key_id = signer.key_id().to_string();
        let canonical = canonical_bytes(&evt);
        let sig = signer.sign(&canonical).await.unwrap();
        evt.producer_signature = sig.bytes.clone().into();

        let mut keys = HashMap::new();
        keys.insert(signer.key_id().to_string(), verifying_key);
        let verifier = LocalEd25519Verifier::from_keys(keys);

        verify_cloudevent(&verifier, &evt).expect("proto-form must verify");
    }

    #[tokio::test]
    async fn json_canonical_roundtrips_for_ledger_minted_rows() {
        let mut rng = rand::rngs::OsRng;
        let signing_key = ed25519_dalek::SigningKey::generate(&mut rng);
        let verifying_key = signing_key.verifying_key();
        let signer = LocalEd25519Signer::from_key(signing_key, "ledger:server-mint".into());

        let mut evt = make_event_json_form();
        evt.signing_key_id = signer.key_id().to_string();
        let canonical = canonical_bytes(&evt);
        // Verify: producer_id starts with "ledger:" so we hit the JSON
        // branch.
        assert!(evt.producer_id.starts_with("ledger:"));
        let sig = signer.sign(&canonical).await.unwrap();
        evt.producer_signature = sig.bytes.clone().into();

        let mut keys = HashMap::new();
        keys.insert(signer.key_id().to_string(), verifying_key);
        let verifier = LocalEd25519Verifier::from_keys(keys);

        verify_cloudevent(&verifier, &evt).expect("json-form must verify");
    }

    #[tokio::test]
    async fn proto_signature_does_not_verify_against_json_canonical_and_vice_versa() {
        // Cross-form check: a sidecar-signed (proto) event whose
        // producer_id is mutated to "ledger:..." must fail verification
        // because the verifier picks the JSON canonical and re-derives
        // a different digest.
        let mut rng = rand::rngs::OsRng;
        let signing_key = ed25519_dalek::SigningKey::generate(&mut rng);
        let verifying_key = signing_key.verifying_key();
        let signer = LocalEd25519Signer::from_key(signing_key, "sidecar:test".into());

        let mut evt = make_event_proto_form();
        evt.signing_key_id = signer.key_id().to_string();
        let canonical_proto = canonical_bytes(&evt);
        let sig = signer.sign(&canonical_proto).await.unwrap();
        evt.producer_signature = sig.bytes.into();

        // Tamper: pretend it's ledger-minted.
        evt.producer_id = "ledger:server-mint".into();

        let mut keys = HashMap::new();
        keys.insert(signer.key_id().to_string(), verifying_key);
        let verifier = LocalEd25519Verifier::from_keys(keys);

        let err = verify_cloudevent(&verifier, &evt).unwrap_err();
        assert_eq!(err, VerifyFailure::InvalidSignature);
    }

    #[test]
    fn canonical_proto_excludes_signature_bytes() {
        // Self-consistency: the canonical form for proto path MUST be
        // independent of producer_signature. Otherwise verifier could
        // never reproduce the producer's input.
        let mut evt = make_event_proto_form();
        evt.producer_signature = b"originally-empty".to_vec().into();
        let a = canonical_bytes(&evt);
        evt.producer_signature = b"completely-different".to_vec().into();
        let b = canonical_bytes(&evt);
        assert_eq!(a, b);
    }
}
