// build.rs — protobuf codegen for the tokenizer gRPC service.
//
// Spec ref `tokenizer-service-spec-v1alpha1.md` §2.2.
//
// SLICE_03 generates:
//   * spendguard.tokenizer.v1 (TokenizerService) — server side
//
// SLICE_05 extends with:
//   * spendguard.common.v1 (CloudEvent envelope) — used for the signed
//     `tokenizer_drift_alert` event the shadow worker emits per spec §4
//   * spendguard.canonical_ingest.v1 (CanonicalIngest service) —
//     client side; shadow worker calls AppendEvents to land the
//     drift_alert CloudEvent in the audit chain.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");

    let protos = &[
        proto_root.join("spendguard/tokenizer/v1/tokenizer.proto"),
        // SLICE_05 additions: CloudEvent envelope + AppendEvents client.
        proto_root.join("spendguard/common/v1/common.proto"),
        proto_root.join("spendguard/canonical_ingest/v1/canonical_ingest.proto"),
    ];
    let includes = &[proto_root.clone()];

    tonic_build::configure()
        .build_server(true)
        // emit client too — calibration / output_predictor link this
        // crate, and SLICE_05 needs the canonical_ingest client for
        // drift_alert emission.
        .build_client(true)
        .bytes(["."])
        .compile_protos(protos, includes)?;

    for p in protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    Ok(())
}
