// build.rs — tonic-build wiring for the vendored Envoy ExternalProcessor proto.
//
// Spec refs:
//   - docs/specs/coverage/D01_envoy_extproc/implementation.md §1 layout
//   - docs/internal/slices/COV_01_envoy_extproc_skeleton.md §2 (tonic-build against
//     vendored ExtProc proto under proto/envoy/service/ext_proc/v3/)
//   - docs/internal/slices/COV_03_envoy_extproc_budget_query.md §"Files touched"
//     (SLICE 3 adds the SpendGuard sidecar_adapter proto as a tonic CLIENT
//     stub so the new sidecar_client.rs can invoke RequestDecision)
//
// Build emits BOTH server stubs for the Envoy ExtProc proto (this binary
// IS the ExternalProcessor) and client stubs for both ExtProc (smoke
// test) AND the SpendGuard sidecar_adapter (SLICE 3 budget query). The
// sidecar adapter server stubs are also emitted to keep the test path
// honest — integration tests in tests/handshake_smoke.rs stand up a mock
// SidecarAdapter server over UDS, exercising the same client path
// production hits.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let envoy_proto_root = std::path::PathBuf::from("proto");
    // SpendGuard protos live in the repo-root /proto tree shared with
    // services/egress_proxy. Use a relative path from this crate so the
    // build is hermetic vs the workspace layout.
    let spendguard_proto_root = std::path::PathBuf::from("../../proto");

    // Envoy vendored protos — server + client stubs.
    let envoy_protos = &[
        envoy_proto_root.join("envoy/config/core/v3/base.proto"),
        envoy_proto_root.join("envoy/type/v3/http_status.proto"),
        envoy_proto_root.join("envoy/extensions/filters/http/ext_proc/v3/processing_mode.proto"),
        envoy_proto_root.join("envoy/service/ext_proc/v3/external_processor.proto"),
    ];
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .bytes(["."])
        .compile_protos(envoy_protos, std::slice::from_ref(&envoy_proto_root))?;
    for p in envoy_protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    println!("cargo:rerun-if-changed=proto/VENDOR.md");

    // SpendGuard sidecar adapter protos — client stubs (production hot
    // path) + server stubs (mock sidecar in handshake_smoke integration).
    let sg_protos = &[
        spendguard_proto_root.join("spendguard/common/v1/common.proto"),
        spendguard_proto_root.join("spendguard/sidecar_adapter/v1/adapter.proto"),
    ];
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .bytes(["."])
        .compile_protos(sg_protos, std::slice::from_ref(&spendguard_proto_root))?;
    for p in sg_protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }

    Ok(())
}
