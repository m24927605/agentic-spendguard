fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");

    let protos = &[
        proto_root.join("spendguard/common/v1/common.proto"),
        proto_root.join("spendguard/sidecar_adapter/v1/adapter.proto"),
        // SLICE_10 Phase A: egress_proxy now dials output_predictor +
        // run_cost_projector directly so it can build the full ClaimEstimate
        // (17 audit cols + Strategy A/B/C) before the DecisionRequest reaches
        // the sidecar. Client-only stubs.
        proto_root.join("spendguard/output_predictor/v1/predictor.proto"),
        proto_root.join("spendguard/run_cost_projector/v1/projector.proto"),
    ];
    let includes = &[proto_root.clone()];

    // Egress proxy is a CLIENT of sidecar UDS gRPC; client stubs only.
    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .bytes(["."])
        .compile_protos(protos, includes)?;

    for p in protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    Ok(())
}
