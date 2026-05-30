// build.rs — protobuf codegen for the run_cost_projector gRPC service.
//
// Spec refs:
//   - run-cost-projector-spec-v1alpha1.md §2.1 (Project / TerminateRun proto)

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");

    let protos = &[
        proto_root.join("spendguard/run_cost_projector/v1/projector.proto"),
    ];
    let includes = &[proto_root.clone()];

    tonic_build::configure()
        .build_server(true)
        // Emit client too — sidecar (Phase E) dials Project from
        // services/sidecar/src/decision/transaction.rs.
        .build_client(true)
        .compile_protos(protos, includes)?;

    for p in protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    Ok(())
}
