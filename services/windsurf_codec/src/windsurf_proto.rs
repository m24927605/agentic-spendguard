//! Windsurf Cascade wire envelope protobuf types.
//!
//! This module re-exports the prost-generated structs from
//! `windsurf.proto`. The proto package is
//! `spendguard.windsurf_codec.v1alpha1` so the generated module is
//! `spendguard.windsurf_codec.v1alpha1`; we include it under [`gen`]
//! and re-export the leaf types here.
//!
//! Per D18 design.md §3 decision 6 (mirrored from D17 review-standards
//! R1): field numbers match the observed wire shape; field names are
//! SpendGuard-chosen. No vendor source is included.

#[allow(clippy::all, missing_docs)]
mod gen {
    include!(concat!(
        env!("OUT_DIR"),
        "/spendguard.windsurf_codec.v1alpha1.rs"
    ));
}

pub use gen::{
    CascadeMessage, CascadeRequest, CascadeResponseDelta, CascadeToolDecl, CascadeUsage,
};
