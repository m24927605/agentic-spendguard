// build.rs — protobuf codegen via tonic-build.
//
// Inputs are the canonical wire-protocol artifacts in the workspace
// `proto/` directory (locked at Stage 2 / Phase 2A round 2).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");

    let protos = &[
        proto_root.join("spendguard/common/v1/common.proto"),
        proto_root.join("spendguard/ledger/v1/ledger.proto"),
    ];

    let includes = &[proto_root.clone()];

    tonic_build::configure()
        .build_server(true)
        .build_client(false)
        .bytes(["."])  // use Bytes for `bytes` fields
        .compile_protos(protos, includes)?;

    for p in protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    Ok(())
}
