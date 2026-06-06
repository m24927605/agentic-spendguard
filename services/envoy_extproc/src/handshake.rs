// Transport: SLICE 1-5 UDS per design.md §3.3 carve-out; SLICE 6 hard-switches to mTLS-TCP.
//! Handshake phase — open the sidecar UDS so the binary refuses to
//! serve traffic if the sidecar is unreachable (design §3.4 fail-closed).
//!
//! SLICE 1 scope:
//!   - On startup, dial the SpendGuard sidecar adapter UDS once (no
//!     protocol-level Handshake yet — the sidecar adapter proto stubs
//!     get pulled into this crate in SLICE 2 when RequestDecision wires
//!     up).
//!   - On each inbound ExtProc `Process` stream, the first
//!     `ProcessingRequest` is treated as the "handshake frame"; the
//!     server replies with a `ProcessingResponse::request_headers` ACK
//!     (CommonResponse status = CONTINUE) so a mock Envoy client gets
//!     a 200-equivalent and can close cleanly. SLICE 2 will replace
//!     this with the real per-phase translation.
//!
//! Mirrors the UDS dial pattern from
//! [`services/egress_proxy/src/sidecar_client.rs`].

use std::path::Path;
use std::time::Duration;

use thiserror::Error;
use tokio::net::UnixStream;
use tracing::{info, warn};

use crate::config::Config;

#[derive(Debug, Error)]
pub enum HandshakeError {
    #[error("sidecar UDS dial to {path} failed after {attempts} attempts ({deadline_s}s deadline): {source}")]
    SidecarUnreachable {
        path: String,
        attempts: u32,
        deadline_s: u64,
        #[source]
        source: std::io::Error,
    },
}

/// Best-effort dial of the sidecar UDS at startup. Retries with the
/// same backoff envelope as `services/egress_proxy/src/sidecar_client.rs`
/// so containers brought up via `docker-compose depends_on: service_started`
/// don't lose the race.
///
/// Returns `Ok(true)` once the socket connect succeeds. SLICE 1 does
/// NOT yet send a sidecar `Handshake` RPC — that's deferred to SLICE 2
/// when the sidecar adapter proto gets imported into this crate. Until
/// then, a successful connect is the strongest signal we can produce.
///
/// On total failure inside the deadline, returns
/// [`HandshakeError::SidecarUnreachable`] — the caller (main) exits
/// non-zero, matching the Round 1 review standards §2.2 "process exits
/// non-zero on handshake failure" requirement.
pub async fn dial_sidecar_with_retry(cfg: &Config) -> Result<bool, HandshakeError> {
    let path = cfg.sidecar_uds_path.clone();
    let deadline = tokio::time::Instant::now() + cfg.sidecar_startup_deadline;
    let mut backoff = cfg.sidecar_initial_backoff;
    let mut attempts: u32 = 0;

    loop {
        attempts += 1;
        match UnixStream::connect(&path).await {
            Ok(_stream) => {
                info!(
                    path = %path.display(),
                    attempts,
                    "sidecar UDS reachable (SLICE 1 dial-only handshake)"
                );
                return Ok(true);
            }
            Err(e) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(HandshakeError::SidecarUnreachable {
                        path: path.display().to_string(),
                        attempts,
                        deadline_s: cfg.sidecar_startup_deadline.as_secs(),
                        source: e,
                    });
                }
                warn!(
                    path = %path.display(),
                    attempts,
                    err = %e,
                    next_backoff_ms = backoff.as_millis() as u64,
                    "sidecar UDS dial failed; retrying"
                );
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, cfg.sidecar_max_backoff);
            }
        }
    }
}

/// Lightweight one-shot dial without retry — useful in unit tests that
/// want to fail fast.
#[doc(hidden)]
pub async fn dial_sidecar_once(path: &Path) -> std::io::Result<()> {
    tokio::time::timeout(Duration::from_millis(100), UnixStream::connect(path))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "dial timeout"))?
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dial_sidecar_once_returns_io_error_when_unreachable() {
        let path = std::path::PathBuf::from("/tmp/spendguard-extproc-test-nonexistent.sock");
        let _ = std::fs::remove_file(&path);
        let err = dial_sidecar_once(&path).await.expect_err("must error");
        // ENOENT or ECONNREFUSED — both acceptable. The test only
        // checks the error path runs at all.
        assert!(matches!(
            err.kind(),
            std::io::ErrorKind::NotFound
                | std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::TimedOut
        ));
    }

    #[tokio::test]
    async fn dial_sidecar_with_retry_fails_fast_with_zero_deadline() {
        let cfg = Config::for_test("127.0.0.1:0".parse().unwrap());
        // Tighten deadline to a few ms so the test runs fast.
        let mut cfg = cfg;
        cfg.sidecar_startup_deadline = Duration::from_millis(10);
        cfg.sidecar_initial_backoff = Duration::from_millis(1);
        cfg.sidecar_max_backoff = Duration::from_millis(2);
        cfg.sidecar_uds_path =
            std::path::PathBuf::from("/tmp/spendguard-extproc-test-nonexistent.sock");
        let _ = std::fs::remove_file(&cfg.sidecar_uds_path);

        let err = dial_sidecar_with_retry(&cfg)
            .await
            .expect_err("must surface SidecarUnreachable");
        assert!(matches!(err, HandshakeError::SidecarUnreachable { .. }));
    }
}
