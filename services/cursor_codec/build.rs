// build.rs — generates `cursor_proto::*` from `src/proto/cursor.proto`.
//
// D17 SLICE 3: SpendGuard's own description of the observed Cursor wire
// envelope. Per review-standards §2 (R3): this build script must NEVER
// pull in a vendor `.proto` file. The only proto compiled here is the
// SpendGuard-authored reconstruction under `src/proto/`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from("src/proto");

    let protos = &[proto_root.join("cursor.proto")];
    let includes = std::slice::from_ref(&proto_root);

    // prost-build emits a single `cursor_codec_proto.rs` (matches the
    // package name in the .proto) into OUT_DIR; `cursor_proto.rs`
    // includes it via `include!(concat!(env!("OUT_DIR"), ...))`.
    prost_build::Config::new()
        .out_dir(std::env::var("OUT_DIR")?)
        .compile_protos(protos, includes)?;

    for p in protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    Ok(())
}
