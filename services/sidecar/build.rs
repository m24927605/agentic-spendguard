// build.rs — protobuf codegen for adapter UDS server, ledger client,
// canonical ingest client.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");

    let protos = &[
        proto_root.join("spendguard/common/v1/common.proto"),
        proto_root.join("spendguard/sidecar_adapter/v1/adapter.proto"),
        proto_root.join("spendguard/ledger/v1/ledger.proto"),
        proto_root.join("spendguard/canonical_ingest/v1/canonical_ingest.proto"),
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
