//! Lifecycle drain (Sidecar §11).
//!
//! Triggers handled:
//!   * SIGTERM (K8s preStop, rolling restart)
//!   * spot interruption (Phase 2+; cloud-specific signal)
//!   * lambda shutdown (Phase 2+ via Extensions API)
//!
//! Drain semantics:
//!   1. Set `state.draining = true`. Decision RPCs return Draining.
//!   2. Wait up to `drain_window_seconds` for in-flight decisions to
//!      complete (their stages 4-7 already in flight).
//!   3. Flush pending canonical events (reqwest in-progress requests).
//!   4. Release any held fencing leases — defer to background renewer
//!      task to skip renewal; ledger TTL eventually frees them.
//!   5. Audit invariant: NEVER publish_effect after drain begins for any
//!      decision whose audit_decision is not yet durable. Stage 2 §4
//!      ensures audit_decision durability is atomic with reserve, so the
//!      only window of concern is between ReserveSet ack and
//!      publish_effect dispatch. The handler in adapter_uds checks
//!      state.is_draining() before issuing apply_mutation.

use std::time::Duration;

use tracing::{info, warn};

use crate::domain::state::SidecarState;

pub async fn run_drain(state: SidecarState, window: Duration) {
    info!(window_secs = window.as_secs(), "drain initiated");
    state.mark_draining();

    let deadline = tokio::time::Instant::now() + window;
    while tokio::time::Instant::now() < deadline {
        // Poll for outstanding work; placeholder for vertical slice expansion
        // (currently the adapter UDS server tracks active streams via tower
        // limit + axum's task counter; we sleep + check is_draining flag).
        tokio::time::sleep(Duration::from_millis(200)).await;
        // Future: check if active_decision_count == 0 and break early.
    }

    if tokio::time::Instant::now() >= deadline {
        warn!("drain deadline reached; some decisions may have been fenced/aborted");
    }
    info!("drain complete");
}
