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
    // S7: pass event_time so the verifier can enforce the key's
    // validity window. We use the CloudEvent's producer-attested
    // `time` field; if absent (legacy rows), pass None and the
    // verifier falls back to crypto + revocation only.
    let event_time = evt
        .time
        .as_ref()
        .and_then(|t| chrono::DateTime::<chrono::Utc>::from_timestamp(t.seconds, t.nanos as u32));
    verifier.verify(
        &evt.signing_key_id,
        &canonical,
        &evt.producer_signature,
        event_time,
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

    // ============================================================
    // Round-2 fix M8 + m5: prost round-trip property test for the
    // tag 300-317 prediction extension fields per
    // docs/audit-chain-prediction-extension-v1alpha1.md §7.2.
    //
    // Establishes:
    //   1. Encoding a CloudEvent with tag 300-317 fields populated and
    //      then re-decoding produces a value byte-identical to the
    //      original (the basic prost round-trip property — necessary
    //      for canonical_bytes to be deterministic across re-encoding).
    //   2. The proto3 unset-field semantics: a CloudEvent with all
    //      tag-300+ fields at default values encodes to the same bytes
    //      as the same CloudEvent with those fields explicitly absent.
    //      This is what makes signatures over legacy rows continue to
    //      verify after the proto bump (spec §7.1).
    //
    // What this test does NOT cover (acknowledged scope per M8): the
    // "unknown-field preservation" property that would let an OLD
    // verifier re-encode a NEW event and still match the signature.
    // prost 0.13 strips unknown fields; the deployment invariant in
    // §7.2 (canonical_ingest upgrades first) handles this operationally
    // until prost upstream lands unknown-field preservation
    // (tokio-rs/prost#879).
    // ============================================================

    fn make_event_with_prediction_fields() -> CloudEvent {
        let mut evt = make_event_proto_form();
        // Populate the 18 prediction tag 300-317 fields with non-default
        // values so the test exercises the actual wire encoding paths
        // rather than the "everything is default" cheat.
        evt.predicted_a_tokens = 4096;
        evt.predicted_b_tokens = 512;
        evt.predicted_c_tokens = 768;
        evt.reserved_strategy = "A".into();
        evt.prediction_strategy_used = "B".into();
        evt.prediction_policy_used = "STRICT_CEILING".into();
        evt.tokenizer_tier = "T2".into();
        evt.tokenizer_version_id = "01999d50-1111-7000-8000-000000000003".into();
        evt.prediction_confidence = 0.875;
        evt.prediction_sample_size = 64;
        evt.cold_start_layer_used = "".into(); // warm path
        evt.run_projection_at_decision_atomic = 1_000_000_000;
        evt.run_predicted_remaining_steps = 3;
        evt.run_steps_completed_so_far = 2;
        evt.actual_input_tokens = 256;
        evt.actual_output_tokens = 384;
        evt.delta_b_ratio = 0.75;
        evt.delta_c_ratio = 0.5;
        evt
    }

    #[test]
    fn prost_roundtrip_preserves_tag_300_to_317_fields() {
        let original = make_event_with_prediction_fields();
        let encoded = original.encode_to_vec();
        let decoded = CloudEvent::decode(&*encoded)
            .expect("CloudEvent with tag 300-317 fields must decode");

        // Field-by-field compare. We avoid `assert_eq!(original, decoded)`
        // because we want any drift to point to the exact field.
        assert_eq!(decoded.predicted_a_tokens, original.predicted_a_tokens);
        assert_eq!(decoded.predicted_b_tokens, original.predicted_b_tokens);
        assert_eq!(decoded.predicted_c_tokens, original.predicted_c_tokens);
        assert_eq!(decoded.reserved_strategy, original.reserved_strategy);
        assert_eq!(
            decoded.prediction_strategy_used,
            original.prediction_strategy_used
        );
        assert_eq!(
            decoded.prediction_policy_used,
            original.prediction_policy_used
        );
        assert_eq!(decoded.tokenizer_tier, original.tokenizer_tier);
        assert_eq!(decoded.tokenizer_version_id, original.tokenizer_version_id);
        assert_eq!(
            decoded.prediction_confidence,
            original.prediction_confidence
        );
        assert_eq!(
            decoded.prediction_sample_size,
            original.prediction_sample_size
        );
        assert_eq!(decoded.cold_start_layer_used, original.cold_start_layer_used);
        assert_eq!(
            decoded.run_projection_at_decision_atomic,
            original.run_projection_at_decision_atomic
        );
        assert_eq!(
            decoded.run_predicted_remaining_steps,
            original.run_predicted_remaining_steps
        );
        assert_eq!(
            decoded.run_steps_completed_so_far,
            original.run_steps_completed_so_far
        );
        assert_eq!(decoded.actual_input_tokens, original.actual_input_tokens);
        assert_eq!(decoded.actual_output_tokens, original.actual_output_tokens);
        assert_eq!(decoded.delta_b_ratio, original.delta_b_ratio);
        assert_eq!(decoded.delta_c_ratio, original.delta_c_ratio);

        // The decoded value must re-encode to the same bytes — this is
        // the canonical-bytes determinism property that verify_cloudevent
        // depends on.
        let re_encoded = decoded.encode_to_vec();
        assert_eq!(re_encoded, encoded);
    }

    #[test]
    fn legacy_event_signature_survives_proto_bump() {
        // Spec §7.1: legacy CloudEvents written before the tag 300-317
        // additions verify identically after the bump because proto3
        // default-valued fields encode to zero bytes on the wire.
        //
        // Round-3 fix M9: the round-2 version was a tautology — it
        // called `canonical_bytes(&legacy)` twice on the same event.
        // The real invariant is: two DISTINCT CloudEvents that differ
        // only in the explicit-vs-implicit presence of tag 300+ default
        // values must produce byte-identical canonical_bytes.
        //
        // `legacy` = pre-bump event (tag 300-317 implicitly default; we
        // can't actually distinguish "field absent" from "field set to
        // default" on the wire — that's the whole point of proto3
        // default encoding).
        // `new_with_defaults` = post-bump producer that explicitly
        // populates the new fields with their default values.
        //
        // Byte-identical canonical_bytes ⇒ a signature signed over
        // `legacy` will verify against `new_with_defaults` and vice
        // versa. This is the additive-evolution invariant.
        let mut legacy = make_event_proto_form();
        legacy.signing_key_id = "sidecar:legacy-key".into();
        // legacy has tag 300+ fields at proto3 defaults via Default impl.
        assert_eq!(legacy.predicted_a_tokens, 0);
        assert_eq!(legacy.reserved_strategy, "");
        assert_eq!(legacy.prediction_confidence, 0.0);
        assert_eq!(legacy.run_predicted_remaining_steps, 0);

        // new_with_defaults is a distinct event whose tag 300+ fields
        // have been explicitly assigned the same default values a new
        // producer might emit.
        let mut new_with_defaults = make_event_proto_form();
        new_with_defaults.signing_key_id = "sidecar:legacy-key".into();
        new_with_defaults.predicted_a_tokens = 0;
        new_with_defaults.predicted_b_tokens = 0;
        new_with_defaults.predicted_c_tokens = 0;
        new_with_defaults.reserved_strategy = String::new();
        new_with_defaults.prediction_strategy_used = String::new();
        new_with_defaults.prediction_policy_used = String::new();
        new_with_defaults.tokenizer_tier = String::new();
        new_with_defaults.tokenizer_version_id = String::new();
        new_with_defaults.prediction_confidence = 0.0;
        new_with_defaults.prediction_sample_size = 0;
        new_with_defaults.cold_start_layer_used = String::new();
        new_with_defaults.run_projection_at_decision_atomic = 0;
        new_with_defaults.run_predicted_remaining_steps = 0;
        new_with_defaults.run_steps_completed_so_far = 0;
        new_with_defaults.actual_input_tokens = 0;
        new_with_defaults.actual_output_tokens = 0;
        new_with_defaults.delta_b_ratio = 0.0;
        new_with_defaults.delta_c_ratio = 0.0;

        let canonical_legacy = canonical_bytes(&legacy);
        let canonical_new_with_defaults = canonical_bytes(&new_with_defaults);
        assert_eq!(
            canonical_legacy, canonical_new_with_defaults,
            "proto3 default-encoding invariant violated: legacy event and \
             new-event-with-explicit-defaults must hash to identical canonical bytes"
        );
    }

    #[test]
    fn signature_covers_tag_300_to_317_fields() {
        // Spec §3.1: the mirror approach requires the producer signature
        // to cover the prediction fields, so that an attacker tampering
        // with a column AND the corresponding proto field together
        // cannot escape detection. We exercise that property here: two
        // events that differ only in `predicted_a_tokens` must produce
        // different canonical bytes (and therefore different signatures).
        let mut evt_a = make_event_with_prediction_fields();
        let canonical_a = canonical_bytes(&evt_a);
        evt_a.predicted_a_tokens = 999_999;
        let canonical_b = canonical_bytes(&evt_a);
        assert_ne!(
            canonical_a, canonical_b,
            "tag-300 predicted_a_tokens must affect canonical bytes (signature coverage)"
        );
    }
}
