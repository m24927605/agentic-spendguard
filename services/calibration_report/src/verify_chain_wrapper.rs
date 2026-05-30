//! verify-chain integration wrapper (Phase A scaffold — full
//! implementation in Phase C).
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §3.4 +
//! `docs/audit-chain-prediction-extension-v1alpha1.md` §7.

use crate::report::VerifyChainFailure;

/// Phase A: not-implemented stub. Phase C wires this to the
/// canonical_ingest library's verifier.
pub async fn run_verify_chain_stub() -> Result<(), VerifyChainFailure> {
    Ok(())
}
