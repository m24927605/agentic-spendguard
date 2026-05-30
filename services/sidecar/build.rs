// build.rs — protobuf codegen for adapter UDS server, ledger client,
// canonical ingest client.
//
// Round-2 fix M8 (mirror on sidecar side): prost 0.13 does NOT preserve
// proto3 unknown fields on decode + re-encode. The SLICE_01 rollout
// invariant per audit-chain-prediction-extension-v1alpha1.md §7.2 is
// therefore that all canonical_ingest pods must be upgraded BEFORE any
// sidecar starts writing tag-300+ prediction fields. See the longer
// comment in services/canonical_ingest/build.rs for rationale.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");

    let protos = &[
        proto_root.join("spendguard/common/v1/common.proto"),
        proto_root.join("spendguard/sidecar_adapter/v1/adapter.proto"),
        proto_root.join("spendguard/ledger/v1/ledger.proto"),
        proto_root.join("spendguard/canonical_ingest/v1/canonical_ingest.proto"),
        // SLICE_09 Phase E: run_cost_projector client stub. Sidecar dials
        // Project from decision/transaction.rs after output_predictor.Predict
        // and before reserve stage. Per run-cost-projector-spec-v1alpha1.md §10
        // failure mode "projector unreachable from sidecar → conservative
        // fall-through (no RUN_* emitted; reservation correct via Strategy A)".
        proto_root.join("spendguard/run_cost_projector/v1/projector.proto"),
    ];
    let includes = &[proto_root.clone()];

    tonic_build::configure()
        // adapter UDS: server side
        // ledger / canonical_ingest: client side
        .build_server(true)
        .build_client(true)
        .bytes(["."])
        .compile_protos(protos, includes)?;

    for p in protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    Ok(())
}
