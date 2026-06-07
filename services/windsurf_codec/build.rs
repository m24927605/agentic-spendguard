// build.rs — generates `windsurf_proto::*` from `src/proto/windsurf.proto`
// AND (under the `mitm` feature) the sidecar_adapter / common gRPC
// client stubs the SLICE 78 MITM session state machine binds against.
//
// D18 SLICE 75-76: SpendGuard's own description of the observed Windsurf
// Cascade wire envelope. Per review-standards: this build script must
// NEVER pull in a vendor `.proto` file. The only proto compiled here is
// the SpendGuard-authored reconstruction under `src/proto/`.
//
// D18 SLICE 78: when `mitm` is on the crate becomes a sidecar gRPC
// client. We compile the workspace-shared
// `proto/spendguard/sidecar_adapter/v1/adapter.proto` +
// `proto/spendguard/common/v1/common.proto` client-only (no server
// codegen) mirroring `services/cursor_codec/build.rs`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── Windsurf Cascade wire envelope (always compiled — SLICE 75 charter) ─
    let windsurf_proto_root = std::path::PathBuf::from("src/proto");
    let windsurf_protos = &[windsurf_proto_root.join("windsurf.proto")];
    let windsurf_includes = std::slice::from_ref(&windsurf_proto_root);
    let out_dir = std::env::var("OUT_DIR")?;

    prost_build::Config::new()
        .out_dir(&out_dir)
        .compile_protos(windsurf_protos, windsurf_includes)?;

    for p in windsurf_protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }

    // ── Sidecar adapter / common (only when `mitm` is on) ────────────────
    //
    // Mirrors `services/cursor_codec/build.rs` so the generated module
    // path is identical: `tonic::include_proto!("spendguard.sidecar_adapter.v1")`.
    //
    // We do this only when `CARGO_FEATURE_MITM` is set so the default
    // build (no feature) keeps the prost-only dep surface — tonic-build
    // would otherwise pull in workspace protoc at no-feature time.
    if std::env::var("CARGO_FEATURE_MITM").is_ok() {
        let workspace_proto_root = std::path::PathBuf::from("../../proto");
        let sidecar_protos = &[
            workspace_proto_root.join("spendguard/common/v1/common.proto"),
            workspace_proto_root.join("spendguard/sidecar_adapter/v1/adapter.proto"),
        ];
        let sidecar_includes = std::slice::from_ref(&workspace_proto_root);

        tonic_build::configure()
            .build_server(false)
            .build_client(true)
            .out_dir(&out_dir)
            .compile_protos(sidecar_protos, sidecar_includes)?;

        for p in sidecar_protos {
            println!("cargo:rerun-if-changed={}", p.display());
        }
    }
    Ok(())
}
