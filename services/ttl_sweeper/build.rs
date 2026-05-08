fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");

    let protos = &[
        proto_root.join("spendguard/common/v1/common.proto"),
        proto_root.join("spendguard/ledger/v1/ledger.proto"),
    ];
    let includes = &[proto_root.clone()];

    // TTL Sweeper is a CLIENT of ledger gRPC; only client stubs needed.
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
