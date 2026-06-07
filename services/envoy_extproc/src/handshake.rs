//! Handshake phase — confirm the sidecar is reachable so the binary
//! refuses to serve traffic if it isn't (design §3.4 fail-closed).
//!
//! ## SLICE 1-5 (UDS carve-out, `uds-dev` feature)
//!   - Dial the SpendGuard sidecar adapter UDS once. Retries with the
//!     same backoff envelope as `services/egress_proxy/src/sidecar_client.rs`
//!     so containers brought up via `docker-compose depends_on: service_started`
//!     don't lose the race.
//!
//! ## SLICE 6 (mTLS-TCP hard-switch, production)
//!   - Dial the sidecar's TLS endpoint. We do not run a full mTLS
//!     handshake at this layer (that happens lazily on the first
//!     `RequestDecision`); instead the startup probe performs a TCP
//!     SYN against the parsed host:port so Kubernetes can distinguish
//!     "sidecar pod not scheduled" from "sidecar pod up but no certs".
//!     The TLS handshake check happens through the SLICE 1 protocol
//!     `Handshake` RPC on the first decision request once the TCP
//!     channel is established.
//!
//! Mirrors the dial pattern from
//! [`services/egress_proxy/src/sidecar_client.rs`].

use thiserror::Error;
use tracing::{info, warn};

use crate::config::{Config, Transport};

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
    #[error("sidecar TCP dial to {endpoint} failed after {attempts} attempts ({deadline_s}s deadline): {source}")]
    SidecarTcpUnreachable {
        endpoint: String,
        attempts: u32,
        deadline_s: u64,
        #[source]
        source: std::io::Error,
    },
    #[error("sidecar URL `{url}` is malformed: {reason}")]
    InvalidSidecarUrl { url: String, reason: String },
}

/// SLICE 6 — dial the sidecar (TCP or UDS depending on transport)
/// and retry until the configured deadline. Returns `Ok(true)` once the
/// transport-level probe succeeds; the protocol-level Handshake RPC
/// happens lazily on the first decision request (SLICE 3 wiring).
pub async fn dial_sidecar_with_retry(cfg: &Config) -> Result<bool, HandshakeError> {
    match &cfg.transport {
        Transport::Tcp { sidecar_url, .. } => dial_sidecar_tcp_with_retry(cfg, sidecar_url).await,
        #[cfg(feature = "uds-dev")]
        Transport::Uds { socket_path } => {
            uds_dev::dial_sidecar_uds_with_retry(cfg, socket_path.as_path()).await
        }
        #[cfg(not(feature = "uds-dev"))]
        Transport::Uds { .. } => {
            // Unreachable: Config::from_env rejects UDS without the feature.
            // Defense in depth — keep the binary fail-closed if a stale
            // Transport instance was hand-built (test helper drift).
            warn!("UDS transport reached production handshake path — fail-closed");
            Err(HandshakeError::SidecarTcpUnreachable {
                endpoint: "uds:disabled".into(),
                attempts: 0,
                deadline_s: cfg.sidecar_startup_deadline.as_secs(),
                source: std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "uds-dev feature disabled in this build",
                ),
            })
        }
    }
}

/// SLICE 6 — TCP host:port dial loop for the mTLS-TCP production path.
/// We pull host:port out of the configured `https://host:port` URL and
/// do a plain `tokio::net::TcpStream::connect` so the startup probe
/// distinguishes "sidecar pod not scheduled" (TCP fail) from "sidecar
/// up but no certs" (TCP ok, mTLS later).
async fn dial_sidecar_tcp_with_retry(
    cfg: &Config,
    sidecar_url: &str,
) -> Result<bool, HandshakeError> {
    let (host, port) = parse_sidecar_host_port(sidecar_url).map_err(|reason| {
        HandshakeError::InvalidSidecarUrl {
            url: sidecar_url.to_string(),
            reason,
        }
    })?;
    let endpoint = format!("{host}:{port}");
    let deadline = tokio::time::Instant::now() + cfg.sidecar_startup_deadline;
    let mut backoff = cfg.sidecar_initial_backoff;
    let mut attempts: u32 = 0;

    loop {
        attempts += 1;
        let last_err: std::io::Error = match tokio::time::timeout(
            cfg.sidecar_request_timeout,
            tokio::net::TcpStream::connect(&endpoint),
        )
        .await
        {
            Ok(Ok(_stream)) => {
                info!(
                    endpoint = %endpoint,
                    attempts,
                    "sidecar TCP reachable (SLICE 6 mTLS hard-switch; protocol Handshake deferred)"
                );
                return Ok(true);
            }
            Ok(Err(e)) => e,
            Err(_elapsed) => std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "sidecar TCP connect timed out after {}ms",
                    cfg.sidecar_request_timeout.as_millis()
                ),
            ),
        };

        if tokio::time::Instant::now() >= deadline {
            return Err(HandshakeError::SidecarTcpUnreachable {
                endpoint,
                attempts,
                deadline_s: cfg.sidecar_startup_deadline.as_secs(),
                source: last_err,
            });
        }
        warn!(
            endpoint = %endpoint,
            attempts,
            err = %last_err,
            next_backoff_ms = backoff.as_millis() as u64,
            "sidecar TCP dial failed; retrying"
        );
        tokio::time::sleep(backoff).await;
        backoff = std::cmp::min(backoff * 2, cfg.sidecar_max_backoff);
    }
}

/// Parse `https://host:port` (with default port 443 when omitted) into
/// `(host, port)`. Used by the TCP handshake probe and unit-tested
/// directly so the production grep gate audit can verify there's no
/// `URI::host().unwrap()` panic on stage.
pub(crate) fn parse_sidecar_host_port(url: &str) -> Result<(String, u16), String> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| "scheme must be https://".to_string())?;
    let host_port = rest.split('/').next().unwrap_or(rest);
    if host_port.is_empty() {
        return Err("missing host".to_string());
    }
    if let Some(idx) = host_port.rfind(':') {
        let (h, p) = host_port.split_at(idx);
        let port = p[1..]
            .parse::<u16>()
            .map_err(|e| format!("port `{}` is not a u16: {e}", &p[1..]))?;
        Ok((h.to_string(), port))
    } else {
        // Default https port. Production Helm always sets the explicit
        // port (8443) so this fallback is only reachable for hand-written
        // configs.
        Ok((host_port.to_string(), 443))
    }
}

/// SLICE 1-5 UDS carve-out helpers. Compiled in only when the `uds-dev`
/// feature is enabled — production builds (chart image) compile this
/// module out entirely so the §7.1 grep gate sees only `cfg(...)`-gated
/// lines. Spec: review-standards §7.1 (Blocker class).
#[cfg(feature = "uds-dev")]
mod uds_dev {
    use std::path::Path;
    use std::time::Duration;

    use tokio::net::UnixStream;
    use tracing::{info, warn};

    use crate::config::Config;

    use super::HandshakeError;

    pub(super) async fn dial_sidecar_uds_with_retry(
        cfg: &Config,
        path: &Path,
    ) -> Result<bool, HandshakeError> {
        let path = path.to_path_buf();
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
                        "sidecar UDS reachable (SLICE 1-5 dial-only handshake)"
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

    /// Lightweight one-shot dial without retry — useful in unit tests
    /// that want to fail fast.
    #[doc(hidden)]
    pub async fn dial_sidecar_once(path: &Path) -> std::io::Result<()> {
        tokio::time::timeout(Duration::from_millis(100), UnixStream::connect(path))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "dial timeout"))?
            .map(|_| ())
    }
}

/// Re-export the dev helper at the crate path so existing SLICE 1-5
/// integration tests (`handshake_smoke.rs`) keep compiling without
/// having to know about the feature gate.
#[cfg(feature = "uds-dev")]
pub use uds_dev::dial_sidecar_once;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn parse_sidecar_host_port_happy_path() {
        let (h, p) = parse_sidecar_host_port("https://spendguard-sidecar:8443").unwrap();
        assert_eq!(h, "spendguard-sidecar");
        assert_eq!(p, 8443);
    }

    #[test]
    fn parse_sidecar_host_port_default_port() {
        let (h, p) = parse_sidecar_host_port("https://sidecar.example.com").unwrap();
        assert_eq!(h, "sidecar.example.com");
        assert_eq!(p, 443);
    }

    #[test]
    fn parse_sidecar_host_port_rejects_http_scheme() {
        let err = parse_sidecar_host_port("http://sidecar:8443").expect_err("must reject http");
        assert!(err.contains("https"));
    }

    #[test]
    fn parse_sidecar_host_port_rejects_bad_port() {
        let err =
            parse_sidecar_host_port("https://sidecar:not-a-port").expect_err("must reject port");
        assert!(err.contains("port"));
    }

    #[cfg(feature = "uds-dev")]
    #[tokio::test]
    async fn dial_sidecar_once_returns_io_error_when_unreachable() {
        let path = std::path::PathBuf::from("/tmp/spendguard-extproc-test-nonexistent.sock");
        let _ = std::fs::remove_file(&path);
        let err = uds_dev::dial_sidecar_once(&path)
            .await
            .expect_err("must error");
        // ENOENT or ECONNREFUSED — both acceptable. The test only
        // checks the error path runs at all.
        assert!(matches!(
            err.kind(),
            std::io::ErrorKind::NotFound
                | std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::TimedOut
        ));
    }

    #[cfg(feature = "uds-dev")]
    #[tokio::test]
    async fn dial_sidecar_with_retry_uds_fails_fast_with_zero_deadline() {
        let mut cfg = Config::for_test("127.0.0.1:0".parse().unwrap());
        cfg.sidecar_startup_deadline = Duration::from_millis(10);
        cfg.sidecar_initial_backoff = Duration::from_millis(1);
        cfg.sidecar_max_backoff = Duration::from_millis(2);
        let bad_path = std::path::PathBuf::from("/tmp/spendguard-extproc-test-nonexistent.sock");
        let _ = std::fs::remove_file(&bad_path);
        cfg.transport = Transport::Uds {
            socket_path: bad_path,
        };

        let err = dial_sidecar_with_retry(&cfg)
            .await
            .expect_err("must surface SidecarUnreachable");
        assert!(matches!(err, HandshakeError::SidecarUnreachable { .. }));
    }

    /// SLICE 6 — TCP dial fails closed when the sidecar URL points at
    /// an unreachable address. We use a port we know is closed in CI.
    #[tokio::test]
    async fn dial_sidecar_with_retry_tcp_fails_fast_with_zero_deadline() {
        let mut cfg = Config::for_test("127.0.0.1:0".parse().unwrap());
        cfg.sidecar_startup_deadline = Duration::from_millis(20);
        cfg.sidecar_initial_backoff = Duration::from_millis(1);
        cfg.sidecar_max_backoff = Duration::from_millis(5);
        cfg.sidecar_request_timeout = Duration::from_millis(10);
        cfg.transport = Transport::Tcp {
            sidecar_url: "https://127.0.0.1:1".into(),
            client_cert_pem: std::path::PathBuf::from("/dev/null"),
            client_key_pem: std::path::PathBuf::from("/dev/null"),
            ca_bundle_pem: std::path::PathBuf::from("/dev/null"),
            expected_sidecar_svid_prefix: crate::config::SIDECAR_SVID_PREFIX.into(),
        };

        let err = dial_sidecar_with_retry(&cfg)
            .await
            .expect_err("must surface SidecarTcpUnreachable");
        assert!(matches!(err, HandshakeError::SidecarTcpUnreachable { .. }));
    }

    /// SLICE 6 — malformed sidecar URL surfaces typed error rather than
    /// `unwrap` panic (review-standards §2.2).
    #[tokio::test]
    async fn dial_sidecar_with_retry_tcp_rejects_bad_url() {
        let mut cfg = Config::for_test("127.0.0.1:0".parse().unwrap());
        cfg.sidecar_startup_deadline = Duration::from_millis(5);
        cfg.transport = Transport::Tcp {
            sidecar_url: "ftp://sidecar:8443".into(),
            client_cert_pem: std::path::PathBuf::from("/dev/null"),
            client_key_pem: std::path::PathBuf::from("/dev/null"),
            ca_bundle_pem: std::path::PathBuf::from("/dev/null"),
            expected_sidecar_svid_prefix: crate::config::SIDECAR_SVID_PREFIX.into(),
        };

        let err = dial_sidecar_with_retry(&cfg)
            .await
            .expect_err("malformed url must error");
        assert!(matches!(err, HandshakeError::InvalidSidecarUrl { .. }));
    }
}
