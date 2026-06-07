//! SLICE 7 (COV_11) — Local proxy reachability probe.
//!
//! Issues a TCP connect to the resolved `host:port` (default
//! `localhost:8443`) with a 5-second deadline. Surfaces three outcomes:
//!
//! - [`ProxyCheckResult::Reachable`] — TCP handshake completed.
//! - [`ProxyCheckResult::ProxyUnreachable`] — connect failed (connection
//!   refused, timeout, DNS error).
//! - [`ProxyCheckResult::TlsHandshakeFailed`] — reserved for a future
//!   slice. Per deviation #4 in `Cargo.toml`, SLICE 7 ships TCP-only;
//!   the TLS handshake leg lives in a follow-up because the rustls
//!   client config plumbing would expand the surface area beyond the
//!   slice doc's "≥18 unit + 3 lib-level integration" budget.
//!
//! ## Anti-scope
//!
//! - No remote-host reachability checks per slice doc anti-scope. We
//!   resolve `host` as a literal (loopback / 127.0.0.1 / ::1) and skip
//!   any external DNS that doesn't resolve via the local resolver — the
//!   probe is only useful for the local proxy.
//!
//! ## Test injection
//!
//! The [`TcpProbe`] handle lets unit tests inject canned outcomes
//! without opening real sockets. Production callers use
//! [`TcpProbe::real`] which delegates to `TcpStream::connect_timeout`.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Outcome of the proxy probe.
#[derive(Debug, Clone)]
pub enum ProxyCheckResult {
    /// TCP handshake completed against `addr` within the timeout.
    Reachable { addr: String },
    /// Connect failed — connection refused, timeout, or DNS error.
    ProxyUnreachable { addr: String, error: String },
    /// Reserved for a future slice. Today: never produced by [`check`]
    /// because SLICE 7 only does TCP. Variant exists so the renderer
    /// shape is forward-compat.
    TlsHandshakeFailed { addr: String, error: String },
}

impl ProxyCheckResult {
    /// One-line render.
    pub fn render(&self, use_color: bool) -> String {
        use crate::doctor::{paint, Color};
        match self {
            Self::Reachable { addr } => {
                let head = paint("OK", Color::Green, use_color);
                format!("{head} TCP to {addr} (5s deadline)")
            }
            Self::ProxyUnreachable { addr, error } => {
                let head = paint("FAIL", Color::Red, use_color);
                format!(
                    "{head} cannot reach {addr}: {error} \
                     (is the SpendGuard proxy running on port 8443?)"
                )
            }
            Self::TlsHandshakeFailed { addr, error } => {
                let head = paint("FAIL", Color::Red, use_color);
                format!("{head} TLS handshake to {addr} failed: {error}")
            }
        }
    }
}

/// Probe handle. Production callers use [`TcpProbe::real`]; unit tests
/// inject a fixed-result probe via [`TcpProbe::always_reachable`] /
/// [`TcpProbe::always_unreachable`] so they never open a real socket.
#[derive(Clone, Copy)]
pub struct TcpProbe {
    inner: TcpProbeInner,
}

#[derive(Clone, Copy)]
enum TcpProbeInner {
    Real,
    AlwaysReachable,
    AlwaysUnreachable,
}

impl std::fmt::Debug for TcpProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.inner {
            TcpProbeInner::Real => f.write_str("TcpProbe::Real"),
            TcpProbeInner::AlwaysReachable => f.write_str("TcpProbe::AlwaysReachable"),
            TcpProbeInner::AlwaysUnreachable => f.write_str("TcpProbe::AlwaysUnreachable"),
        }
    }
}

impl TcpProbe {
    /// Real probe — calls [`TcpStream::connect_timeout`].
    pub fn real() -> Self {
        Self {
            inner: TcpProbeInner::Real,
        }
    }
    /// Test probe that always reports the TCP handshake as completed.
    pub fn always_reachable() -> Self {
        Self {
            inner: TcpProbeInner::AlwaysReachable,
        }
    }
    /// Test probe that always reports the TCP handshake as failing with
    /// a stub error message ("test: unreachable").
    pub fn always_unreachable() -> Self {
        Self {
            inner: TcpProbeInner::AlwaysUnreachable,
        }
    }

    fn probe(&self, addr: &SocketAddr, timeout: Duration) -> std::result::Result<(), String> {
        match self.inner {
            TcpProbeInner::Real => TcpStream::connect_timeout(addr, timeout)
                .map(|_stream| ())
                .map_err(|e| e.to_string()),
            TcpProbeInner::AlwaysReachable => Ok(()),
            TcpProbeInner::AlwaysUnreachable => Err("test: unreachable".to_string()),
        }
    }
}

/// Probe the proxy. `proxy_url` is parsed for `host:port`; falls back
/// to port 8443 if absent. `timeout` is shared between DNS resolve +
/// TCP connect (a real `connect_timeout` deadline doesn't include DNS,
/// so we wrap the resolve too).
///
/// `tcp_probe=None` → use the real socket; tests pass `Some(stub)`.
pub fn check(proxy_url: &str, timeout: Duration, tcp_probe: Option<TcpProbe>) -> ProxyCheckResult {
    let probe = tcp_probe.unwrap_or_else(TcpProbe::real);
    let (host, port) = match parse_proxy_url(proxy_url) {
        Ok(hp) => hp,
        Err(e) => {
            return ProxyCheckResult::ProxyUnreachable {
                addr: proxy_url.to_string(),
                error: e,
            };
        }
    };
    let addr_str = format!("{host}:{port}");
    // DNS resolve via std — note we only honour the local resolver
    // (no /etc/hosts override outside what the OS already reads).
    let socket_addr = match (host.as_str(), port).to_socket_addrs() {
        Ok(mut iter) => match iter.next() {
            Some(a) => a,
            None => {
                return ProxyCheckResult::ProxyUnreachable {
                    addr: addr_str,
                    error: "DNS resolved no addresses".to_string(),
                };
            }
        },
        Err(e) => {
            return ProxyCheckResult::ProxyUnreachable {
                addr: addr_str,
                error: format!("DNS resolve failed: {e}"),
            };
        }
    };

    match probe.probe(&socket_addr, timeout) {
        Ok(()) => ProxyCheckResult::Reachable { addr: addr_str },
        Err(e) => ProxyCheckResult::ProxyUnreachable {
            addr: addr_str,
            error: e,
        },
    }
}

/// Parse `https://host:port[/path]` or `host:port` into `(host, port)`.
///
/// We do this by hand rather than pulling `url` into the workspace —
/// the slice doc only needs `host:port` extraction and the URL shape
/// is locked at `https://localhost:8443` per design §3.
fn parse_proxy_url(input: &str) -> std::result::Result<(String, u16), String> {
    // Strip scheme.
    let without_scheme = input
        .strip_prefix("https://")
        .or_else(|| input.strip_prefix("http://"))
        .unwrap_or(input);
    // Take the authority component up to the first '/'.
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    if authority.is_empty() {
        return Err(format!("proxy URL has no host: {input}"));
    }
    // Split on the LAST ':' to handle bare IPv6 (`[::1]:8443`) and
    // `host:port` symmetrically.
    let (host, port_str) = if let Some(rest) = authority.strip_prefix('[') {
        // IPv6 literal — take everything up to `]`.
        let close = rest.find(']').ok_or_else(|| {
            format!("malformed IPv6 authority — missing closing bracket: {input}")
        })?;
        let host = &rest[..close];
        let after = &rest[close + 1..];
        let port_str = after.strip_prefix(':').unwrap_or("");
        (host.to_string(), port_str)
    } else if let Some(colon_idx) = authority.rfind(':') {
        let (h, p) = authority.split_at(colon_idx);
        (h.to_string(), &p[1..])
    } else {
        (authority.to_string(), "")
    };
    let port = if port_str.is_empty() {
        8443
    } else {
        port_str
            .parse::<u16>()
            .map_err(|e| format!("invalid port `{port_str}`: {e}"))?
    };
    Ok((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `parse_proxy_url` handles the locked `https://localhost:8443`
    /// format.
    #[test]
    fn parse_proxy_url_handles_locked_default() {
        let (host, port) = parse_proxy_url("https://localhost:8443").expect("parse");
        assert_eq!(host, "localhost");
        assert_eq!(port, 8443);
    }

    /// `parse_proxy_url` strips trailing path components.
    #[test]
    fn parse_proxy_url_strips_trailing_path() {
        let (host, port) = parse_proxy_url("https://localhost:8443/v1/messages").expect("parse");
        assert_eq!(host, "localhost");
        assert_eq!(port, 8443);
    }

    /// `parse_proxy_url` falls back to port 8443 when none supplied.
    #[test]
    fn parse_proxy_url_defaults_port_to_8443() {
        let (host, port) = parse_proxy_url("https://localhost").expect("parse");
        assert_eq!(host, "localhost");
        assert_eq!(port, 8443);
    }

    /// `parse_proxy_url` rejects garbage port strings.
    #[test]
    fn parse_proxy_url_errors_on_invalid_port() {
        let err = parse_proxy_url("https://localhost:not-a-port").expect_err("must reject");
        assert!(err.contains("invalid port"));
    }

    /// `parse_proxy_url` handles the IPv6 bracketed form.
    #[test]
    fn parse_proxy_url_handles_ipv6_bracketed() {
        let (host, port) = parse_proxy_url("https://[::1]:8443").expect("parse");
        assert_eq!(host, "::1");
        assert_eq!(port, 8443);
    }

    /// `check` with the always-reachable stub returns Reachable.
    #[test]
    fn check_with_always_reachable_probe_returns_reachable() {
        let res = check(
            "https://127.0.0.1:8443",
            Duration::from_secs(5),
            Some(TcpProbe::always_reachable()),
        );
        match res {
            ProxyCheckResult::Reachable { addr } => assert_eq!(addr, "127.0.0.1:8443"),
            other => panic!("expected Reachable, got {other:?}"),
        }
    }

    /// `check` with the always-unreachable stub returns ProxyUnreachable
    /// carrying the stub error message.
    #[test]
    fn check_with_always_unreachable_probe_returns_proxy_unreachable() {
        let res = check(
            "https://127.0.0.1:8443",
            Duration::from_secs(5),
            Some(TcpProbe::always_unreachable()),
        );
        match res {
            ProxyCheckResult::ProxyUnreachable { addr, error } => {
                assert_eq!(addr, "127.0.0.1:8443");
                assert_eq!(error, "test: unreachable");
            }
            other => panic!("expected ProxyUnreachable, got {other:?}"),
        }
    }

    /// `check` returns ProxyUnreachable when DNS resolves zero hits.
    /// We trigger this by supplying an empty host (URL parse short-
    /// circuits this — the error path is "proxy URL has no host").
    #[test]
    fn check_with_empty_host_returns_proxy_unreachable() {
        let res = check(
            "https://",
            Duration::from_secs(1),
            Some(TcpProbe::always_unreachable()),
        );
        match res {
            ProxyCheckResult::ProxyUnreachable { error, .. } => {
                assert!(error.contains("no host"));
            }
            other => panic!("expected ProxyUnreachable, got {other:?}"),
        }
    }

    /// Renderer emits the colour-coded status tokens.
    #[test]
    fn render_emits_expected_status_tokens() {
        let reachable = ProxyCheckResult::Reachable {
            addr: "127.0.0.1:8443".into(),
        };
        assert!(reachable.render(false).contains("OK"));
        let unreachable = ProxyCheckResult::ProxyUnreachable {
            addr: "127.0.0.1:8443".into(),
            error: "connection refused".into(),
        };
        let plain = unreachable.render(false);
        assert!(plain.contains("FAIL"));
        assert!(plain.contains("connection refused"));
        let tls = ProxyCheckResult::TlsHandshakeFailed {
            addr: "127.0.0.1:8443".into(),
            error: "untrusted root".into(),
        };
        assert!(tls.render(false).contains("FAIL"));
    }
}
