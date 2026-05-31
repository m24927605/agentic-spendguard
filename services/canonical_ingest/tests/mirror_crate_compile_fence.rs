//! Round-4 fix M7: mirror crate compile fence.
//!
//! The round-3 wiring of `spendguard-prediction-mirror` as a dev-dependency
//! of canonical_ingest (services/canonical_ingest/Cargo.toml line 93) was
//! a build-link only — nothing in canonical_ingest actually calls the
//! crate's public API. Codex M7 flagged the risk: if SLICE_06 changes
//! the mirror crate's public surface (e.g., renames `MirrorField`,
//! changes the enum tags, or drops `column_to_proto_sentinel`),
//! canonical_ingest's CI keeps passing because nothing references the
//! API. SLICE_06 then lands a producer change that calls the new
//! surface but canonical_ingest still compiles against the old.
//!
//! This test exercises every public type + every public function once
//! so `cargo test --workspace` (or `cargo test -p
//! spendguard-canonical-ingest`) FAILS at the build step if the API
//! drifts. It does NOT assert behaviour — the mirror crate's own unit
//! tests cover that — it just catches breaking API changes.
//!
//! See docs/audit-chain-prediction-extension-v1alpha1.md §6.3 +
//! crates/spendguard-prediction-mirror/src/lib.rs preamble.

use spendguard_prediction_mirror::{
    column_to_proto_sentinel, proto_to_column_value, ColumnValue, MirrorField, ProtoValue,
};
use uuid::Uuid;

#[test]
fn mirror_crate_public_api_compiles() {
    // Construct every MirrorField variant. If any is renamed or removed
    // upstream, this fails to build.
    let fields = [
        MirrorField::PredictedATokens,
        MirrorField::PredictedBTokens,
        MirrorField::PredictedCTokens,
        MirrorField::TokenizerVersionId,
        MirrorField::PredictionConfidence,
        MirrorField::PredictionSampleSize,
        MirrorField::ColdStartLayerUsed,
        MirrorField::RunPredictedRemainingSteps,
        MirrorField::RunStepsCompletedSoFar, // round-4 M10 addition
        MirrorField::DeltaBRatio,
        MirrorField::DeltaCRatio,
    ];
    assert_eq!(fields.len(), 11, "expected 11 MirrorField variants");

    // Exercise column_to_proto_sentinel and proto_to_column_value for
    // representative inputs of each ColumnValue / ProtoValue variant.
    // Don't assert outcomes — that's the mirror crate's job. Only
    // assert the calls type-check.

    let p = column_to_proto_sentinel(MirrorField::PredictedBTokens, ColumnValue::NullBigInt);
    let _ = proto_to_column_value(MirrorField::PredictedBTokens, p);

    let p = column_to_proto_sentinel(MirrorField::PredictedATokens, ColumnValue::BigInt(1024));
    let _ = proto_to_column_value(MirrorField::PredictedATokens, p);

    let u = Uuid::nil();
    let p = column_to_proto_sentinel(MirrorField::TokenizerVersionId, ColumnValue::Uuid(u));
    let _ = proto_to_column_value(MirrorField::TokenizerVersionId, p);

    let p = column_to_proto_sentinel(MirrorField::TokenizerVersionId, ColumnValue::NullUuid);
    let _ = proto_to_column_value(MirrorField::TokenizerVersionId, p);

    let p = column_to_proto_sentinel(MirrorField::PredictionConfidence, ColumnValue::Real(0.5));
    let _ = proto_to_column_value(MirrorField::PredictionConfidence, p);

    let p = column_to_proto_sentinel(MirrorField::PredictionSampleSize, ColumnValue::BigInt(64));
    let _ = proto_to_column_value(MirrorField::PredictionSampleSize, p);

    let p = column_to_proto_sentinel(
        MirrorField::ColdStartLayerUsed,
        ColumnValue::Text("L2".into()),
    );
    let _ = proto_to_column_value(MirrorField::ColdStartLayerUsed, p);

    let p = column_to_proto_sentinel(MirrorField::ColdStartLayerUsed, ColumnValue::NullText);
    let _ = proto_to_column_value(MirrorField::ColdStartLayerUsed, p);

    let p = column_to_proto_sentinel(
        MirrorField::RunPredictedRemainingSteps,
        ColumnValue::NullInt,
    );
    let _ = proto_to_column_value(MirrorField::RunPredictedRemainingSteps, p);

    let p = column_to_proto_sentinel(MirrorField::RunPredictedRemainingSteps, ColumnValue::Int(7));
    let _ = proto_to_column_value(MirrorField::RunPredictedRemainingSteps, p);

    // Round-4 M10: RunStepsCompletedSoFar exercise — fails to build if
    // the variant is dropped or its mapping arms removed.
    let p = column_to_proto_sentinel(MirrorField::RunStepsCompletedSoFar, ColumnValue::NullBigInt);
    let _ = proto_to_column_value(MirrorField::RunStepsCompletedSoFar, p);

    let p = column_to_proto_sentinel(MirrorField::RunStepsCompletedSoFar, ColumnValue::BigInt(99));
    let _ = proto_to_column_value(MirrorField::RunStepsCompletedSoFar, p);

    let p = column_to_proto_sentinel(MirrorField::DeltaBRatio, ColumnValue::NullReal);
    let _ = proto_to_column_value(MirrorField::DeltaBRatio, p);

    let p = column_to_proto_sentinel(MirrorField::DeltaBRatio, ColumnValue::Real(0.75));
    let _ = proto_to_column_value(MirrorField::DeltaBRatio, p);

    let p = column_to_proto_sentinel(MirrorField::DeltaCRatio, ColumnValue::NullReal);
    let _ = proto_to_column_value(MirrorField::DeltaCRatio, p);

    let p = column_to_proto_sentinel(MirrorField::DeltaCRatio, ColumnValue::Real(0.25));
    let _ = proto_to_column_value(MirrorField::DeltaCRatio, p);

    // ProtoValue variants — construct each so a future rename / removal
    // fails to build.
    let _ = ProtoValue::I64(0);
    let _ = ProtoValue::I32(-1);
    let _ = ProtoValue::F32(0.0);
    let _ = ProtoValue::Text(String::new());
}
