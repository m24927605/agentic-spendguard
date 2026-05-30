// build.rs — protobuf codegen for the tokenizer gRPC service.
//
// Spec ref `tokenizer-service-spec-v1alpha1.md` §2.2.
//
// SLICE_03 generates:
//   * spendguard.tokenizer.v1 (TokenizerService) — server side
//
// We deliberately do NOT pull in spendguard.common.v1 here — the
// tokenizer service surface is self-contained (it operates on plain
// strings + int64 token counts; no `TraceContext` / `Money` /
// `UnitRef` cross-references needed in SLICE_03). SLICE_05 might
// add a CloudEvent envelope for drift_alert events, at which point
// common.proto + the canonical_ingest client will join the
// `protos` slice.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");

    let protos = &[proto_root.join("spendguard/tokenizer/v1/tokenizer.proto")];
    let includes = &[proto_root.clone()];

    tonic_build::configure()
        .build_server(true)
        .build_client(true) // emit client too — calibration / output_predictor link this crate
        .bytes(["."])
        .compile_protos(protos, includes)?;

    for p in protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    Ok(())
}
