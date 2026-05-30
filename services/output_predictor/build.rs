// build.rs — protobuf codegen for the output_predictor gRPC service.
//
// Spec refs:
//   - output-predictor-service-spec-v1alpha1.md §2.1 (Predict proto)
//   - output-predictor-plugin-contract-v1alpha1.md §2.1 (CustomerPredictor
//     plugin proto — SLICE_07)

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");

    let protos = &[
        proto_root.join("spendguard/output_predictor/v1/predictor.proto"),
        // SLICE_07: customer plugin contract. We need the CLIENT stub to
        // dial customer-hosted plugin endpoints from strategy_c.rs.
        proto_root.join("spendguard/output_predictor_plugin/v1/plugin.proto"),
    ];
    let includes = &[proto_root.clone()];

    tonic_build::configure()
        .build_server(true)
        // emit client too — SLICE_10 wires sidecar/egress_proxy as clients
        // for the main predictor RPC; SLICE_07 uses the client stub for
        // the customer plugin RPC.
        .build_client(true)
        .compile_protos(protos, includes)?;

    for p in protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    Ok(())
}
