fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");
    let protos = &[
        proto_root.join("spendguard/common/v1/common.proto"),
        proto_root.join("spendguard/canonical_ingest/v1/canonical_ingest.proto"),
    ];
    let includes = &[proto_root.clone()];
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
