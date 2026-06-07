//! Configuration surface — env-driven, fail-closed.
//!
//! Spec ref: docs/specs/coverage/D01_envoy_extproc/implementation.md §11.
//!
//! ## SLICE 6 transport hard-switch
//!
//! `SPENDGUARD_EXTPROC_TRANSPORT` selects between [`Transport::Tcp`] (mTLS
//! over TCP, production default) and [`Transport::Uds`] (SLICE 1-5
//! carve-out for local-dev / docker-compose ergonomic — same-pod or
//! same-node sidecar layout only).
//!
//! Per design §3.3 production deployment shape LOCKED at SLICE 6:
//! - Default = `tcp` when the env var is unset (production posture).
//! - `uds` is honoured only when the binary was built with the
//!   `uds-dev` cargo feature (default for `cargo build`, OFF for the
//!   chart-published release image which sets `--no-default-features`).
//!   A production binary that receives `SPENDGUARD_EXTPROC_TRANSPORT=uds`
//!   fails [`ConfigError::UdsTransportDisabled`] at startup — defense
//!   in depth against an operator mis-flipping the env in production.
//! - The mTLS-TCP path reads PEM material from
//!   `SPENDGUARD_EXTPROC_CLIENT_CERT_PATH` / `_CLIENT_KEY_PATH` /
//!   `_CA_BUNDLE_PATH` (implementation §11). The expected sidecar
//!   SPIFFE URI prefix `spiffe://spendguard.platform/sidecar/<tenant>`
//!   is pinned by the rustls verifier in `sidecar_client.rs`.
//!
//! Round 1 review standards §2.2 require typed errors (not `unwrap`).
//!
//! Fail-closed posture (matches GA_03): `SPENDGUARD_EXTPROC_TENANT_ID`
//! is REQUIRED in production. The only way to fall back to the demo
//! tenant id is to opt-in via `SPENDGUARD_EXTPROC_DEV_MODE=1`, matching
//! the HARDEN_05 pattern used by the rest of the platform.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:9443";
const DEFAULT_SIDECAR_UDS: &str = "/var/run/spendguard/adapter.sock";
const DEFAULT_SIDECAR_TCP_URL: &str = "https://spendguard-sidecar:8443";
const DEFAULT_READYZ_ADDR: &str = "0.0.0.0:9090";
const DEFAULT_CA_BUNDLE_PATH: &str = "/run/secrets/svid/ca.crt";
const DEFAULT_CLIENT_CERT_PATH: &str = "/run/secrets/svid/tls.crt";
const DEFAULT_CLIENT_KEY_PATH: &str = "/run/secrets/svid/tls.key";
/// SLICE 6 — SPIFFE prefix the sidecar's server cert URI SAN MUST match.
/// Mirrors `services/output_predictor/src/plugin_svid.rs` PREDICTOR_CLIENT_SVID_PREFIX
/// pattern from HARDEN_08, but on the *sidecar* path component.
pub const SIDECAR_SVID_PREFIX: &str = "spiffe://spendguard.platform/sidecar/";
/// Demo-only tenant id. Used by [`Config::for_test`] and the
/// `SPENDGUARD_EXTPROC_DEV_MODE=1` fallback. Production deployments MUST
/// set `SPENDGUARD_EXTPROC_TENANT_ID` or `Config::from_env` fails closed.
const DEFAULT_TENANT_ID: &str = "00000000-0000-4000-8000-000000000001";
/// Default hot-path sidecar RequestDecision RPC timeout in ms. Spec §11
/// lists 50ms as the upper bound; 75ms is chosen so transient sidecar GC
/// pauses don't trip the gate (matches Contract §14 p99 budget envelope).
/// Mirrors [`crate::sidecar_client::DEFAULT_REQUEST_TIMEOUT`] — kept in
/// sync so non-Config callers (tests / smoke) still get the same default.
const DEFAULT_SIDECAR_REQUEST_TIMEOUT_MS: u64 = 75;

/// Sidecar adapter transport — design §3.3 carve-out.
///
/// SLICE 1-5 carve-out: `Uds` was the only mode. SLICE 6 hard-switches
/// production to `Tcp` (mTLS-over-TCP); UDS stays available behind the
/// `uds-dev` cargo feature for local-dev / docker-compose ergonomic.
///
/// The variant is chosen by the `SPENDGUARD_EXTPROC_TRANSPORT` env var
/// (default `tcp`). Production Helm wiring (SLICE 6) sets `tcp`
/// explicitly and ships a binary built `--no-default-features` so
/// even an operator typo that flips the env to `uds` fails closed.
#[derive(Debug, Clone)]
pub enum Transport {
    /// mTLS over TCP. Production default. Carries the SpendGuard sidecar
    /// mTLS URL (https://host:port) and the file paths for the SVID
    /// client cert + key + CA trust anchor.
    Tcp {
        /// `https://<sidecar>:<port>` URL.
        sidecar_url: String,
        client_cert_pem: PathBuf,
        client_key_pem: PathBuf,
        ca_bundle_pem: PathBuf,
        /// Expected SPIFFE URI SAN prefix the sidecar's server cert
        /// must carry. The configured `tenant_id` is appended at
        /// verification time (e.g.
        /// `spiffe://spendguard.platform/sidecar/<tenant>`). This is
        /// the SLICE 6 production transport pin.
        expected_sidecar_svid_prefix: String,
    },
    /// Unix Domain Socket — SLICE 1-5 carve-out. Only available when
    /// the binary was built with the `uds-dev` cargo feature.
    Uds { socket_path: PathBuf },
}

impl Transport {
    /// SLICE 6 — returns true iff this variant represents the production
    /// mTLS-TCP transport. Used by the `/readyz` probe + structured logs.
    pub fn is_tcp(&self) -> bool {
        matches!(self, Transport::Tcp { .. })
    }
}

/// Loaded configuration for one envoy_extproc process.
#[derive(Debug, Clone)]
pub struct Config {
    /// gRPC bind address. Default `127.0.0.1:9443` to match the
    /// implementation §11 table.
    pub bind_addr: SocketAddr,
    /// SLICE 6 — sidecar transport. Either mTLS-TCP (production) or
    /// UDS (SLICE 1-5 carve-out, `uds-dev` feature only).
    pub transport: Transport,
    /// Tenant id assertion forwarded in the sidecar Handshake. POC
    /// default value is a v4-shaped UUID for the demo flow; production
    /// deployments MUST set `SPENDGUARD_EXTPROC_TENANT_ID`.
    pub tenant_id: String,
    /// Stable per-process workload id; default is a v4 UUID prefix so
    /// crash-restarts can be correlated in audit logs.
    pub workload_instance_id: String,
    /// SLICE 6 — `/readyz` + `/livez` HTTP probe listener (Kubernetes
    /// readinessProbe + livenessProbe). Default `0.0.0.0:9090`. Probe
    /// returns 200 once the sidecar handshake has succeeded; 503
    /// otherwise (fail-closed posture).
    pub readyz_addr: SocketAddr,
    /// Total retry deadline for sidecar handshake. Matches the
    /// egress_proxy default (30s) so docker-compose `depends_on:
    /// service_started` races don't crash the binary.
    pub sidecar_startup_deadline: Duration,
    pub sidecar_initial_backoff: Duration,
    pub sidecar_max_backoff: Duration,
    /// Hot-path RequestDecision RPC timeout. Driven by env var
    /// `SPENDGUARD_EXTPROC_REQUEST_TIMEOUT_MS` (implementation.md §11);
    /// default 75 ms — see [`DEFAULT_SIDECAR_REQUEST_TIMEOUT_MS`] for the
    /// rationale on the 50 ms spec envelope vs 75 ms GC-headroom choice
    /// (review-standards §4.1.2).
    pub sidecar_request_timeout: Duration,
}

impl Config {
    /// SLICE 6 — backwards-compat shim for SLICE 1-5 callers that still
    /// read `cfg.sidecar_uds_path`. Returns the path only when the
    /// active transport is UDS; production callers MUST switch to the
    /// `cfg.transport` variant directly.
    ///
    /// Test code uses this; production main.rs uses `cfg.transport`.
    pub fn sidecar_uds_path(&self) -> Option<&std::path::Path> {
        match &self.transport {
            Transport::Uds { socket_path } => Some(socket_path.as_path()),
            Transport::Tcp { .. } => None,
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("SPENDGUARD_EXTPROC_BIND_ADDR `{value}` failed to parse: {source}")]
    InvalidBindAddr {
        value: String,
        #[source]
        source: std::net::AddrParseError,
    },
    #[error("SPENDGUARD_EXTPROC_READYZ_ADDR `{value}` failed to parse: {source}")]
    InvalidReadyzAddr {
        value: String,
        #[source]
        source: std::net::AddrParseError,
    },
    #[error("SPENDGUARD_EXTPROC_TENANT_ID `{0}` is not a valid UUID")]
    InvalidTenantId(String),
    #[error(
        "SPENDGUARD_EXTPROC_TENANT_ID is required (set to the UUID of the tenant this gateway processes); \
         set SPENDGUARD_EXTPROC_DEV_MODE=1 to opt-in to the demo tenant fallback"
    )]
    MissingTenantId,
    #[error(
        "SPENDGUARD_EXTPROC_TRANSPORT=`{0}` is invalid (expected `tcp` or `uds`); \
         production Helm sets `tcp` per design §3.3"
    )]
    InvalidTransport(String),
    #[error(
        "SPENDGUARD_EXTPROC_TRANSPORT=uds rejected: this binary was built `--no-default-features` \
         (production posture, design §3.3 + review-standards §7.1). Rebuild with `--features uds-dev` \
         for the SLICE 1-5 carve-out or set SPENDGUARD_EXTPROC_TRANSPORT=tcp."
    )]
    UdsTransportDisabled,
    #[error(
        "SPENDGUARD_EXTPROC_SIDECAR_URL is required under SPENDGUARD_EXTPROC_TRANSPORT=tcp \
         (no production default — Helm injects `https://<release>-spendguard-sidecar:8443`)"
    )]
    MissingSidecarUrl,
    #[error(
        "SPENDGUARD_EXTPROC_SIDECAR_URL=`{0}` must be https:// (mTLS over TCP per design §3.3); \
         plaintext http:// disallowed"
    )]
    InvalidSidecarUrl(String),
}

impl Config {
    /// Read config from env. Production posture (GA_03):
    /// `SPENDGUARD_EXTPROC_TENANT_ID` is REQUIRED — missing returns
    /// [`ConfigError::MissingTenantId`] and the process exits non-zero.
    ///
    /// Setting `SPENDGUARD_EXTPROC_DEV_MODE=1` opts into the demo
    /// tenant fallback (matches HARDEN_05 pattern). The dev-mode
    /// branch is intended for `make demo-up`, docker-compose smoke,
    /// and SLICE 1 cargo-test boots — production Helm wiring (SLICE 6)
    /// MUST NOT set `SPENDGUARD_EXTPROC_DEV_MODE`.
    pub fn from_env() -> Result<Self, ConfigError> {
        let bind_addr_raw = std::env::var("SPENDGUARD_EXTPROC_BIND_ADDR")
            .unwrap_or_else(|_| DEFAULT_BIND_ADDR.into());
        let bind_addr = bind_addr_raw
            .parse()
            .map_err(|source| ConfigError::InvalidBindAddr {
                value: bind_addr_raw.clone(),
                source,
            })?;

        let readyz_addr_raw = std::env::var("SPENDGUARD_EXTPROC_READYZ_ADDR")
            .unwrap_or_else(|_| DEFAULT_READYZ_ADDR.into());
        let readyz_addr =
            readyz_addr_raw
                .parse()
                .map_err(|source| ConfigError::InvalidReadyzAddr {
                    value: readyz_addr_raw.clone(),
                    source,
                })?;

        let dev_mode = matches!(
            std::env::var("SPENDGUARD_EXTPROC_DEV_MODE").as_deref(),
            Ok("1" | "true" | "TRUE")
        );
        let tenant_id = match std::env::var("SPENDGUARD_EXTPROC_TENANT_ID") {
            Ok(v) => v,
            Err(_) if dev_mode => DEFAULT_TENANT_ID.into(),
            Err(_) => return Err(ConfigError::MissingTenantId),
        };
        if uuid::Uuid::parse_str(&tenant_id).is_err() {
            return Err(ConfigError::InvalidTenantId(tenant_id));
        }

        // SLICE 6 — production transport is mTLS-TCP per design §3.3.
        // The env var defaults to `tcp`; UDS is honoured only when the
        // binary was built with the `uds-dev` cargo feature (default for
        // cargo build + cargo test; OFF for the chart-published release
        // image). A production binary that receives `uds` from a
        // misconfigured env fails closed via `UdsTransportDisabled`.
        let transport_raw =
            std::env::var("SPENDGUARD_EXTPROC_TRANSPORT").unwrap_or_else(|_| "tcp".into());
        let transport = parse_transport_env(&transport_raw, dev_mode)?;

        let workload_instance_id = std::env::var("SPENDGUARD_EXTPROC_WORKLOAD_INSTANCE_ID")
            .unwrap_or_else(|_| format!("envoy-extproc-{}", uuid::Uuid::new_v4().simple()));

        let sidecar_request_timeout = Duration::from_millis(parse_request_timeout_ms(
            std::env::var("SPENDGUARD_EXTPROC_REQUEST_TIMEOUT_MS")
                .ok()
                .as_deref(),
        ));

        Ok(Self {
            bind_addr,
            transport,
            tenant_id,
            workload_instance_id,
            readyz_addr,
            sidecar_startup_deadline: Duration::from_secs(30),
            sidecar_initial_backoff: Duration::from_millis(250),
            sidecar_max_backoff: Duration::from_secs(4),
            sidecar_request_timeout,
        })
    }

    /// Test-only constructor — preserves the SLICE 1-5 UDS default so
    /// existing smoke-test fixtures keep dialling the tempdir socket.
    #[doc(hidden)]
    pub fn for_test(bind_addr: SocketAddr) -> Self {
        Self {
            bind_addr,
            transport: Transport::Uds {
                socket_path: DEFAULT_SIDECAR_UDS.into(),
            },
            tenant_id: DEFAULT_TENANT_ID.into(),
            workload_instance_id: "envoy-extproc-test".into(),
            readyz_addr: DEFAULT_READYZ_ADDR.parse().expect("default readyz addr"),
            sidecar_startup_deadline: Duration::from_secs(1),
            sidecar_initial_backoff: Duration::from_millis(50),
            sidecar_max_backoff: Duration::from_millis(200),
            sidecar_request_timeout: Duration::from_millis(DEFAULT_SIDECAR_REQUEST_TIMEOUT_MS),
        }
    }
}

/// SLICE 6 — pure helper for `SPENDGUARD_EXTPROC_TRANSPORT` parsing.
/// Production posture (design §3.3): default = `tcp`; `uds` is only
/// honoured when the binary was built with the `uds-dev` cargo
/// feature (the production chart image sets `--no-default-features`).
fn parse_transport_env(raw: &str, dev_mode: bool) -> Result<Transport, ConfigError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "tcp" => Ok(build_tcp_transport_from_env(dev_mode)?),
        "uds" => build_uds_transport_from_env(),
        other => Err(ConfigError::InvalidTransport(other.to_string())),
    }
}

/// Build a [`Transport::Tcp`] from env. Sidecar URL is required (no
/// production-safe default — Helm injects the in-cluster ClusterIP DNS
/// name); cert paths default to the SVID Secret mount point pattern
/// used by the rest of the platform.
fn build_tcp_transport_from_env(dev_mode: bool) -> Result<Transport, ConfigError> {
    // Dev mode loosens the URL requirement so `make demo-up` and
    // local cargo runs don't trip the missing-URL gate. Production
    // Helm wiring sets the env explicitly so this fallback never
    // applies in production.
    let sidecar_url = match std::env::var("SPENDGUARD_EXTPROC_SIDECAR_URL") {
        Ok(v) if !v.is_empty() => v,
        _ if dev_mode => DEFAULT_SIDECAR_TCP_URL.into(),
        _ => return Err(ConfigError::MissingSidecarUrl),
    };
    if !sidecar_url.starts_with("https://") {
        return Err(ConfigError::InvalidSidecarUrl(sidecar_url));
    }

    let client_cert_pem = std::env::var("SPENDGUARD_EXTPROC_CLIENT_CERT_PATH")
        .unwrap_or_else(|_| DEFAULT_CLIENT_CERT_PATH.into())
        .into();
    let client_key_pem = std::env::var("SPENDGUARD_EXTPROC_CLIENT_KEY_PATH")
        .unwrap_or_else(|_| DEFAULT_CLIENT_KEY_PATH.into())
        .into();
    let ca_bundle_pem = std::env::var("SPENDGUARD_EXTPROC_CA_BUNDLE_PATH")
        .unwrap_or_else(|_| DEFAULT_CA_BUNDLE_PATH.into())
        .into();
    let expected_sidecar_svid_prefix = std::env::var("SPENDGUARD_EXTPROC_SIDECAR_SVID_PREFIX")
        .unwrap_or_else(|_| SIDECAR_SVID_PREFIX.into());

    Ok(Transport::Tcp {
        sidecar_url,
        client_cert_pem,
        client_key_pem,
        ca_bundle_pem,
        expected_sidecar_svid_prefix,
    })
}

/// Build a [`Transport::Uds`] from env. Only reachable when the
/// `uds-dev` cargo feature is enabled at compile time. Production
/// binary returns [`ConfigError::UdsTransportDisabled`].
#[cfg(feature = "uds-dev")]
fn build_uds_transport_from_env() -> Result<Transport, ConfigError> {
    let socket_path = std::env::var("SPENDGUARD_EXTPROC_SIDECAR_UDS_PATH")
        .unwrap_or_else(|_| DEFAULT_SIDECAR_UDS.into())
        .into();
    Ok(Transport::Uds { socket_path })
}

#[cfg(not(feature = "uds-dev"))]
fn build_uds_transport_from_env() -> Result<Transport, ConfigError> {
    Err(ConfigError::UdsTransportDisabled)
}

/// Pure helper for `SPENDGUARD_EXTPROC_REQUEST_TIMEOUT_MS` parsing.
/// Unit-tested directly so we don't have to mutate process env.
///
/// Invalid / unparseable / unset → default 75 ms. We deliberately do NOT
/// fail the boot on a parse error here: the field has a safe default that
/// keeps the gateway fail-closed-on-timeout posture from
/// review-standards §4.1.2. An ops typo would still produce a working
/// gateway; the log line in `from_env` makes the parse failure visible.
fn parse_request_timeout_ms(raw: Option<&str>) -> u64 {
    raw.and_then(|v| v.parse::<u64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_SIDECAR_REQUEST_TIMEOUT_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The config tests mutate process env. Cargo runs unit tests in
    /// parallel within a binary so we serialise env-mutating tests via
    /// a module-level Mutex. Mirrors the pattern used in
    /// services/egress_proxy/src/sidecar_client.rs.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn save_env(keys: &[&str]) -> Vec<(String, Option<String>)> {
        keys.iter()
            .map(|k| (k.to_string(), std::env::var(k).ok()))
            .collect()
    }

    fn restore_env(saved: Vec<(String, Option<String>)>) {
        for (k, v) in saved {
            match v {
                Some(v) => std::env::set_var(&k, v),
                None => std::env::remove_var(&k),
            }
        }
    }

    /// SLICE 6 helper: env keys touched by the transport switch tests.
    /// Snapshotting / restoring these around a single test keeps the
    /// parallel cargo test runner from poisoning sibling tests.
    const TRANSPORT_ENV_KEYS: &[&str] = &[
        "SPENDGUARD_EXTPROC_BIND_ADDR",
        "SPENDGUARD_EXTPROC_TENANT_ID",
        "SPENDGUARD_EXTPROC_SIDECAR_UDS_PATH",
        "SPENDGUARD_EXTPROC_WORKLOAD_INSTANCE_ID",
        "SPENDGUARD_EXTPROC_DEV_MODE",
        "SPENDGUARD_EXTPROC_TRANSPORT",
        "SPENDGUARD_EXTPROC_SIDECAR_URL",
        "SPENDGUARD_EXTPROC_CLIENT_CERT_PATH",
        "SPENDGUARD_EXTPROC_CLIENT_KEY_PATH",
        "SPENDGUARD_EXTPROC_CA_BUNDLE_PATH",
        "SPENDGUARD_EXTPROC_READYZ_ADDR",
        "SPENDGUARD_EXTPROC_SIDECAR_SVID_PREFIX",
    ];

    fn clear_transport_env() {
        for key in TRANSPORT_ENV_KEYS {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn missing_tenant_id_fails_in_prod() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(TRANSPORT_ENV_KEYS);
        clear_transport_env();

        // Production posture: missing tenant id must fail closed.
        let err = Config::from_env().expect_err("missing tenant must fail closed");
        assert!(matches!(err, ConfigError::MissingTenantId), "got: {err}");

        restore_env(saved);
    }

    #[test]
    fn missing_tenant_id_uses_demo_in_dev_mode() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(TRANSPORT_ENV_KEYS);
        clear_transport_env();
        std::env::set_var("SPENDGUARD_EXTPROC_DEV_MODE", "1");

        let cfg = Config::from_env().expect("dev mode falls back to demo tenant");
        assert_eq!(cfg.bind_addr.to_string(), "127.0.0.1:9443");
        assert_eq!(cfg.tenant_id, DEFAULT_TENANT_ID);
        // SLICE 6 production default: TCP, even without SIDECAR_URL set
        // because dev_mode applies the default sidecar URL fallback.
        assert!(cfg.transport.is_tcp(), "default transport must be TCP");

        restore_env(saved);
    }

    #[test]
    fn invalid_tenant_id_returns_typed_error() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(TRANSPORT_ENV_KEYS);
        clear_transport_env();
        std::env::set_var("SPENDGUARD_EXTPROC_TENANT_ID", "not-a-uuid");
        // Enable dev_mode so the missing-sidecar-url path doesn't fire
        // first and mask the tenant-id error we want to surface.
        std::env::set_var("SPENDGUARD_EXTPROC_DEV_MODE", "1");

        let err = Config::from_env().expect_err("invalid uuid must error");
        assert!(matches!(err, ConfigError::InvalidTenantId(_)), "got: {err}");

        restore_env(saved);
    }

    /// SLICE 6 — production transport default is TCP. Even with `tcp`
    /// explicitly set, missing `SPENDGUARD_EXTPROC_SIDECAR_URL` in
    /// non-dev mode fails closed.
    #[test]
    fn tcp_transport_requires_sidecar_url_in_prod() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(TRANSPORT_ENV_KEYS);
        clear_transport_env();
        std::env::set_var("SPENDGUARD_EXTPROC_TENANT_ID", DEFAULT_TENANT_ID);
        // Explicit TCP, no dev mode, no SIDECAR_URL → MissingSidecarUrl.
        std::env::set_var("SPENDGUARD_EXTPROC_TRANSPORT", "tcp");

        let err = Config::from_env().expect_err("missing sidecar url must error");
        assert!(matches!(err, ConfigError::MissingSidecarUrl), "got: {err}");

        restore_env(saved);
    }

    /// SLICE 6 — plaintext http:// rejected; only https:// honoured.
    #[test]
    fn tcp_transport_rejects_plaintext_sidecar_url() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(TRANSPORT_ENV_KEYS);
        clear_transport_env();
        std::env::set_var("SPENDGUARD_EXTPROC_TENANT_ID", DEFAULT_TENANT_ID);
        std::env::set_var("SPENDGUARD_EXTPROC_TRANSPORT", "tcp");
        std::env::set_var(
            "SPENDGUARD_EXTPROC_SIDECAR_URL",
            "http://spendguard-sidecar:8443",
        );

        let err = Config::from_env().expect_err("plaintext http must error");
        assert!(
            matches!(err, ConfigError::InvalidSidecarUrl(ref s) if s.starts_with("http://")),
            "got: {err}"
        );

        restore_env(saved);
    }

    /// SLICE 6 — happy-path TCP transport with all required env set.
    #[test]
    fn tcp_transport_happy_path_assembles_full_config() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(TRANSPORT_ENV_KEYS);
        clear_transport_env();
        std::env::set_var("SPENDGUARD_EXTPROC_TENANT_ID", DEFAULT_TENANT_ID);
        std::env::set_var("SPENDGUARD_EXTPROC_TRANSPORT", "tcp");
        std::env::set_var(
            "SPENDGUARD_EXTPROC_SIDECAR_URL",
            "https://spendguard-sidecar:8443",
        );
        std::env::set_var(
            "SPENDGUARD_EXTPROC_CLIENT_CERT_PATH",
            "/run/secrets/svid/tls.crt",
        );
        std::env::set_var(
            "SPENDGUARD_EXTPROC_CLIENT_KEY_PATH",
            "/run/secrets/svid/tls.key",
        );
        std::env::set_var(
            "SPENDGUARD_EXTPROC_CA_BUNDLE_PATH",
            "/run/secrets/svid/ca.crt",
        );

        let cfg = Config::from_env().expect("tcp config must load");
        match &cfg.transport {
            Transport::Tcp {
                sidecar_url,
                expected_sidecar_svid_prefix,
                ..
            } => {
                assert_eq!(sidecar_url, "https://spendguard-sidecar:8443");
                assert_eq!(expected_sidecar_svid_prefix, SIDECAR_SVID_PREFIX);
            }
            other => panic!("expected Tcp transport, got {other:?}"),
        }
        assert!(cfg.sidecar_uds_path().is_none(), "TCP mode has no UDS path");

        restore_env(saved);
    }

    /// SLICE 6 — invalid `SPENDGUARD_EXTPROC_TRANSPORT` value returns
    /// typed `InvalidTransport` so a typo is loud at boot.
    #[test]
    fn invalid_transport_value_returns_typed_error() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(TRANSPORT_ENV_KEYS);
        clear_transport_env();
        std::env::set_var("SPENDGUARD_EXTPROC_TENANT_ID", DEFAULT_TENANT_ID);
        std::env::set_var("SPENDGUARD_EXTPROC_TRANSPORT", "uds-but-with-typo");

        let err = Config::from_env().expect_err("unknown transport must error");
        assert!(
            matches!(err, ConfigError::InvalidTransport(_)),
            "got: {err}"
        );

        restore_env(saved);
    }

    /// SLICE 6 — UDS transport honoured only when `uds-dev` feature on.
    /// This test runs unconditionally; under default features it must
    /// succeed; with `--no-default-features` it must fail closed.
    #[test]
    fn uds_transport_gated_by_uds_dev_feature() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(TRANSPORT_ENV_KEYS);
        clear_transport_env();
        std::env::set_var("SPENDGUARD_EXTPROC_TENANT_ID", DEFAULT_TENANT_ID);
        std::env::set_var("SPENDGUARD_EXTPROC_TRANSPORT", "uds");

        let result = Config::from_env();
        #[cfg(feature = "uds-dev")]
        {
            let cfg = result.expect("uds-dev feature → UDS transport must load");
            assert!(cfg.sidecar_uds_path().is_some());
        }
        #[cfg(not(feature = "uds-dev"))]
        {
            let err = result.expect_err("no uds-dev feature → UDS rejected");
            assert!(
                matches!(err, ConfigError::UdsTransportDisabled),
                "got: {err}"
            );
        }

        restore_env(saved);
    }

    /// SLICE 6 — `SPENDGUARD_EXTPROC_READYZ_ADDR` round-trips parsed.
    #[test]
    fn readyz_addr_parsed_from_env() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(TRANSPORT_ENV_KEYS);
        clear_transport_env();
        std::env::set_var("SPENDGUARD_EXTPROC_TENANT_ID", DEFAULT_TENANT_ID);
        std::env::set_var("SPENDGUARD_EXTPROC_DEV_MODE", "1");
        std::env::set_var("SPENDGUARD_EXTPROC_READYZ_ADDR", "127.0.0.1:18181");

        let cfg = Config::from_env().expect("readyz override must load");
        assert_eq!(cfg.readyz_addr.to_string(), "127.0.0.1:18181");

        restore_env(saved);
    }

    /// SLICE 6 — invalid readyz addr returns typed error (no `unwrap`).
    #[test]
    fn invalid_readyz_addr_returns_typed_error() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(TRANSPORT_ENV_KEYS);
        clear_transport_env();
        std::env::set_var("SPENDGUARD_EXTPROC_TENANT_ID", DEFAULT_TENANT_ID);
        std::env::set_var("SPENDGUARD_EXTPROC_DEV_MODE", "1");
        std::env::set_var("SPENDGUARD_EXTPROC_READYZ_ADDR", "not::a::addr");

        let err = Config::from_env().expect_err("invalid readyz addr must error");
        assert!(
            matches!(err, ConfigError::InvalidReadyzAddr { .. }),
            "got: {err}"
        );

        restore_env(saved);
    }

    /// SLICE 6 — `parse_transport_env` pure-helper coverage. We exercise
    /// the dev-mode default branch and the `tcp` / unknown branches
    /// without mutating process env.
    #[test]
    fn parse_transport_env_default_to_tcp_in_dev() {
        // `tcp` default in dev mode honours the default sidecar URL
        // fallback. Production posture rejects missing URL.
        let t = parse_transport_env("", true).expect("dev mode + empty → Tcp with default URL");
        assert!(t.is_tcp());
        let err = parse_transport_env("", false).expect_err("non-dev + empty + no URL → error");
        assert!(matches!(err, ConfigError::MissingSidecarUrl));
        let err = parse_transport_env("bogus", false).expect_err("bogus transport rejected");
        assert!(matches!(err, ConfigError::InvalidTransport(_)));
    }

    #[test]
    fn request_timeout_ms_env_overrides_default() {
        // Pure parser — no env mutation required.
        // Unset / empty / garbage all fall back to the 75ms default.
        assert_eq!(parse_request_timeout_ms(None), 75);
        assert_eq!(parse_request_timeout_ms(Some("")), 75);
        assert_eq!(parse_request_timeout_ms(Some("not-a-number")), 75);
        // Zero is also rejected — a zero timeout would defeat the
        // fail-closed gate, so we fall back to the default.
        assert_eq!(parse_request_timeout_ms(Some("0")), 75);
        // Valid override is honoured exactly. Smaller (50ms — spec
        // envelope) and larger (250ms — ops emergency override) both
        // round-trip without clamping.
        assert_eq!(parse_request_timeout_ms(Some("50")), 50);
        assert_eq!(parse_request_timeout_ms(Some("250")), 250);
    }

    #[test]
    fn from_env_picks_up_request_timeout_override() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(
            &[
                TRANSPORT_ENV_KEYS,
                &["SPENDGUARD_EXTPROC_REQUEST_TIMEOUT_MS"][..],
            ]
            .concat(),
        );
        clear_transport_env();
        std::env::remove_var("SPENDGUARD_EXTPROC_REQUEST_TIMEOUT_MS");
        std::env::set_var("SPENDGUARD_EXTPROC_TENANT_ID", DEFAULT_TENANT_ID);
        // dev_mode lets the transport default to TCP without us having
        // to set SIDECAR_URL — keeps the test focused on timeout.
        std::env::set_var("SPENDGUARD_EXTPROC_DEV_MODE", "1");
        std::env::set_var("SPENDGUARD_EXTPROC_REQUEST_TIMEOUT_MS", "120");

        let cfg = Config::from_env().expect("config must load");
        assert_eq!(cfg.sidecar_request_timeout, Duration::from_millis(120));

        restore_env(saved);
    }

    #[test]
    fn invalid_bind_addr_returns_typed_error() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(TRANSPORT_ENV_KEYS);
        clear_transport_env();
        std::env::set_var("SPENDGUARD_EXTPROC_BIND_ADDR", "not::a::addr");
        // Ensure tenant_id is valid so the bind error path triggers
        // before tenant validation.
        std::env::set_var("SPENDGUARD_EXTPROC_TENANT_ID", DEFAULT_TENANT_ID);

        let err = Config::from_env().expect_err("invalid addr must error");
        assert!(matches!(err, ConfigError::InvalidBindAddr { .. }));

        restore_env(saved);
    }
}
