// build.rs — protobuf codegen for canonical ingest service.
//
// Round-2 fix M8 — prost unknown-field preservation:
//   prost 0.13 does NOT preserve proto3 unknown fields on
//   decode + re-encode (upstream issue tokio-rs/prost#879). For
//   SLICE_01 + SLICE_06 the audit-chain-prediction-extension-v1alpha1.md
//   §7.2 rollout invariant therefore reads:
//
//     "All canonical_ingest pods MUST be upgraded to the SLICE_01 proto
//      definitions BEFORE any sidecar / webhook_receiver / ttl_sweeper
//      pod starts writing tag-300+ fields. Otherwise, mid-upgrade
//      canonical_ingest pods would decode-then-re-encode CloudEvents
//      with tag-300+ fields stripped, producing canonical bytes that
//      DO NOT match the producer's signature (verify-chain regression)."
//
//   This is enforced operationally via the Helm chart's pod restart
//   policy + roll-out script (charts/spendguard/templates/migrations.yaml
//   NOTES.txt warns operators to run canonical_ingest restarts first).
//   When prost upstream lands unknown-field preservation we can drop
//   this constraint; tracking issue: services/canonical_ingest/Cargo.toml
//   comment near prost = "0.13".

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("../../proto");

    let protos = &[
        proto_root.join("spendguard/common/v1/common.proto"),
        proto_root.join("spendguard/canonical_ingest/v1/canonical_ingest.proto"),
    ];
    let includes = &[proto_root.clone()];

    tonic_build::configure()
        .build_server(true)
        .build_client(false)
        .bytes(["."])
        .compile_protos(protos, includes)?;

    for p in protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    Ok(())
}
