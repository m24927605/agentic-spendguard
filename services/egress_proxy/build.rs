fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");

    let protos = &[
        proto_root.join("spendguard/common/v1/common.proto"),
        proto_root.join("spendguard/sidecar_adapter/v1/adapter.proto"),
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
