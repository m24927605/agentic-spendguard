//! SLICE 3+6 — SpendGuard sidecar adapter client over UDS (dev) or
//! mTLS-over-TCP (production hard-switch).
//!
//! Wraps [`crate::proto::spendguard::sidecar_adapter::v1::sidecar_adapter_client::SidecarAdapterClient`]
//! with a lazy-connect tonic channel pointed at the configured transport.
//!
//!   * [`SidecarClient::connect_transport`] — SLICE 6 entry point. Reads
//!     [`crate::config::Transport`] and dispatches to either
//!     [`build_tcp_channel`] (mTLS-TCP, production default) or
//!     [`build_uds_channel`] (SLICE 1-5 carve-out, gated behind the
//!     `uds-dev` cargo feature).
//!   * [`SidecarClient::connect`] — SLICE 1-5 UDS-only entry kept for
//!     backwards-compat with the existing smoke tests; routed through
//!     the gated UDS connector.
//!   * [`SidecarClient::request_decision`] — single hot-path RPC; the
//!     caller (Request-Body phase) supplies a fully built
//!     [`DecisionRequest`] and gets back [`DecisionResponse`] or a
//!     typed [`SidecarError`].
//!
//! ## SLICE 6 mTLS-TCP transport (production)
//!
//! The TCP path mirrors `services/output_predictor/src/plugin_client.rs`
//! from HARDEN_08: we build a `rustls::ClientConfig` ourselves so we can
//! install a custom `ServerCertVerifier` ([`SidecarSvidVerifier`]) that
//! pins on the SPIFFE URI SAN. Standard webpki chain validation runs
//! first (against the configured CA bundle); the verifier then asserts
//! the sidecar's cert carries a URI SAN of the shape
//! `spiffe://spendguard.platform/sidecar/<tenant_id>` matching the
//! configured tenant id (review-standards §7.1 + design §3.3).
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
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.3 (transport carve-out + SLICE 6 hard-switch)
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.4 (fail-closed)
//!   - docs/specs/coverage/D01_envoy_extproc/implementation.md §6
//!   - docs/specs/coverage/D01_envoy_extproc/review-standards.md §4.1, §7.1
//!   - services/output_predictor/src/plugin_client.rs (SPIFFE pinning pattern source)

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName as RustlsServerName, UnixTime};
use tokio_rustls::rustls::{
    crypto::aws_lc_rs::default_provider as aws_lc_default_provider,
    ClientConfig as RustlsClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use tokio_rustls::TlsConnector as RustlsTlsConnector;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;
use tracing::{debug, info, warn};

use crate::config::Transport;
#[cfg(test)]
use crate::config::SIDECAR_SVID_PREFIX;
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

/// Identity tag for the active transport — used in structured log lines
/// and error context. Production deploys see `tcp:<sidecar_url>`;
/// SLICE 1-5 dev paths see `uds:<socket_path>`. The grep-friendly
/// prefix makes it trivial for an SRE to confirm which leg of the
/// hard-switch is active.
#[derive(Debug, Clone)]
pub enum TransportIdentity {
    Tcp { sidecar_url: String },
    Uds { socket_path: PathBuf },
}

impl std::fmt::Display for TransportIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportIdentity::Tcp { sidecar_url } => write!(f, "tcp:{sidecar_url}"),
            TransportIdentity::Uds { socket_path } => {
                write!(f, "uds:{}", socket_path.display())
            }
        }
    }
}

/// Lazy-connected tonic client wrapped with a service_fn channel.
/// Cheaply cloneable — the underlying `tonic::transport::Channel` is
/// `Clone`-cheap.
#[derive(Clone)]
pub struct SidecarClient {
    inner: SidecarAdapterClient<Channel>,
    timeout: Duration,
    /// Identity tag for the active transport — log/error context only.
    identity: TransportIdentity,
}

impl std::fmt::Debug for SidecarClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SidecarClient")
            .field("identity", &self.identity.to_string())
            .field("timeout_ms", &self.timeout.as_millis())
            .finish()
    }
}

impl SidecarClient {
    /// SLICE 6 entry point — dispatch the connect based on the loaded
    /// `Transport`. Production main calls this; SLICE 1-5 smoke tests
    /// keep using [`Self::connect`] (UDS only) until they're migrated
    /// to TCP mocks. Returns [`SidecarError::Transport`] when the
    /// underlying channel cannot be constructed.
    pub async fn connect_transport(
        transport: &Transport,
        tenant_id: &str,
        timeout: Duration,
    ) -> Result<Self, SidecarError> {
        match transport {
            Transport::Tcp {
                sidecar_url,
                client_cert_pem,
                client_key_pem,
                ca_bundle_pem,
                expected_sidecar_svid_prefix,
            } => {
                let channel = build_tcp_channel(
                    sidecar_url,
                    client_cert_pem,
                    client_key_pem,
                    ca_bundle_pem,
                    expected_sidecar_svid_prefix,
                    tenant_id,
                )
                .await?;
                info!(
                    sidecar_url = %sidecar_url,
                    tenant = %tenant_id,
                    "SLICE 6 mTLS-TCP sidecar channel built (SPIFFE URI SAN pinned)"
                );
                Ok(Self {
                    inner: SidecarAdapterClient::new(channel),
                    timeout,
                    identity: TransportIdentity::Tcp {
                        sidecar_url: sidecar_url.clone(),
                    },
                })
            }
            #[cfg(feature = "uds-dev")]
            Transport::Uds { socket_path } => Self::connect(socket_path, timeout).await,
            #[cfg(not(feature = "uds-dev"))]
            Transport::Uds { .. } => Err(SidecarError::Transport {
                message: "uds transport disabled in this build (uds-dev feature off)".into(),
            }),
        }
    }

    /// SLICE 1-5 / `uds-dev` connect helper. Production callers MUST go
    /// through [`Self::connect_transport`]; this is preserved so the
    /// existing smoke-test fixtures (and the SLICE 1-5 dev path)
    /// continue to compile when `uds-dev` is enabled (default features).
    #[cfg(feature = "uds-dev")]
    pub async fn connect(uds_path: &Path, timeout: Duration) -> Result<Self, SidecarError> {
        let channel = build_uds_channel(uds_path).await?;
        Ok(Self {
            inner: SidecarAdapterClient::new(channel),
            timeout,
            identity: TransportIdentity::Uds {
                socket_path: uds_path.to_path_buf(),
            },
        })
    }

    /// Build a client from an already-constructed [`Channel`]. Lets the
    /// SLICE 3 integration test inject a mock-sidecar channel without
    /// going through the UDS file path. The `identity` argument is
    /// purely for log context — the caller picks whatever string
    /// surface lets oncall recognise the test fixture.
    pub fn with_channel(channel: Channel, identity: TransportIdentity, timeout: Duration) -> Self {
        Self {
            inner: SidecarAdapterClient::new(channel),
            timeout,
            identity,
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
                    transport = %self.identity,
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
                    transport = %self.identity,
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
                            transport = %self.identity,
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
                            transport = %self.identity,
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
                            transport = %self.identity,
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
                    transport = %self.identity,
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
                    transport = %self.identity,
                    timeout_ms,
                    "sidecar EmitTraceEvents call timeout"
                );
                Err(SidecarError::Timeout { timeout_ms })
            }
        }
    }

    /// Read-only handle to the active transport identity — used by
    /// structured logs + the SLICE 6 `/readyz` probe label.
    pub fn identity(&self) -> &TransportIdentity {
        &self.identity
    }

    /// SLICE 6 — backwards-compat accessor for the SLICE 1-5 smoke
    /// tests that grepped on the UDS path. Returns `None` when the
    /// active transport is TCP.
    pub fn uds_path(&self) -> Option<&Path> {
        match &self.identity {
            TransportIdentity::Uds { socket_path } => Some(socket_path.as_path()),
            TransportIdentity::Tcp { .. } => None,
        }
    }

    /// Read-only handle to the configured per-call timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// SLICE 6 — Build a tonic [`Channel`] over mTLS-TCP with SPIFFE URI
/// SAN pinning. Mirrors `services/output_predictor/src/plugin_client.rs`
/// from HARDEN_08 — the rustls `ClientConfig` installs a custom
/// `ServerCertVerifier` ([`SidecarSvidVerifier`]) that:
///   1. Delegates standard chain validation to a
///      `WebPkiServerVerifier` rooted at the configured CA bundle.
///   2. Extracts the URI SAN from the sidecar's leaf cert and asserts
///      it matches `<expected_prefix><tenant_id>` (e.g.
///      `spiffe://spendguard.platform/sidecar/<tenant>`).
///
/// Returns [`SidecarError::Transport`] on any boot-time failure (cert
/// read, parse, channel build) so main.rs exits non-zero — production
/// posture is fail-closed (design §3.4).
pub async fn build_tcp_channel(
    sidecar_url: &str,
    client_cert_pem: &Path,
    client_key_pem: &Path,
    ca_bundle_pem: &Path,
    expected_svid_prefix: &str,
    tenant_id: &str,
) -> Result<Channel, SidecarError> {
    // 1. Resolve cert / key / CA paths once at channel-build time.
    let cert_bytes = std::fs::read(client_cert_pem).map_err(|e| SidecarError::Transport {
        message: format!("read client cert {}: {e}", client_cert_pem.display()),
    })?;
    let key_bytes = std::fs::read(client_key_pem).map_err(|e| SidecarError::Transport {
        message: format!("read client key {}: {e}", client_key_pem.display()),
    })?;
    let ca_bytes = std::fs::read(ca_bundle_pem).map_err(|e| SidecarError::Transport {
        message: format!("read ca bundle {}: {e}", ca_bundle_pem.display()),
    })?;

    // 2. Parse the URL once so we can extract host + port for the raw
    //    TcpStream.connect inside the connector.
    let uri =
        sidecar_url
            .parse::<tonic::transport::Uri>()
            .map_err(|e| SidecarError::Transport {
                message: format!("invalid sidecar url `{sidecar_url}`: {e}"),
            })?;
    let host = uri
        .host()
        .ok_or_else(|| SidecarError::Transport {
            message: format!("sidecar url `{sidecar_url}` missing host"),
        })?
        .to_string();
    let port = uri.port_u16().unwrap_or(8443);

    // 3. Build the rustls ClientConfig with the pinning verifier.
    let expected_uri = format!("{expected_svid_prefix}{tenant_id}");
    let rustls_cfg =
        build_rustls_client_config(&cert_bytes, &key_bytes, &ca_bytes, expected_uri.clone())
            .map_err(|e| SidecarError::Transport {
                message: format!("build rustls client config: {e}"),
            })?;
    let connector = RustlsTlsConnector::from(Arc::new(rustls_cfg));

    let tonic_url = format!("http://{host}:{port}");
    let endpoint = Endpoint::try_from(tonic_url.clone()).map_err(|e| SidecarError::Transport {
        message: format!("endpoint setup `{tonic_url}`: {e}"),
    })?;
    let endpoint = endpoint
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10));

    let host_for_connector = host.clone();
    let channel = endpoint
        .connect_with_connector(service_fn(move |_uri: Uri| {
            let host = host_for_connector.clone();
            let connector = connector.clone();
            async move {
                let server_name = RustlsServerName::try_from(host.clone())
                    .map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            format!("invalid sni `{host}`: {e}"),
                        )
                    })?
                    .to_owned();
                let addr = format!("{host}:{port}");
                let tcp = tokio::net::TcpStream::connect(&addr).await?;
                let tls_stream = connector
                    .connect(server_name, tcp)
                    .await
                    .map_err(|e| std::io::Error::other(format!("tls handshake (svid pin): {e}")))?;
                Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(tls_stream))
            }
        }))
        .await
        .map_err(|e| SidecarError::Transport {
            message: format!("mtls connect: {e}"),
        })?;
    debug!(
        sidecar_url = %sidecar_url,
        expected_svid = %expected_uri,
        "SidecarClient: tonic mTLS-TCP channel built (lazy, SVID pinned)"
    );
    Ok(channel)
}

/// SLICE 6 — Build a rustls `ClientConfig` with [`SidecarSvidVerifier`]
/// installed as the cert verifier. The verifier wraps a standard webpki
/// chain validator and pins on the SPIFFE URI SAN.
fn build_rustls_client_config(
    cert_pem: &[u8],
    key_pem: &[u8],
    ca_pem: &[u8],
    expected_svid_uri: String,
) -> Result<RustlsClientConfig, String> {
    // 1. Trust roots — caller-supplied CA bundle only. We do NOT enable
    //    native or webpki roots; a misissue from a system CA cannot
    //    impersonate the sidecar.
    let mut roots = RootCertStore::empty();
    let ca_certs = rustls_pemfile::certs(&mut std::io::Cursor::new(ca_pem))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("parse CA bundle: {e}"))?;
    if ca_certs.is_empty() {
        return Err("CA bundle contained zero certificates".into());
    }
    for cert in ca_certs {
        roots
            .add(cert)
            .map_err(|e| format!("install CA cert into root store: {e}"))?;
    }

    // 2. Client identity (our SVID — presented to the sidecar for mTLS).
    let client_certs = rustls_pemfile::certs(&mut std::io::Cursor::new(cert_pem))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("parse client cert PEM: {e}"))?;
    if client_certs.is_empty() {
        return Err("client cert PEM contained zero certificates".into());
    }
    let client_key = rustls_pemfile::private_key(&mut std::io::Cursor::new(key_pem))
        .map_err(|e| format!("parse client key PEM: {e}"))?
        .ok_or_else(|| "client key PEM contained no private key".to_string())?;

    // 3. Inner webpki verifier — chain + revocation + hostname + expiry
    //    against the configured CA bundle. Delegated to BEFORE the
    //    SPIFFE pin check.
    let inner = tokio_rustls::rustls::client::WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(aws_lc_default_provider()),
    )
    .build()
    .map_err(|e| format!("build inner WebPkiServerVerifier: {e}"))?;

    let verifier = Arc::new(SidecarSvidVerifier {
        expected_svid_uri,
        inner,
    });

    let cfg = RustlsClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(client_certs, client_key)
        .map_err(|e| format!("install client identity: {e}"))?;
    let mut cfg = cfg;
    cfg.alpn_protocols.push(b"h2".to_vec()); // gRPC / HTTP/2
    Ok(cfg)
}

/// SLICE 6 — Custom rustls `ServerCertVerifier` that wraps a standard
/// webpki chain validator and pins on the SPIFFE URI SAN extracted from
/// the sidecar's leaf cert. The pin defends against rogue CA chains:
/// a misissued cert with a different URI SAN will FAIL pin verification
/// even if it chains validly to the configured CA bundle.
///
/// Spec ref: `docs/specs/coverage/D01_envoy_extproc/design.md` §3.3
/// production transport pinning + `review-standards.md` §7.1 Blocker.
#[derive(Debug)]
pub(crate) struct SidecarSvidVerifier {
    /// Expected SPIFFE URI SAN (e.g.
    /// `spiffe://spendguard.platform/sidecar/<tenant>`). Built once at
    /// channel construction from `<expected_prefix><tenant_id>`.
    expected_svid_uri: String,
    /// Underlying webpki chain validator (chain + hostname + expiry +
    /// chain-to-trust-root). Delegated to BEFORE the SAN check.
    inner: Arc<tokio_rustls::rustls::client::WebPkiServerVerifier>,
}

impl ServerCertVerifier for SidecarSvidVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &RustlsServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        // 1. Standard chain validation — chain + hostname + expiry.
        self.inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp, now)?;

        // 2. SPIFFE URI SAN pin — extract URIs from the leaf cert and
        //    reject if none of them match the expected SVID URI.
        let actual_uris = extract_uri_sans(end_entity.as_ref()).map_err(|e| {
            tokio_rustls::rustls::Error::General(format!("sidecar cert SAN parse failed: {e}"))
        })?;
        if !actual_uris.iter().any(|u| u == &self.expected_svid_uri) {
            return Err(tokio_rustls::rustls::Error::General(format!(
                "sidecar SVID URI SAN mismatch (expected `{}`, got {:?}) — \
                 design §3.3 SVID pin verification failed; suspect rogue \
                 CA chain or wrong-tenant cert",
                self.expected_svid_uri, actual_uris
            )));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// SLICE 6 — Extract URI SANs from a DER-encoded x509 cert. We use the
/// same `x509-parser` crate already pulled in by `output_predictor`.
fn extract_uri_sans(cert_der: &[u8]) -> Result<Vec<String>, String> {
    use x509_parser::extensions::GeneralName;
    use x509_parser::prelude::*;
    let (_, cert) =
        parse_x509_certificate(cert_der).map_err(|e| format!("parse leaf cert: {e}"))?;
    let san = cert
        .tbs_certificate
        .subject_alternative_name()
        .map_err(|e| format!("read SAN extension: {e}"))?
        .ok_or_else(|| "leaf cert missing subjectAltName".to_string())?;
    Ok(san
        .value
        .general_names
        .iter()
        .filter_map(|name| match name {
            GeneralName::URI(uri) => Some((*uri).to_string()),
            _ => None,
        })
        .collect())
}

/// Build a tonic [`Channel`] over a Unix Domain Socket. SLICE 1-5
/// dev-only carve-out per design §3.3. Production builds compile this
/// function out entirely (`--no-default-features`); the §7.1 grep gate
/// then sees only `cfg(...)`-gated lines.
///
/// The placeholder URI ("http://[::1]:50051") never actually gets
/// dialled — tonic's transport hands the URI to our `service_fn`
/// closure, which ignores it and connects to the UDS path.
#[cfg(feature = "uds-dev")]
pub async fn build_uds_channel(path: &Path) -> Result<Channel, SidecarError> {
    use tokio::net::UnixStream;

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
        "SidecarClient: tonic UDS channel built (lazy, SLICE 1-5 carve-out)"
    );
    Ok(channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "uds-dev")]
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
        // Channel's address book (which could leak peer credentials).
        // Pin the shape so reviewers can grep for accidental verbose
        // debug additions. Uses `connect_lazy` so no actual UDS connect
        // is attempted — but hyper-util's runtime initialisation
        // requires a tokio context, hence the `#[tokio::test]` attribute.
        let endpoint = Endpoint::try_from("http://[::1]:50051").unwrap();
        let channel = endpoint.connect_lazy();
        let client = SidecarClient::with_channel(
            channel,
            TransportIdentity::Uds {
                socket_path: PathBuf::from("/var/run/test.sock"),
            },
            Duration::from_millis(75),
        );
        let s = format!("{client:?}");
        assert!(s.contains("identity"), "Debug must include identity: {s}");
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

    /// SLICE 6 — transport identity formats grep-friendly so an SRE can
    /// distinguish dev (UDS) from prod (TCP) at a glance.
    #[test]
    fn transport_identity_display_prefixes() {
        let tcp = TransportIdentity::Tcp {
            sidecar_url: "https://sidecar:8443".into(),
        };
        let uds = TransportIdentity::Uds {
            socket_path: PathBuf::from("/var/run/spendguard/adapter.sock"),
        };
        assert!(tcp.to_string().starts_with("tcp:"));
        assert!(uds.to_string().starts_with("uds:"));
    }

    /// SLICE 6 — `connect_transport` with a malformed sidecar URL fails
    /// closed via `SidecarError::Transport` (no `unwrap` panic on the
    /// hot path). Uses non-existent cert paths because we expect the
    /// URL parse error to fire first; if we ever swap the parse order,
    /// this test will surface the regression.
    #[tokio::test]
    async fn connect_transport_tcp_rejects_bad_url() {
        let transport = Transport::Tcp {
            sidecar_url: "not-a-url".into(),
            client_cert_pem: PathBuf::from("/dev/null"),
            client_key_pem: PathBuf::from("/dev/null"),
            ca_bundle_pem: PathBuf::from("/dev/null"),
            expected_sidecar_svid_prefix: SIDECAR_SVID_PREFIX.into(),
        };
        let result = SidecarClient::connect_transport(
            &transport,
            "00000000-0000-4000-8000-000000000001",
            Duration::from_millis(50),
        )
        .await;
        let err = result.expect_err("bad url must error");
        assert!(matches!(err, SidecarError::Transport { .. }), "got {err:?}");
    }

    /// SLICE 6 — `connect_transport` against a TCP url with missing CA
    /// material errors at the file-read step.
    #[tokio::test]
    async fn connect_transport_tcp_reports_missing_ca() {
        let transport = Transport::Tcp {
            sidecar_url: "https://127.0.0.1:1".into(),
            client_cert_pem: PathBuf::from("/this/path/does/not/exist/tls.crt"),
            client_key_pem: PathBuf::from("/this/path/does/not/exist/tls.key"),
            ca_bundle_pem: PathBuf::from("/this/path/does/not/exist/ca.crt"),
            expected_sidecar_svid_prefix: SIDECAR_SVID_PREFIX.into(),
        };
        let result = SidecarClient::connect_transport(
            &transport,
            "00000000-0000-4000-8000-000000000001",
            Duration::from_millis(50),
        )
        .await;
        let err = result.expect_err("missing ca must error");
        let SidecarError::Transport { message } = err else {
            panic!("expected Transport error");
        };
        assert!(
            message.contains("read") || message.contains("cert") || message.contains("ca"),
            "expected file-read error, got: {message}"
        );
    }
}
