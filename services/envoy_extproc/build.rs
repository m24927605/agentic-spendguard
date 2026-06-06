// build.rs — tonic-build wiring for the vendored Envoy ExternalProcessor proto.
//
// Spec refs:
//   - docs/specs/coverage/D01_envoy_extproc/implementation.md §1 layout
//   - docs/slices/COV_01_envoy_extproc_skeleton.md §2 (tonic-build against
//     vendored ExtProc proto under proto/envoy/service/ext_proc/v3/)
//
// SLICE 1 builds BOTH server stubs (needed: this binary IS the
// ExternalProcessor) and client stubs (small surface; useful for the
// smoke test in tests/handshake_smoke.rs).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("proto");

    let protos = &[
        proto_root.join("envoy/config/core/v3/base.proto"),
        proto_root.join("envoy/type/v3/http_status.proto"),
        proto_root.join("envoy/extensions/filters/http/ext_proc/v3/processing_mode.proto"),
        proto_root.join("envoy/service/ext_proc/v3/external_processor.proto"),
    ];
    let includes = &[proto_root.clone()];

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .bytes(["."])
        .compile_protos(protos, includes)?;

    for p in protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    println!("cargo:rerun-if-changed=proto/VENDOR.md");
    Ok(())
}
