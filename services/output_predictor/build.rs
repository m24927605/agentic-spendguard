// build.rs — protobuf codegen for the output_predictor gRPC service.
//
// Spec ref output-predictor-service-spec-v1alpha1.md §2.1.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");

    let protos = &[proto_root.join("spendguard/output_predictor/v1/predictor.proto")];
    let includes = &[proto_root.clone()];

    tonic_build::configure()
        .build_server(true)
        // emit client too — SLICE_10 wires sidecar/egress_proxy as clients.
        .build_client(true)
        .compile_protos(protos, includes)?;

    for p in protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    Ok(())
}
