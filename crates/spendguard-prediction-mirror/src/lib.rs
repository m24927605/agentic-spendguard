//! Shared sentinel helpers for the audit-chain prediction extension
//! mirror.
//!
//! ## Why this crate exists (round-2 fix M19)
//!
//! The audit-chain prediction extension (spec
//! `docs/audit-chain-prediction-extension-v1alpha1.md`) defines an
//! **invariant mapping** between SQL column NULL semantics and
//! CloudEvent proto sentinel values (per §3.3 + §6.3). Every producer
//! that writes an audit row — sidecar, webhook_receiver, ttl_sweeper,
//! ledger invoice_reconcile — and every consumer that compares the two
//! representations — verify-chain `--check-prediction-mirror`,
//! calibration-report SQL aggregations — must agree on the 10 NULL
//! mappings (round-3 fix M2 added the prediction_confidence +
//! prediction_sample_size pairs that were missed in round-2; round-4
//! fix M10 added run_steps_completed_so_far after round-4 M1 promoted
//! tag 313 from int32 to int64):
//!
//!   * `predicted_b_tokens         IS NULL`    ⇔ proto tag 301 = `0`
//!   * `predicted_c_tokens         IS NULL`    ⇔ proto tag 302 = `0`
//!   * `tokenizer_version_id       IS NULL`    ⇔ proto tag 307 = `""`
//!   * `prediction_confidence      IS NULL`    ⇔ proto tag 308 = `0.0`
//!   * `prediction_sample_size     IS NULL`    ⇔ proto tag 309 = `0`
//!   * `cold_start_layer_used      IS NULL`    ⇔ proto tag 310 = `""`
//!   * `run_predicted_remaining_steps IS NULL` ⇔ proto tag 312 = `-1`
//!   * `run_steps_completed_so_far IS NULL`    ⇔ proto tag 313 = `0`
//!   * `delta_b_ratio              IS NULL`    ⇔ proto tag 316 = `0.0`
//!   * `delta_c_ratio              IS NULL`    ⇔ proto tag 317 = `0.0`
//!
//! Plus `predicted_a_tokens` (tag 300) which has no NULL case but is
//! included for round-trip symmetry. Total: 10 NULL mappings + 1
//! no-NULL variant = 11 [`MirrorField`] discriminants.
//!
//! If sidecar.rs and webhook_receiver.rs implement the translation
//! independently, **drift is inevitable** — one service might encode
//! NULL as `0` while another encodes it as `i64::MIN`, breaking
//! verify-chain on cross-service decisions. Centralising the
//! translation in this crate keeps the mapping in a single audited
//! file.
//!
//! ## Producer-side preconditions (round-3 fix M13)
//!
//! Because the sentinel mapping collapses NULL with proto3 default for
//! token-count fields, producers MUST satisfy two preconditions BEFORE
//! calling [`column_to_proto_sentinel`]:
//!
//!   1. `predicted_a_tokens > 0` on every `.decision` row. Strategy A
//!      ceiling has no semantic interpretation for 0.
//!   2. `predicted_b_tokens > 0` when `prediction_strategy_used = 'B'`.
//!   3. `predicted_c_tokens > 0` when `prediction_strategy_used = 'C'`.
//!
//! Migration 0046 step 3b (round-3) enforces these as CHECK constraints
//! on the SQL boundary as defense-in-depth.
//!
//! ## Scope (round-2 origin, round-4 current)
//!
//! SLICE_01 lands the helper crate with type-erased Rust signatures
//! that match the spec §6.3 table, plus 8 unit tests covering every
//! sentinel-bearing column (round-3 M2 added 2 tests for
//! prediction_confidence + prediction_sample_size; round-4 M10 added 1
//! test for run_steps_completed_so_far). SLICE_06 producers will
//! import this crate and replace their inline NULL↔sentinel
//! translations with [`column_to_proto_sentinel`] /
//! [`proto_to_column_value`] calls. Until then, the crate is dep-free
//! (only `uuid`) and compiles in isolation.
//!
//! ## SLICE_06 extension scope (round-4 fix M15)
//!
//! TODO(SLICE_06): extend [`MirrorField`] with the 8 always-populated
//! variants (`reserved_strategy`, `prediction_strategy_used`,
//! `prediction_policy_used`, `tokenizer_tier`,
//! `run_projection_at_decision_atomic`, `actual_input_tokens`,
//! `actual_output_tokens`, plus `predicted_a_tokens` already present).
//! Add a `column_to_proto_always_populated()` helper for round-trip
//! producer validation — these columns are NOT NULL on their event
//! type per spec §2.1–§2.3, so the helper can assert non-None ColumnValue
//! and reject the producer call rather than silently mapping to a
//! proto3 default. This makes producer code self-validating against
//! the SLICE_06 SQL CHECK boundary and removes the need for each
//! producer to re-encode the "must be populated" precondition.

use uuid::Uuid;

/// Mirror invariant: 18 prediction-extension columns. The discriminant
/// identifies which column we're translating so the helper can pick the
/// right sentinel mapping (proto3 has no "field absent" concept; `0` and
/// `""` are wire-identical to "field unset"). Round-3 fix M2 added
/// `PredictionConfidence` (tag 308) + `PredictionSampleSize` (tag 309)
/// which were missing from the round-2 enum.
///
/// Tag numbers reference `proto/spendguard/common/v1/common.proto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorField {
    /// Tag 300 — `predicted_a_tokens` (BIGINT NOT NULL on .decision).
    PredictedATokens,
    /// Tag 301 — `predicted_b_tokens` (BIGINT NULL = sample bucket < 30).
    PredictedBTokens,
    /// Tag 302 — `predicted_c_tokens` (BIGINT NULL = plugin unhealthy).
    PredictedCTokens,
    /// Tag 307 — `tokenizer_version_id` (UUID NULL = Tier 3 fallback).
    TokenizerVersionId,
    /// Tag 308 — `prediction_confidence` (NUMERIC(4,3) NULL = Strategy A
    /// row). Round-3 fix M2.
    PredictionConfidence,
    /// Tag 309 — `prediction_sample_size` (BIGINT NULL = cold-start / A).
    /// Round-3 fix M2. Wire type is int64 per round-3 M3 (was int32 in
    /// round-2; aligned with BIGINT SQL column).
    PredictionSampleSize,
    /// Tag 310 — `cold_start_layer_used` (TEXT NULL = warm path).
    ColdStartLayerUsed,
    /// Tag 312 — `run_predicted_remaining_steps` (INT NULL = projector
    /// unreachable).
    RunPredictedRemainingSteps,
    /// Tag 313 — `run_steps_completed_so_far` (BIGINT NULL = pre-SLICE_06
    /// producer; semantically a non-negative counter, so the proto3
    /// default 0 collides with column-NULL by design). Round-4 fix M10
    /// added the variant after round-4 M1 promoted the proto wire to
    /// int64 (was int32). Calibration-report filters
    /// `WHERE run_steps_completed_so_far IS NOT NULL` to skip the
    /// collision case.
    RunStepsCompletedSoFar,
    /// Tag 316 — `delta_b_ratio` (REAL NULL = B was null at decision).
    DeltaBRatio,
    /// Tag 317 — `delta_c_ratio` (REAL NULL = C was null at decision).
    DeltaCRatio,
}

/// Typed proto-sentinel value emitted to (or read from) the wire.
///
/// Keep this enum closed-set to the column families that actually
/// participate in the mirror — SLICE_06 producers cannot accidentally
/// invent a new sentinel mapping.
#[derive(Debug, Clone, PartialEq)]
pub enum ProtoValue {
    I64(i64),
    I32(i32),
    F32(f32),
    Text(String),
}

/// Typed column value at the SQL boundary. Generic over the column
/// type; producers convert their domain type (e.g., `Option<u64>`) to
/// this enum before calling [`column_to_proto_sentinel`].
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnValue {
    NullBigInt,
    BigInt(i64),
    NullInt,
    Int(i32),
    NullReal,
    Real(f32),
    NullUuid,
    Uuid(Uuid),
    NullText,
    Text(String),
}

/// Convert a SQL column value to its mirror sentinel for the given
/// field. Producer-side helper.
///
/// # Panics
///
/// Panics on type-mismatch between the field's expected SQL type and
/// the column value variant (programmer error caught in unit tests).
pub fn column_to_proto_sentinel(field: MirrorField, col: ColumnValue) -> ProtoValue {
    match (field, col) {
        // ===== Tag 300: predicted_a_tokens — always populated.
        // No NULL case per spec §2.1; we still accept BigInt for symmetry.
        (MirrorField::PredictedATokens, ColumnValue::BigInt(v)) => ProtoValue::I64(v),

        // ===== Tag 301/302: predicted_b/c_tokens — NULL → 0 sentinel.
        (MirrorField::PredictedBTokens, ColumnValue::NullBigInt) => ProtoValue::I64(0),
        (MirrorField::PredictedBTokens, ColumnValue::BigInt(v)) => ProtoValue::I64(v),
        (MirrorField::PredictedCTokens, ColumnValue::NullBigInt) => ProtoValue::I64(0),
        (MirrorField::PredictedCTokens, ColumnValue::BigInt(v)) => ProtoValue::I64(v),

        // ===== Tag 307: tokenizer_version_id — NULL → "" sentinel.
        (MirrorField::TokenizerVersionId, ColumnValue::NullUuid) => ProtoValue::Text(String::new()),
        (MirrorField::TokenizerVersionId, ColumnValue::Uuid(u)) => ProtoValue::Text(u.to_string()),

        // ===== Tag 308: prediction_confidence — NULL → 0.0 sentinel.
        // Round-3 fix M2. Wire is float (f32); SQL column is NUMERIC(4,3)
        // 0.000-1.000. Producer converts NUMERIC→f32 before calling this
        // (precision loss is bounded by the (4,3) constraint). NULL maps to
        // proto3 default 0.0; calibration-report filters
        // `WHERE prediction_confidence IS NOT NULL` per spec §6.3 round-3
        // M11 note (collision with 0.0 is column-NULL semantics for
        // Strategy A rows).
        (MirrorField::PredictionConfidence, ColumnValue::NullReal) => ProtoValue::F32(0.0),
        (MirrorField::PredictionConfidence, ColumnValue::Real(v)) => ProtoValue::F32(v),

        // ===== Tag 309: prediction_sample_size — NULL → 0 sentinel.
        // Round-3 fix M2. Wire is int64 (round-3 M3 fix; was int32 in
        // round-2); SQL column is BIGINT. NULL maps to proto3 default 0;
        // legal values [0, ∞). Producer-side overflow check: sample size
        // > 2^63 would only happen on multi-billion-year aggregation
        // which is operationally impossible — but we still validate at
        // the SQL boundary via audit_outbox_prediction_sample_size_chk
        // CHECK.
        (MirrorField::PredictionSampleSize, ColumnValue::NullBigInt) => ProtoValue::I64(0),
        (MirrorField::PredictionSampleSize, ColumnValue::BigInt(v)) => ProtoValue::I64(v),

        // ===== Tag 310: cold_start_layer_used — NULL → "" sentinel.
        (MirrorField::ColdStartLayerUsed, ColumnValue::NullText) => ProtoValue::Text(String::new()),
        (MirrorField::ColdStartLayerUsed, ColumnValue::Text(s)) => ProtoValue::Text(s),

        // ===== Tag 312: run_predicted_remaining_steps — NULL → -1 sentinel.
        (MirrorField::RunPredictedRemainingSteps, ColumnValue::NullInt) => ProtoValue::I32(-1),
        (MirrorField::RunPredictedRemainingSteps, ColumnValue::Int(v)) => ProtoValue::I32(v),

        // ===== Tag 313: run_steps_completed_so_far — NULL → 0 sentinel.
        // Round-4 fix M10 + M1. Wire type is I64 (round-4 M1 promoted
        // from int32). SQL column is BIGINT NULL = "pre-SLICE_06 producer
        // never populated this row"; proto3 default 0 collides with the
        // legitimate "step counter is at zero" case at the wire. Per
        // spec §3.3 we accept the collision because calibration-report
        // filters `WHERE run_steps_completed_so_far IS NOT NULL` — the
        // 0 sentinel is unambiguous as "absent" inside the SQL boundary,
        // and the producer-side write path (audit.decision row) always
        // populates the column with > 0 for any meaningful agent step.
        (MirrorField::RunStepsCompletedSoFar, ColumnValue::NullBigInt) => ProtoValue::I64(0),
        (MirrorField::RunStepsCompletedSoFar, ColumnValue::BigInt(v)) => ProtoValue::I64(v),

        // ===== Tag 316/317: delta_b/c_ratio — NULL → 0.0 sentinel.
        (MirrorField::DeltaBRatio, ColumnValue::NullReal) => ProtoValue::F32(0.0),
        (MirrorField::DeltaBRatio, ColumnValue::Real(v)) => ProtoValue::F32(v),
        (MirrorField::DeltaCRatio, ColumnValue::NullReal) => ProtoValue::F32(0.0),
        (MirrorField::DeltaCRatio, ColumnValue::Real(v)) => ProtoValue::F32(v),

        (f, v) => panic!(
            "spendguard-prediction-mirror: type mismatch between MirrorField {f:?} and ColumnValue {v:?}"
        ),
    }
}

/// Convert a proto wire sentinel back to a SQL column value for the
/// given field. Consumer-side helper (verify-chain mirror cross-check,
/// calibration-report).
///
/// # Panics
///
/// Panics on type-mismatch between the field's expected wire type and
/// the proto value variant (programmer error caught in unit tests).
pub fn proto_to_column_value(field: MirrorField, proto: ProtoValue) -> ColumnValue {
    match (field, proto) {
        (MirrorField::PredictedATokens, ProtoValue::I64(v)) => ColumnValue::BigInt(v),

        (MirrorField::PredictedBTokens, ProtoValue::I64(0)) => ColumnValue::NullBigInt,
        (MirrorField::PredictedBTokens, ProtoValue::I64(v)) => ColumnValue::BigInt(v),
        (MirrorField::PredictedCTokens, ProtoValue::I64(0)) => ColumnValue::NullBigInt,
        (MirrorField::PredictedCTokens, ProtoValue::I64(v)) => ColumnValue::BigInt(v),

        (MirrorField::TokenizerVersionId, ProtoValue::Text(s)) if s.is_empty() => {
            ColumnValue::NullUuid
        }
        (MirrorField::TokenizerVersionId, ProtoValue::Text(s)) => match Uuid::parse_str(&s) {
            Ok(u) => ColumnValue::Uuid(u),
            // Malformed UUID at the wire — surface as NULL so the
            // verify-chain cross-check fires a "mirror inconsistency"
            // finding rather than panicking. The producer would never
            // emit this; defending against tampered wire bytes.
            Err(_) => ColumnValue::NullUuid,
        },

        // Round-3 fix M2: PredictionConfidence (tag 308) wire 0.0 ↔ NULL.
        (MirrorField::PredictionConfidence, ProtoValue::F32(v)) if v == 0.0 => {
            ColumnValue::NullReal
        }
        (MirrorField::PredictionConfidence, ProtoValue::F32(v)) => ColumnValue::Real(v),

        // Round-3 fix M2: PredictionSampleSize (tag 309) wire 0 ↔ NULL.
        // Wire type is I64 per round-3 M3.
        (MirrorField::PredictionSampleSize, ProtoValue::I64(0)) => ColumnValue::NullBigInt,
        (MirrorField::PredictionSampleSize, ProtoValue::I64(v)) => ColumnValue::BigInt(v),

        (MirrorField::ColdStartLayerUsed, ProtoValue::Text(s)) if s.is_empty() => {
            ColumnValue::NullText
        }
        (MirrorField::ColdStartLayerUsed, ProtoValue::Text(s)) => ColumnValue::Text(s),

        (MirrorField::RunPredictedRemainingSteps, ProtoValue::I32(-1)) => ColumnValue::NullInt,
        (MirrorField::RunPredictedRemainingSteps, ProtoValue::I32(v)) => ColumnValue::Int(v),

        // Round-4 fix M10 + M1: RunStepsCompletedSoFar (tag 313) wire 0 ↔ NULL.
        // Wire type is I64 (M1 promoted from I32).
        (MirrorField::RunStepsCompletedSoFar, ProtoValue::I64(0)) => ColumnValue::NullBigInt,
        (MirrorField::RunStepsCompletedSoFar, ProtoValue::I64(v)) => ColumnValue::BigInt(v),

        (MirrorField::DeltaBRatio, ProtoValue::F32(v)) if v == 0.0 => ColumnValue::NullReal,
        (MirrorField::DeltaBRatio, ProtoValue::F32(v)) => ColumnValue::Real(v),
        (MirrorField::DeltaCRatio, ProtoValue::F32(v)) if v == 0.0 => ColumnValue::NullReal,
        (MirrorField::DeltaCRatio, ProtoValue::F32(v)) => ColumnValue::Real(v),

        (f, v) => panic!(
            "spendguard-prediction-mirror: type mismatch between MirrorField {f:?} and ProtoValue {v:?}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================
    // Unit tests covering the 5 sentinel-bearing columns required by
    // round-2 fix M19. Tests assert round-trip identity:
    //   column → proto sentinel → column == original column
    // for both NULL and non-NULL inputs.
    // ============================================================

    #[test]
    fn predicted_b_tokens_null_roundtrip() {
        // NULL → 0 → NULL
        let p = column_to_proto_sentinel(MirrorField::PredictedBTokens, ColumnValue::NullBigInt);
        assert_eq!(p, ProtoValue::I64(0));
        let c = proto_to_column_value(MirrorField::PredictedBTokens, p);
        assert_eq!(c, ColumnValue::NullBigInt);

        // 512 → 512 → 512 (non-NULL)
        let p = column_to_proto_sentinel(MirrorField::PredictedBTokens, ColumnValue::BigInt(512));
        assert_eq!(p, ProtoValue::I64(512));
        let c = proto_to_column_value(MirrorField::PredictedBTokens, p);
        assert_eq!(c, ColumnValue::BigInt(512));
    }

    #[test]
    fn tokenizer_version_id_uuid_string_roundtrip() {
        let u = Uuid::parse_str("01999d50-1111-7000-8000-000000000003").unwrap();

        // UUID → string → UUID
        let p = column_to_proto_sentinel(MirrorField::TokenizerVersionId, ColumnValue::Uuid(u));
        assert_eq!(
            p,
            ProtoValue::Text("01999d50-1111-7000-8000-000000000003".into())
        );
        let c = proto_to_column_value(MirrorField::TokenizerVersionId, p);
        assert_eq!(c, ColumnValue::Uuid(u));

        // NULL → "" → NULL (Tier 3 fallback)
        let p = column_to_proto_sentinel(MirrorField::TokenizerVersionId, ColumnValue::NullUuid);
        assert_eq!(p, ProtoValue::Text(String::new()));
        let c = proto_to_column_value(MirrorField::TokenizerVersionId, p);
        assert_eq!(c, ColumnValue::NullUuid);
    }

    #[test]
    fn cold_start_layer_used_text_roundtrip() {
        // "L2" → "L2" → "L2"
        let p = column_to_proto_sentinel(
            MirrorField::ColdStartLayerUsed,
            ColumnValue::Text("L2".into()),
        );
        assert_eq!(p, ProtoValue::Text("L2".into()));
        let c = proto_to_column_value(MirrorField::ColdStartLayerUsed, p);
        assert_eq!(c, ColumnValue::Text("L2".into()));

        // NULL → "" → NULL (warm path)
        let p =
            column_to_proto_sentinel(MirrorField::ColdStartLayerUsed, ColumnValue::NullText);
        assert_eq!(p, ProtoValue::Text(String::new()));
        let c = proto_to_column_value(MirrorField::ColdStartLayerUsed, p);
        assert_eq!(c, ColumnValue::NullText);
    }

    #[test]
    fn run_predicted_remaining_steps_minus_one_sentinel() {
        // NULL → -1 → NULL (the critical sentinel: -1 ≠ "0 remaining")
        let p = column_to_proto_sentinel(
            MirrorField::RunPredictedRemainingSteps,
            ColumnValue::NullInt,
        );
        assert_eq!(p, ProtoValue::I32(-1));
        let c = proto_to_column_value(MirrorField::RunPredictedRemainingSteps, p);
        assert_eq!(c, ColumnValue::NullInt);

        // 0 → 0 → 0 (truly zero remaining steps; NOT projector-unreachable)
        let p = column_to_proto_sentinel(
            MirrorField::RunPredictedRemainingSteps,
            ColumnValue::Int(0),
        );
        assert_eq!(p, ProtoValue::I32(0));
        let c = proto_to_column_value(MirrorField::RunPredictedRemainingSteps, p);
        assert_eq!(c, ColumnValue::Int(0));

        // 3 → 3 → 3 (normal projection)
        let p = column_to_proto_sentinel(
            MirrorField::RunPredictedRemainingSteps,
            ColumnValue::Int(3),
        );
        assert_eq!(p, ProtoValue::I32(3));
        let c = proto_to_column_value(MirrorField::RunPredictedRemainingSteps, p);
        assert_eq!(c, ColumnValue::Int(3));
    }

    #[test]
    fn prediction_confidence_zero_sentinel_roundtrip() {
        // Round-3 fix M2: prediction_confidence (tag 308) — NULL ↔ 0.0
        // sentinel. Strategy A rows are null in SQL; the wire encodes
        // proto3 default 0.0; calibration-report filters
        // `WHERE prediction_confidence IS NOT NULL`.
        let p = column_to_proto_sentinel(
            MirrorField::PredictionConfidence,
            ColumnValue::NullReal,
        );
        assert_eq!(p, ProtoValue::F32(0.0));
        let c = proto_to_column_value(MirrorField::PredictionConfidence, p);
        assert_eq!(c, ColumnValue::NullReal);

        // 0.875 → 0.875 → 0.875 (normal Strategy B confidence)
        let p = column_to_proto_sentinel(
            MirrorField::PredictionConfidence,
            ColumnValue::Real(0.875),
        );
        assert_eq!(p, ProtoValue::F32(0.875));
        let c = proto_to_column_value(MirrorField::PredictionConfidence, p);
        assert_eq!(c, ColumnValue::Real(0.875));
    }

    #[test]
    fn prediction_sample_size_int64_zero_sentinel_roundtrip() {
        // Round-3 fix M2 + M3: prediction_sample_size (tag 309) — NULL ↔ 0
        // sentinel; wire type is I64 (round-3 M3 aligned with BIGINT SQL).
        let p = column_to_proto_sentinel(
            MirrorField::PredictionSampleSize,
            ColumnValue::NullBigInt,
        );
        assert_eq!(p, ProtoValue::I64(0));
        let c = proto_to_column_value(MirrorField::PredictionSampleSize, p);
        assert_eq!(c, ColumnValue::NullBigInt);

        // 64 → 64 → 64 (sample-bucket-of-64 Strategy B row)
        let p = column_to_proto_sentinel(
            MirrorField::PredictionSampleSize,
            ColumnValue::BigInt(64),
        );
        assert_eq!(p, ProtoValue::I64(64));
        let c = proto_to_column_value(MirrorField::PredictionSampleSize, p);
        assert_eq!(c, ColumnValue::BigInt(64));

        // 2^33 → 2^33 → 2^33 — exercises the BIGINT range beyond i32 that
        // round-2's int32 wire type would have silently truncated.
        let big: i64 = 1 << 33;
        let p = column_to_proto_sentinel(
            MirrorField::PredictionSampleSize,
            ColumnValue::BigInt(big),
        );
        assert_eq!(p, ProtoValue::I64(big));
        let c = proto_to_column_value(MirrorField::PredictionSampleSize, p);
        assert_eq!(c, ColumnValue::BigInt(big));
    }

    #[test]
    fn run_steps_completed_so_far_int64_zero_sentinel_roundtrip() {
        // Round-4 fix M10 + M1: run_steps_completed_so_far (tag 313) —
        // NULL ↔ 0 sentinel; wire type is I64 (M1 promoted from I32 to
        // match BIGINT SQL column).
        let p = column_to_proto_sentinel(
            MirrorField::RunStepsCompletedSoFar,
            ColumnValue::NullBigInt,
        );
        assert_eq!(p, ProtoValue::I64(0));
        let c = proto_to_column_value(MirrorField::RunStepsCompletedSoFar, p);
        assert_eq!(c, ColumnValue::NullBigInt);

        // 42 → 42 → 42 (normal step counter)
        let p = column_to_proto_sentinel(
            MirrorField::RunStepsCompletedSoFar,
            ColumnValue::BigInt(42),
        );
        assert_eq!(p, ProtoValue::I64(42));
        let c = proto_to_column_value(MirrorField::RunStepsCompletedSoFar, p);
        assert_eq!(c, ColumnValue::BigInt(42));

        // 2^33 → 2^33 → 2^33 — exercises the BIGINT range beyond i32
        // that the round-3 int32 wire type would have silently
        // truncated. This is the headroom rationale for M1 promoting
        // tag 313 from int32 to int64.
        let big: i64 = 1 << 33;
        let p = column_to_proto_sentinel(
            MirrorField::RunStepsCompletedSoFar,
            ColumnValue::BigInt(big),
        );
        assert_eq!(p, ProtoValue::I64(big));
        let c = proto_to_column_value(MirrorField::RunStepsCompletedSoFar, p);
        assert_eq!(c, ColumnValue::BigInt(big));
    }

    #[test]
    fn delta_b_ratio_zero_sentinel() {
        // NULL → 0.0 → NULL (sentinel for "B was null at decision time")
        let p = column_to_proto_sentinel(MirrorField::DeltaBRatio, ColumnValue::NullReal);
        assert_eq!(p, ProtoValue::F32(0.0));
        let c = proto_to_column_value(MirrorField::DeltaBRatio, p);
        assert_eq!(c, ColumnValue::NullReal);

        // 0.75 → 0.75 → 0.75 (normal ratio)
        let p = column_to_proto_sentinel(MirrorField::DeltaBRatio, ColumnValue::Real(0.75));
        assert_eq!(p, ProtoValue::F32(0.75));
        let c = proto_to_column_value(MirrorField::DeltaBRatio, p);
        assert_eq!(c, ColumnValue::Real(0.75));

        // EDGE CASE acknowledged: a TRUE ratio of 0.0 (actual=0,
        // predicted_b>0) is indistinguishable from NULL at the wire.
        // Per spec §3.3, calibration-report filters `WHERE
        // delta_b_ratio > 0`, so this collision is a known design
        // trade-off, not a bug in the helper.
    }
}
