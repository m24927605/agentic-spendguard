//! Cursor wire envelope protobuf types.
//!
//! This module re-exports the prost-generated structs from
//! `cursor.proto`. The proto package is `spendguard.cursor_codec.v1alpha1`
//! so the generated module is `spendguard.cursor_codec.v1alpha1`; we
//! include it under [`gen`] and re-export the leaf types here.
//!
//! Per D17 design.md §8 decision 6 and review-standards.md §2 (R1):
//! field numbers match the observed wire shape; field names are
//! SpendGuard-chosen. No vendor source is included.

#[allow(clippy::all, missing_docs)]
mod gen {
    include!(concat!(
        env!("OUT_DIR"),
        "/spendguard.cursor_codec.v1alpha1.rs"
    ));
}

pub use gen::{CursorChatRequest, CursorChatResponseChunk, Message};
