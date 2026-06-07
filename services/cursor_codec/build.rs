// build.rs — generates `cursor_proto::*` from `src/proto/cursor.proto`
// AND (under the `mitm` feature) the sidecar_adapter / common gRPC
// client stubs the SLICE 6 MITM session state machine binds against.
//
// D17 SLICE 3: SpendGuard's own description of the observed Cursor wire
// envelope. Per review-standards §2 (R3): this build script must NEVER
// pull in a vendor `.proto` file. The only proto compiled here is the
// SpendGuard-authored reconstruction under `src/proto/`.
//
// D17 SLICE 6: when `mitm` is on the crate becomes a sidecar gRPC
// client. We compile the workspace-shared
// `proto/spendguard/sidecar_adapter/v1/adapter.proto` +
// `proto/spendguard/common/v1/common.proto` client-only (no server
// codegen) mirroring `services/egress_proxy/build.rs`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── Cursor wire envelope (always compiled — SLICE 3 charter) ─────────
    let cursor_proto_root = std::path::PathBuf::from("src/proto");
    let cursor_protos = &[cursor_proto_root.join("cursor.proto")];
    let cursor_includes = std::slice::from_ref(&cursor_proto_root);
    let out_dir = std::env::var("OUT_DIR")?;

    prost_build::Config::new()
        .out_dir(&out_dir)
        .compile_protos(cursor_protos, cursor_includes)?;

    for p in cursor_protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }

    // ── Sidecar adapter / common (only when `mitm` is on) ────────────────
    //
    // We always compile so `cargo build` with NO features emits the same
    // module set the `mitm` feature exposes (client stubs are dead code
    // when the consumer doesn't enable the feature, but they compile
    // cleanly under proto3 additive evolution). Mirrors
    // `services/egress_proxy/build.rs` so the generated module path is
    // identical: `tonic::include_proto!("spendguard.sidecar_adapter.v1")`.
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
