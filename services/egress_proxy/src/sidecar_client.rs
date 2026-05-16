//! Sidecar UDS gRPC client for the egress proxy.
//!
//! Slice 4a deliverable per spec §15 row 4a:
//! - Connect to the sidecar's adapter UDS (default
//!   `/var/run/spendguard/adapter.sock`)
//! - Drive a Handshake at startup with retry-with-backoff (1s × 30s
//!   total deadline) so docker-compose `depends_on: service_started`
//!   races don't crash the proxy
//! - Reflect handshake state in `/readyz` via a shared atomic
//!
//! Mirrors the existing wrapper-SDK pattern at
//! `sdk/python/src/spendguard/client.py:212-307`. Spec invariants:
//! - This is a single long-lived connection; new requests reuse it
//! - Sidecar disconnect mid-flight → returns `tonic::Status` error;
//!   handlers translate to 502 per §4.2 fail-closed invariant.
//!
//! Slice 4b adds `request_decision`; slice 5 adds `emit_llm_call_post`
//! + `confirm_publish_outcome`.

use anyhow::Context;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;
use tracing::{info, warn};

use crate::proto::sidecar_adapter::v1::{
    handshake_request::CapabilityLevel,
    sidecar_adapter_client::SidecarAdapterClient,
    HandshakeRequest, HandshakeResponse,
};

pub type SidecarChannel = Channel;
pub type Client = SidecarAdapterClient<SidecarChannel>;

/// Shared per-process handle to the sidecar client + readiness flag.
///
/// `client` is `Clone`-cheap (it's a `tonic` channel underneath).
/// `ready` flips true after a successful Handshake. Health/readiness
/// route reads this atomic.
#[derive(Clone)]
pub struct SidecarHandle {
    pub client: Client,
    pub ready: Arc<AtomicBool>,
    pub session_id: String,
    pub tenant_id: String,
}

impl SidecarHandle {
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone)]
pub struct SidecarConfig {
    pub uds_path: PathBuf,
    pub tenant_id: String,
    pub workload_instance_id: String,
    /// Total retry deadline for connect + handshake.
    pub startup_deadline: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl SidecarConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let uds_path: PathBuf = std::env::var("SPENDGUARD_PROXY_SIDECAR_UDS_PATH")
            .unwrap_or_else(|_| "/var/run/spendguard/adapter.sock".to_string())
            .into();
        let tenant_id = std::env::var("SPENDGUARD_PROXY_DEFAULT_TENANT_ID")
            .context(
                "SPENDGUARD_PROXY_DEFAULT_TENANT_ID required (per spec §6.1 Path A); \
                 may be overridden per-request by X-SpendGuard-Tenant-Id (slice 6)",
            )?;
        // Validate UUID format at startup (codex r4 NEWr5-3 hint).
        uuid::Uuid::parse_str(&tenant_id).with_context(|| {
            format!("SPENDGUARD_PROXY_DEFAULT_TENANT_ID is not a valid UUID: {tenant_id}")
        })?;

        let workload_instance_id = std::env::var("SPENDGUARD_PROXY_WORKLOAD_INSTANCE_ID")
            .unwrap_or_else(|_| format!("egress-proxy-{}", uuid::Uuid::new_v4().simple()));

        Ok(Self {
            uds_path,
            tenant_id,
            workload_instance_id,
            startup_deadline: Duration::from_secs(30),
            initial_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(4),
        })
    }
}

/// Connect to the sidecar UDS + complete a handshake.
///
/// Retries the connect + handshake until `startup_deadline` elapses
/// (per spec §9 + codex r3 P2-r3.A 1s × 30s wrapper for ledger /
/// canonical_ingest gRPC clients). Each attempt has its own timeout;
/// the loop wraps them.
///
/// Returns a ready `SidecarHandle`. If the deadline elapses without
/// a successful handshake, returns an error and `main()` exits 1
/// (fail-fast per spec §9).
pub async fn connect_with_retry(cfg: &SidecarConfig) -> anyhow::Result<SidecarHandle> {
    let deadline = tokio::time::Instant::now() + cfg.startup_deadline;
    let mut backoff = cfg.initial_backoff;
    let mut attempt: u32 = 0;

    loop {
        attempt += 1;

        match try_connect_and_handshake(cfg).await {
            Ok(handle) => {
                info!(
                    attempts = attempt,
                    session_id = %handle.session_id,
                    "sidecar UDS handshake succeeded"
                );
                return Ok(handle);
            }
            Err(e) => {
                if tokio::time::Instant::now() >= deadline {
                    anyhow::bail!(
                        "sidecar UDS connect+handshake to {} failed after {} attempts ({}s deadline): {}",
                        cfg.uds_path.display(),
                        attempt,
                        cfg.startup_deadline.as_secs(),
                        e,
                    );
                }
                warn!(
                    uds = %cfg.uds_path.display(),
                    attempt,
                    err = %e,
                    next_backoff_ms = backoff.as_millis() as u64,
                    "sidecar handshake failed; retrying"
                );
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, cfg.max_backoff);
            }
        }
    }
}

async fn try_connect_and_handshake(cfg: &SidecarConfig) -> anyhow::Result<SidecarHandle> {
    let channel = build_uds_channel(&cfg.uds_path).await?;
    let mut client = SidecarAdapterClient::new(channel.clone());

    let req = HandshakeRequest {
        sdk_version: format!("egress-proxy/{}", env!("CARGO_PKG_VERSION")),
        runtime_kind: "egress-proxy".to_string(),
        runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        capability_level: CapabilityLevel::L1LlmCall as i32,
        tenant_id_assertion: cfg.tenant_id.clone(),
        workload_instance_id: cfg.workload_instance_id.clone(),
        protocol_version: 1,
        // The proto has additional optional fields (key epochs, etc.)
        // — defaults are fine for the proxy adapter.
        ..Default::default()
    };

    let resp: HandshakeResponse = client.handshake(req).await.map(|r| r.into_inner())?;

    let ready = Arc::new(AtomicBool::new(true));
    Ok(SidecarHandle {
        client,
        ready,
        session_id: resp.session_id,
        tenant_id: cfg.tenant_id.clone(),
    })
}

/// Build a tonic Channel over Unix Domain Socket.
///
/// tonic's transport layer is HTTP/2-over-TCP by default. UDS support
/// is achieved by handing tonic a custom `service_fn` that produces
/// `tokio::net::UnixStream` instances wrapped in hyper-util's
/// `TokioIo`. Mirrors the pattern used by `services/sidecar/src/main.rs`
/// for the server side; here we mirror it on the client side.
async fn build_uds_channel(path: &std::path::Path) -> anyhow::Result<Channel> {
    let path = path.to_path_buf();

    // The URI here is a placeholder — tonic requires it to parse but
    // the actual connection goes through the service_fn below.
    let endpoint = Endpoint::try_from("http://[::1]:50051")?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10));

    let channel = endpoint
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = path.clone();
            async move {
                UnixStream::connect(path)
                    .await
                    .map(hyper_util::rt::TokioIo::new)
            }
        }))
        .await?;
    Ok(channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_config_defaults_validate_tenant_uuid() {
        // Save + restore env to avoid test pollution.
        let original = std::env::var("SPENDGUARD_PROXY_DEFAULT_TENANT_ID").ok();

        std::env::set_var("SPENDGUARD_PROXY_DEFAULT_TENANT_ID", "not-a-uuid");
        let result = SidecarConfig::from_env();
        assert!(result.is_err(), "invalid UUID must fail at startup");
        let msg = format!("{:#}", result.unwrap_err());
        assert!(msg.contains("not a valid UUID"), "got: {msg}");

        std::env::set_var(
            "SPENDGUARD_PROXY_DEFAULT_TENANT_ID",
            "00000000-0000-4000-8000-000000000001",
        );
        let result = SidecarConfig::from_env();
        assert!(result.is_ok(), "valid UUID must succeed: {:?}", result);
        let cfg = result.unwrap();
        assert_eq!(
            cfg.tenant_id,
            "00000000-0000-4000-8000-000000000001"
        );

        // Restore.
        if let Some(v) = original {
            std::env::set_var("SPENDGUARD_PROXY_DEFAULT_TENANT_ID", v);
        } else {
            std::env::remove_var("SPENDGUARD_PROXY_DEFAULT_TENANT_ID");
        }
    }

    #[test]
    fn sidecar_config_workload_default_is_uuid() {
        let original_tid = std::env::var("SPENDGUARD_PROXY_DEFAULT_TENANT_ID").ok();
        let original_wid = std::env::var("SPENDGUARD_PROXY_WORKLOAD_INSTANCE_ID").ok();

        std::env::set_var(
            "SPENDGUARD_PROXY_DEFAULT_TENANT_ID",
            "00000000-0000-4000-8000-000000000001",
        );
        std::env::remove_var("SPENDGUARD_PROXY_WORKLOAD_INSTANCE_ID");
        let cfg = SidecarConfig::from_env().unwrap();
        assert!(cfg.workload_instance_id.starts_with("egress-proxy-"));

        if let Some(v) = original_tid {
            std::env::set_var("SPENDGUARD_PROXY_DEFAULT_TENANT_ID", v);
        } else {
            std::env::remove_var("SPENDGUARD_PROXY_DEFAULT_TENANT_ID");
        }
        if let Some(v) = original_wid {
            std::env::set_var("SPENDGUARD_PROXY_WORKLOAD_INSTANCE_ID", v);
        }
    }
}
