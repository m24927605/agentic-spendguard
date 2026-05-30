// build.rs — protobuf codegen for stats_aggregator's signed CloudEvent
// emission to canonical_ingest.
//
// Spec ref stats-aggregator-spec-v1alpha1.md §7.2.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");

    let protos = &[
        proto_root.join("spendguard/common/v1/common.proto"),
        proto_root.join("spendguard/canonical_ingest/v1/canonical_ingest.proto"),
    ];
    let includes = &[proto_root.clone()];

    tonic_build::configure()
        // server side: stats_aggregator does NOT expose a gRPC service
        // (per spec §2.1 — daemon + scheduler, not an RPC service).
        .build_server(false)
        // client side: emits AppendEvents to canonical_ingest for the
        // signed prediction_drift_alert CloudEvent.
        .build_client(true)
        .bytes(["."])
        .compile_protos(protos, includes)?;

    for p in protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    Ok(())
}
