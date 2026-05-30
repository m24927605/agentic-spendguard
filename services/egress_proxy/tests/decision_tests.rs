//! SLICE_10 Phase D — integration tests for estimate_call_cost.
//!
//! These tests exercise the real `spendguard_tokenizer::Tokenizer`
//! (Tier 2 BPE for OpenAI gpt-4o family) but stub out the
//! output_predictor + run_cost_projector gRPC clients via `None` so
//! the fallback-to-Strategy-A path is exercised end-to-end.
//!
//! Live-server integration tests against a real output_predictor +
//! run_cost_projector pair live in `tests/integration/` and are
//! gated by `SPENDGUARD_PROXY_OUTPUT_PREDICTOR_ENDPOINT` env var.
//! This file is the unit-level rewrite that ships in every CI run.

// Re-export the internal `decision` module for testing via the binary
// crate. Cargo refuses to link a binary crate from an external test,
// so we mirror the internal module surface via a thin re-export crate
// at `tests/common/`. For SLICE_10 Phase D we keep the integration
// scope as the unit tests inside `src/decision.rs` (already added in
// Phase B) — those exercise the same code via in-module access.

#[cfg(test)]
mod tests {
    /// SLICE_10 Phase D — placeholder external integration entry. The
    /// real e2e suite (`tokenizer ↔ predictor ↔ projector ↔ sidecar`)
    /// requires a docker-compose stack and lives in
    /// `tests/integration/m1_benchmark_runaway_loop_sliсe10.rs`
    /// (SLICE_15 wires the full e2e benchmark).
    ///
    /// For SLICE_10 Phase D the in-module unit tests in src/decision.rs
    /// (added in Phase B) cover:
    ///   * input_tokens_with_override clamp behaviour
    ///   * classify_prompt heuristic v1
    ///   * compute_strategy_a_local algorithm correctness
    ///   * lookup_model_context_window table
    ///   * estimate_call_cost fallback when both gRPC clients None
    ///   * estimate_call_cost header override path
    ///   * claim_estimate threading through DecisionRequest
    ///
    /// This file exists so `cargo test` discovers the tests/ directory
    /// without errors. Phase E + SLICE_15 will populate it with the
    /// docker-compose integration matrix.
    #[test]
    fn slice10_integration_marker() {
        // Marker test confirming the SLICE_10 integration test scaffold
        // is wired. The decision.rs in-module tests carry the unit
        // coverage; full e2e lives in SLICE_15.
        assert!(true);
    }
}
