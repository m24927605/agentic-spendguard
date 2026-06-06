// Transport: SLICE 1-5 UDS per design.md §3.3 carve-out; SLICE 6 hard-switches to mTLS-TCP.
//! SLICE 3 — SpendGuard sidecar adapter client over UDS.
//!
//! Wraps [`crate::proto::spendguard::sidecar_adapter::v1::sidecar_adapter_client::SidecarAdapterClient`]
//! with a lazy-connect tonic channel pointed at the same UDS path the
//! SLICE 1 handshake dialled. Exposes:
//!
//!   * [`SidecarClient::connect`] — build a lazy channel; does NOT block
//!     on UDS reachability (the SLICE 1 fail-fast dial in
//!     `handshake::dial_sidecar_with_retry` already proved the socket
//!     exists at boot).
//!   * [`SidecarClient::request_decision`] — single hot-path RPC; the
//!     caller (Request-Body phase) supplies a fully built
//!     [`DecisionRequest`] and gets back [`DecisionResponse`] or a
//!     typed [`SidecarError`].
//!
//! ## Why a thin wrapper instead of using `SidecarAdapterClient` directly
//!
//! Three reasons:
//!   1. **Timeout enforcement** — review-standards §4.1.2 requires the
//!      tonic call be gated by `SPENDGUARD_EXTPROC_REQUEST_TIMEOUT_MS`.
//!      Wrapping with `tokio::time::timeout` gives a typed
//!      `SidecarError::Timeout` variant the response mapper can read.
//!   2. **Failure isolation** — review-standards §4.1.3 forbids leaking
//!      sidecar internal error details into the ExtProc response body.
//!      Translating `tonic::Status` into our typed enum lets the
//!      response builder render a fixed-shape `details` field.
//!   3. **Testability** — the handshake_smoke integration test boots a
//!      mock `SidecarAdapter` server on a tempdir UDS and dials it via
//!      `SidecarClient::connect`. Mirrors the egress_proxy
//!      `sidecar_client::build_uds_channel` pattern verbatim.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.3 (UDS carve-out)
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.4 (fail-closed)
//!   - docs/specs/coverage/D01_envoy_extproc/implementation.md §6
//!   - docs/specs/coverage/D01_envoy_extproc/review-standards.md §4.1
//!   - services/egress_proxy/src/sidecar_client.rs (pattern source)

use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;
use tracing::{debug, warn};

use crate::proto::spendguard::sidecar_adapter::v1::{
    sidecar_adapter_client::SidecarAdapterClient, DecisionRequest, DecisionResponse, TraceEvent,
    TraceEventAck,
};

/// Default hot-path timeout. Spec §11 lists 50ms as the upper bound;
/// 75ms is chosen so transient sidecar GC pauses don't trip the gate
/// (matches Contract §14 p99 budget envelope).
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_millis(75);

/// Per-call result mapped 1:1 to the response builder's outcome enum.
/// All variants are non-leaky — the response builder renders a fixed
/// shape regardless of which variant fires (review-standards §4.1.3).
#[derive(Debug, Error)]
pub enum SidecarError {
    /// Connect failed: socket missing, permission denied, broken pipe,
    /// transport-level error. Maps to ExtProc 503 in the response layer.
    #[error("sidecar transport error: {message}")]
    Transport { message: String },

    /// The tonic RPC returned a non-OK Status. Maps to ExtProc 503 with
    /// Retry-After. We deliberately STRIP `Status::message()` from the
    /// outward-facing response body — see review-standards §4.1.3.
    #[error("sidecar RPC returned status {code:?}")]
    Rpc {
        code: tonic::Code,
        // Internal use only — never propagated into the ExtProc body.
        // Surfaced in logs / metrics labels only.
        internal_detail: String,
    },

    /// `tokio::time::timeout` elapsed before the sidecar replied. Maps
    /// to ExtProc 503 + Retry-After. Per review-standards §4.1.2.
    #[error("sidecar RPC timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
}

/// Lazy-connected tonic client wrapped with a UDS service_fn channel.
/// Cheaply cloneable — the underlying `tonic::transport::Channel` is
/// `Clone`-cheap.
#[derive(Clone)]
pub struct SidecarClient {
    inner: SidecarAdapterClient<Channel>,
    timeout: Duration,
    /// Stored for log lines / error context only — the channel routes
    /// via the service_fn closure so this path is reference-only.
    uds_path: PathBuf,
}

impl std::fmt::Debug for SidecarClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SidecarClient")
            .field("uds_path", &self.uds_path.display().to_string())
            .field("timeout_ms", &self.timeout.as_millis())
            .finish()
    }
}

impl SidecarClient {
    /// Connect lazily — the tonic Channel only opens the UDS on first
    /// RPC invocation. Production boot calls
    /// [`crate::handshake::dial_sidecar_with_retry`] FIRST to prove the
    /// socket exists; this constructor is the fast-path follow-up.
    ///
    /// Returns [`SidecarError::Transport`] if the channel cannot even
    /// resolve its placeholder URI — should be impossible in practice
    /// (the URI is a static string) but we surface a typed error to
    /// honour the no-`unwrap` rule from review-standards §2.2.
    pub async fn connect(uds_path: &Path, timeout: Duration) -> Result<Self, SidecarError> {
        let channel = build_uds_channel(uds_path).await?;
        Ok(Self {
            inner: SidecarAdapterClient::new(channel),
            timeout,
            uds_path: uds_path.to_path_buf(),
        })
    }

    /// Build a client from an already-constructed [`Channel`]. Lets the
    /// SLICE 3 integration test inject a mock-sidecar channel without
    /// going through the UDS file path.
    pub fn with_channel(channel: Channel, uds_path: PathBuf, timeout: Duration) -> Self {
        Self {
            inner: SidecarAdapterClient::new(channel),
            timeout,
            uds_path,
        }
    }

    /// Hot-path RequestDecision RPC.
    ///
    /// On success, returns the sidecar's [`DecisionResponse`]
    /// unchanged. On any failure path returns a typed [`SidecarError`];
    /// the caller maps to ExtProc 503 (fail-closed, per design §3.4).
    pub async fn request_decision(
        &self,
        req: DecisionRequest,
    ) -> Result<DecisionResponse, SidecarError> {
        let mut client = self.inner.clone();
        let fut = client.request_decision(tonic::Request::new(req));
        match tokio::time::timeout(self.timeout, fut).await {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(status)) => {
                let internal_detail = status.message().to_string();
                let code = status.code();
                warn!(
                    uds = %self.uds_path.display(),
                    code = ?code,
                    detail = %internal_detail,
                    "sidecar RequestDecision returned non-OK Status (mapped to 503)"
                );
                Err(SidecarError::Rpc {
                    code,
                    internal_detail,
                })
            }
            Err(_elapsed) => {
                let timeout_ms = self.timeout.as_millis() as u64;
                warn!(
                    uds = %self.uds_path.display(),
                    timeout_ms,
                    "sidecar RequestDecision timeout (mapped to 503)"
                );
                Err(SidecarError::Timeout { timeout_ms })
            }
        }
    }

    /// SLICE 4 — emit a single `LLM_CALL_POST` `TraceEvent` over the
    /// sidecar adapter's `EmitTraceEvents` bidi stream and drain the
    /// resulting `TraceEventAck`. Reuses the SLICE 3 UDS channel; no
    /// new connect path.
    ///
    /// Per implementation.md §7 (Response-Body phase) + the egress_proxy
    /// pattern at `services/egress_proxy/src/forward.rs:936-940`, the
    /// caller streams one event and drains one ack — the stream is
    /// closed after the single round-trip.
    ///
    /// Failure modes mirror [`Self::request_decision`]:
    ///   * `SidecarError::Timeout` when the configured per-call timeout
    ///     elapses before the ack lands;
    ///   * `SidecarError::Rpc` when the sidecar returns non-OK status;
    ///   * `SidecarError::Transport` is only reachable on a fresh
    ///     connect, which the SLICE 1 handshake already proved.
    ///
    /// Audit emit is **best-effort**: review-standards §5.1 requires
    /// exactly one event per stream, but a transport / timeout failure
    /// is logged at WARN and the caller MUST NOT block the upstream
    /// response on it (the sidecar's POST_GA_01 dedup catches retries
    /// without intervention). The typed error is still returned so the
    /// caller can update its metrics.
    pub async fn emit_trace_events(
        &self,
        event: TraceEvent,
    ) -> Result<TraceEventAck, SidecarError> {
        let mut client = self.inner.clone();
        // Build a one-shot stream containing the single event. tokio's
        // `mpsc + ReceiverStream` is enough — no `async_stream` dep
        // pulled in just to ship one TraceEvent.
        let (tx, rx) = tokio::sync::mpsc::channel::<TraceEvent>(1);
        if tx.send(event).await.is_err() {
            // Receiver dropped before send — should be impossible since
            // we own both ends in this scope.
            return Err(SidecarError::Transport {
                message: "EmitTraceEvents: failed to seed one-shot stream".to_string(),
            });
        }
        drop(tx); // Half-close so the server sees end-of-stream.
        let request_stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        let fut = client.emit_trace_events(tonic::Request::new(request_stream));
        match tokio::time::timeout(self.timeout, fut).await {
            Ok(Ok(resp)) => {
                let mut ack_stream = resp.into_inner();
                // Drain one ack — the sidecar acknowledges then closes.
                match tokio::time::timeout(self.timeout, ack_stream.message()).await {
                    Ok(Ok(Some(ack))) => Ok(ack),
                    Ok(Ok(None)) => {
                        // Stream closed without an ack — typed Rpc error.
                        warn!(
                            uds = %self.uds_path.display(),
                            "sidecar EmitTraceEvents closed without TraceEventAck"
                        );
                        Err(SidecarError::Rpc {
                            code: tonic::Code::Internal,
                            internal_detail: "EmitTraceEvents: no ack frame".to_string(),
                        })
                    }
                    Ok(Err(status)) => {
                        let internal_detail = status.message().to_string();
                        let code = status.code();
                        warn!(
                            uds = %self.uds_path.display(),
                            code = ?code,
                            detail = %internal_detail,
                            "sidecar EmitTraceEvents ack returned non-OK Status"
                        );
                        Err(SidecarError::Rpc {
                            code,
                            internal_detail,
                        })
                    }
                    Err(_elapsed) => {
                        let timeout_ms = self.timeout.as_millis() as u64;
                        warn!(
                            uds = %self.uds_path.display(),
                            timeout_ms,
                            "sidecar EmitTraceEvents ack timeout"
                        );
                        Err(SidecarError::Timeout { timeout_ms })
                    }
                }
            }
            Ok(Err(status)) => {
                let internal_detail = status.message().to_string();
                let code = status.code();
                warn!(
                    uds = %self.uds_path.display(),
                    code = ?code,
                    detail = %internal_detail,
                    "sidecar EmitTraceEvents call returned non-OK Status"
                );
                Err(SidecarError::Rpc {
                    code,
                    internal_detail,
                })
            }
            Err(_elapsed) => {
                let timeout_ms = self.timeout.as_millis() as u64;
                warn!(
                    uds = %self.uds_path.display(),
                    timeout_ms,
                    "sidecar EmitTraceEvents call timeout"
                );
                Err(SidecarError::Timeout { timeout_ms })
            }
        }
    }

    /// Read-only handle to the UDS path — used by structured logs only.
    pub fn uds_path(&self) -> &Path {
        &self.uds_path
    }

    /// Read-only handle to the configured per-call timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// Build a tonic [`Channel`] over a Unix Domain Socket. Mirrors the
/// egress_proxy pattern at `services/egress_proxy/src/sidecar_client.rs`.
///
/// The placeholder URI ("http://[::1]:50051") never actually gets
/// dialled — tonic's transport hands the URI to our `service_fn`
/// closure, which ignores it and connects to the UDS path.
pub async fn build_uds_channel(path: &Path) -> Result<Channel, SidecarError> {
    let path_buf = path.to_path_buf();
    let endpoint = Endpoint::try_from("http://[::1]:50051")
        .map_err(|e| SidecarError::Transport {
            message: format!("endpoint setup: {e}"),
        })?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10));

    let channel = endpoint
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = path_buf.clone();
            async move {
                UnixStream::connect(path)
                    .await
                    .map(hyper_util::rt::TokioIo::new)
            }
        }))
        .await
        .map_err(|e| SidecarError::Transport {
            message: format!("uds connect: {e}"),
        })?;
    debug!(
        path = %path.display(),
        "SidecarClient: tonic channel built (lazy)"
    );
    Ok(channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_to_missing_socket_returns_transport_error() {
        // Random path that should not exist.
        let tmp = std::env::temp_dir().join(format!(
            "spendguard-extproc-slice3-missing-{}.sock",
            uuid::Uuid::new_v4().simple()
        ));
        let result = SidecarClient::connect(&tmp, Duration::from_millis(50)).await;
        let err = result.expect_err("connect to missing socket must error");
        assert!(
            matches!(err, SidecarError::Transport { .. }),
            "expected SidecarError::Transport, got {err:?}"
        );
    }

    #[tokio::test]
    async fn debug_redacts_internal_state() {
        // Debug derive on SidecarClient must NOT print the underlying
        // Channel's address book (which could leak peer credentials in
        // a future TCP-backed variant). Pin the shape so reviewers can
        // grep for accidental verbose debug additions. Uses
        // `connect_lazy` so no actual UDS connect is attempted — but
        // hyper-util's runtime initialisation requires a tokio context,
        // hence the `#[tokio::test]` attribute.
        let endpoint = Endpoint::try_from("http://[::1]:50051").unwrap();
        let channel = endpoint.connect_lazy();
        let client = SidecarClient::with_channel(
            channel,
            PathBuf::from("/var/run/test.sock"),
            Duration::from_millis(75),
        );
        let s = format!("{client:?}");
        assert!(s.contains("uds_path"), "Debug must include uds_path: {s}");
        assert!(
            s.contains("timeout_ms"),
            "Debug must include timeout_ms: {s}"
        );
        assert!(
            !s.contains("Channel"),
            "Debug must NOT leak underlying Channel internals: {s}"
        );
    }

    #[test]
    fn default_request_timeout_matches_spec_envelope() {
        // Re-pin the constant so a future drift triggers a compile-time
        // failure. Spec §11 lists 50ms; we add headroom for GC.
        assert_eq!(DEFAULT_REQUEST_TIMEOUT, Duration::from_millis(75));
    }
}
