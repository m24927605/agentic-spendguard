fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");

    let protos = &[proto_root.join("spendguard/cost_advisor/v1/cost_advisor.proto")];
    let includes = &[proto_root.clone()];

    tonic_build::configure()
        .build_server(false)
        .build_client(false)
        .bytes(["."])
        .compile_protos(protos, includes)?;

    for p in protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    Ok(())
}
