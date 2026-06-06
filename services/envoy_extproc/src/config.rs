//! Configuration surface — env-driven, fail-closed.
//!
//! Spec ref: docs/specs/coverage/D01_envoy_extproc/implementation.md §11.
//!
//! SLICE 1 only reads the bind address + the sidecar UDS path. Required
//! production env vars (SVID cert paths, sidecar mTLS URI, tenant id,
//! etc.) are listed in the implementation §11 table and will be wired
//! in SLICE 6 (Helm) so they don't gate the SLICE 1 skeleton.
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
/// Demo-only tenant id. Used by [`Config::for_test`] and the
/// `SPENDGUARD_EXTPROC_DEV_MODE=1` fallback. Production deployments MUST
/// set `SPENDGUARD_EXTPROC_TENANT_ID` or `Config::from_env` fails closed.
const DEFAULT_TENANT_ID: &str = "00000000-0000-4000-8000-000000000001";

/// Loaded configuration for one envoy_extproc process.
#[derive(Debug, Clone)]
pub struct Config {
    /// gRPC bind address. Default `127.0.0.1:9443` to match the
    /// implementation §11 table.
    pub bind_addr: SocketAddr,
    /// Path to the SpendGuard sidecar adapter UDS. Default
    /// `/var/run/spendguard/adapter.sock` matches the egress_proxy
    /// default (see services/egress_proxy/src/sidecar_client.rs).
    pub sidecar_uds_path: PathBuf,
    /// Tenant id assertion forwarded in the sidecar Handshake. POC
    /// default value is a v4-shaped UUID for the demo flow; production
    /// deployments MUST set `SPENDGUARD_EXTPROC_TENANT_ID`.
    pub tenant_id: String,
    /// Stable per-process workload id; default is a v4 UUID prefix so
    /// crash-restarts can be correlated in audit logs.
    pub workload_instance_id: String,
    /// Total retry deadline for sidecar handshake. Matches the
    /// egress_proxy default (30s) so docker-compose `depends_on:
    /// service_started` races don't crash the binary.
    pub sidecar_startup_deadline: Duration,
    pub sidecar_initial_backoff: Duration,
    pub sidecar_max_backoff: Duration,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("SPENDGUARD_EXTPROC_BIND_ADDR `{value}` failed to parse: {source}")]
    InvalidBindAddr {
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

        let sidecar_uds_path = std::env::var("SPENDGUARD_EXTPROC_SIDECAR_UDS_PATH")
            .unwrap_or_else(|_| DEFAULT_SIDECAR_UDS.into())
            .into();

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

        let workload_instance_id = std::env::var("SPENDGUARD_EXTPROC_WORKLOAD_INSTANCE_ID")
            .unwrap_or_else(|_| format!("envoy-extproc-{}", uuid::Uuid::new_v4().simple()));

        Ok(Self {
            bind_addr,
            sidecar_uds_path,
            tenant_id,
            workload_instance_id,
            sidecar_startup_deadline: Duration::from_secs(30),
            sidecar_initial_backoff: Duration::from_millis(250),
            sidecar_max_backoff: Duration::from_secs(4),
        })
    }

    /// Test-only constructor.
    #[doc(hidden)]
    pub fn for_test(bind_addr: SocketAddr) -> Self {
        Self {
            bind_addr,
            sidecar_uds_path: DEFAULT_SIDECAR_UDS.into(),
            tenant_id: DEFAULT_TENANT_ID.into(),
            workload_instance_id: "envoy-extproc-test".into(),
            sidecar_startup_deadline: Duration::from_secs(1),
            sidecar_initial_backoff: Duration::from_millis(50),
            sidecar_max_backoff: Duration::from_millis(200),
        }
    }
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

    #[test]
    fn missing_tenant_id_fails_in_prod() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(&[
            "SPENDGUARD_EXTPROC_BIND_ADDR",
            "SPENDGUARD_EXTPROC_TENANT_ID",
            "SPENDGUARD_EXTPROC_SIDECAR_UDS_PATH",
            "SPENDGUARD_EXTPROC_WORKLOAD_INSTANCE_ID",
            "SPENDGUARD_EXTPROC_DEV_MODE",
        ]);
        std::env::remove_var("SPENDGUARD_EXTPROC_BIND_ADDR");
        std::env::remove_var("SPENDGUARD_EXTPROC_TENANT_ID");
        std::env::remove_var("SPENDGUARD_EXTPROC_SIDECAR_UDS_PATH");
        std::env::remove_var("SPENDGUARD_EXTPROC_WORKLOAD_INSTANCE_ID");
        std::env::remove_var("SPENDGUARD_EXTPROC_DEV_MODE");

        // Production posture: missing tenant id must fail closed.
        let err = Config::from_env().expect_err("missing tenant must fail closed");
        assert!(matches!(err, ConfigError::MissingTenantId), "got: {err}");

        restore_env(saved);
    }

    #[test]
    fn missing_tenant_id_uses_demo_in_dev_mode() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(&[
            "SPENDGUARD_EXTPROC_BIND_ADDR",
            "SPENDGUARD_EXTPROC_TENANT_ID",
            "SPENDGUARD_EXTPROC_SIDECAR_UDS_PATH",
            "SPENDGUARD_EXTPROC_WORKLOAD_INSTANCE_ID",
            "SPENDGUARD_EXTPROC_DEV_MODE",
        ]);
        std::env::remove_var("SPENDGUARD_EXTPROC_BIND_ADDR");
        std::env::remove_var("SPENDGUARD_EXTPROC_TENANT_ID");
        std::env::remove_var("SPENDGUARD_EXTPROC_SIDECAR_UDS_PATH");
        std::env::remove_var("SPENDGUARD_EXTPROC_WORKLOAD_INSTANCE_ID");
        std::env::set_var("SPENDGUARD_EXTPROC_DEV_MODE", "1");

        let cfg = Config::from_env().expect("dev mode falls back to demo tenant");
        assert_eq!(cfg.bind_addr.to_string(), "127.0.0.1:9443");
        assert_eq!(cfg.tenant_id, DEFAULT_TENANT_ID);

        restore_env(saved);
    }

    #[test]
    fn invalid_tenant_id_returns_typed_error() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(&[
            "SPENDGUARD_EXTPROC_BIND_ADDR",
            "SPENDGUARD_EXTPROC_TENANT_ID",
            "SPENDGUARD_EXTPROC_DEV_MODE",
        ]);
        // Reset BIND_ADDR so the bind parse succeeds and we exercise the
        // tenant-id validation branch specifically.
        std::env::remove_var("SPENDGUARD_EXTPROC_BIND_ADDR");
        std::env::remove_var("SPENDGUARD_EXTPROC_DEV_MODE");
        std::env::set_var("SPENDGUARD_EXTPROC_TENANT_ID", "not-a-uuid");

        let err = Config::from_env().expect_err("invalid uuid must error");
        assert!(matches!(err, ConfigError::InvalidTenantId(_)), "got: {err}");

        restore_env(saved);
    }

    #[test]
    fn invalid_bind_addr_returns_typed_error() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env(&[
            "SPENDGUARD_EXTPROC_BIND_ADDR",
            "SPENDGUARD_EXTPROC_TENANT_ID",
            "SPENDGUARD_EXTPROC_DEV_MODE",
        ]);
        std::env::set_var("SPENDGUARD_EXTPROC_BIND_ADDR", "not::a::addr");
        // Ensure tenant_id is valid so the bind error path triggers
        // before tenant validation.
        std::env::set_var("SPENDGUARD_EXTPROC_TENANT_ID", DEFAULT_TENANT_ID);
        std::env::remove_var("SPENDGUARD_EXTPROC_DEV_MODE");

        let err = Config::from_env().expect_err("invalid addr must error");
        assert!(matches!(err, ConfigError::InvalidBindAddr { .. }));

        restore_env(saved);
    }
}
