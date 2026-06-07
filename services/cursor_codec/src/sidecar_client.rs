//! Sidecar UDS gRPC client for the Cursor MITM codec.
//!
//! D17 SLICE 6 ([`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md)
//! §7 + [`implementation.md`](../../docs/specs/coverage/D17_cursor_mitm/implementation.md)
//! §6): once the codec has decoded a Cursor request and translated it
//! to the canonical OpenAI shape, the SLICE 6 MITM session feeds it to
//! the same sidecar UDS gRPC surface that `services/egress_proxy`
//! talks to. This module is the client.
//!
//! ## Why a separate client (vs. depending on `services/egress_proxy`)
//!
//! The egress proxy is a binary, not a library — there's no reusable
//! crate-level client there. We mirror the
//! [`services/egress_proxy/src/sidecar_client.rs`] pattern verbatim
//! (UDS connector via `tower::service_fn` + `tokio::net::UnixStream`,
//! `hyper-util::TokioIo` adapter, retry-with-backoff handshake) so
//! the two clients converge as the sidecar surface evolves.
//!
//! ## Feature gate
//!
//! Compiled only under `--features mitm`. The default build (no
//! `mitm`) has no transitive dependency on `tonic` / `tokio`, which
//! keeps SLICE 5 consumers (pure translation) build-cheap.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;
use tracing::{info, warn};

/// Generated gRPC stubs for the sidecar adapter + common types. Mirrors
/// `services/egress_proxy/src/proto.rs`.
#[allow(clippy::all, missing_docs)]
pub mod proto {
    /// Common types shared across all SpendGuard services.
    pub mod common {
        /// v1 of the common schema.
        pub mod v1 {
            tonic::include_proto!("spendguard.common.v1");
        }
    }
    /// Sidecar adapter UDS gRPC surface.
    pub mod sidecar_adapter {
        /// v1 of the adapter wire.
        pub mod v1 {
            tonic::include_proto!("spendguard.sidecar_adapter.v1");
        }
    }
}

use proto::sidecar_adapter::v1::{
    handshake_request::CapabilityLevel, sidecar_adapter_client::SidecarAdapterClient,
    HandshakeRequest, HandshakeResponse,
};

/// The underlying tonic channel.
pub type SidecarChannel = Channel;

/// The typed gRPC client.
pub type Client = SidecarAdapterClient<SidecarChannel>;

/// Shared per-process handle to the sidecar client + readiness flag.
///
/// Cheap to `Clone` — the underlying `tonic::Channel` is reference-
/// counted. The MITM session machine clones this once per Cursor
/// connection and the clones share the underlying connection.
#[derive(Clone)]
pub struct SidecarHandle {
    /// Cloneable tonic client.
    pub client: Client,
    /// `true` after handshake completes.
    pub ready: Arc<AtomicBool>,
    /// Negotiated session id from the handshake.
    pub session_id: String,
    /// Tenant id the handshake asserted.
    pub tenant_id: String,
}

impl SidecarHandle {
    /// `true` after the handshake has landed.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }
}

/// Configuration for connecting to the sidecar UDS.
#[derive(Debug, Clone)]
pub struct SidecarConfig {
    /// Path to the sidecar's UDS (default `/var/run/spendguard/adapter.sock`).
    pub uds_path: PathBuf,
    /// Tenant id assertion sent in the handshake.
    pub tenant_id: String,
    /// Workload instance id for fencing parity.
    pub workload_instance_id: String,
    /// Total wall-clock deadline for connect + handshake retry.
    pub startup_deadline: Duration,
    /// Initial backoff between handshake retries.
    pub initial_backoff: Duration,
    /// Cap on the backoff after exponential growth.
    pub max_backoff: Duration,
}

impl SidecarConfig {
    /// Build a config the way the live MITM proxy would — environment
    /// variables for the UDS path and tenant id, sensible defaults for
    /// the backoff knobs.
    pub fn from_env() -> anyhow::Result<Self> {
        let uds_path: PathBuf = std::env::var("SPENDGUARD_CURSOR_MITM_SIDECAR_UDS_PATH")
            .unwrap_or_else(|_| "/var/run/spendguard/adapter.sock".to_string())
            .into();
        let tenant_id = std::env::var("SPENDGUARD_CURSOR_MITM_TENANT_ID").context(
            "SPENDGUARD_CURSOR_MITM_TENANT_ID required (D17 design §5 / §7); \
             see docs/customer/sow-cursor-mitm.md for SOW provisioning",
        )?;
        uuid::Uuid::parse_str(&tenant_id).with_context(|| {
            format!("SPENDGUARD_CURSOR_MITM_TENANT_ID is not a valid UUID: {tenant_id}")
        })?;

        let workload_instance_id = std::env::var("SPENDGUARD_CURSOR_MITM_WORKLOAD_INSTANCE_ID")
            .unwrap_or_else(|_| format!("cursor-mitm-{}", uuid::Uuid::new_v4().simple()));

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
/// Mirrors the egress proxy retry-with-backoff (1s × 30s deadline)
/// so docker-compose `depends_on: service_started` races don't crash
/// the MITM listener at boot.
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
                    "cursor-mitm sidecar UDS handshake succeeded"
                );
                return Ok(handle);
            }
            Err(e) => {
                if tokio::time::Instant::now() >= deadline {
                    anyhow::bail!(
                        "cursor-mitm sidecar UDS connect+handshake to {} failed after {} attempts ({}s deadline): {}",
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
                    "cursor-mitm sidecar handshake failed; retrying"
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

    // Proto3 additive evolution: when adapter.proto adds a new
    // HandshakeRequest field (capability claims, key epochs, etc.) the
    // codec MUST keep compiling without changes here. `..Default::default()`
    // future-proofs against that even though every field today is
    // explicitly set; #[allow(clippy::needless_update)] documents the
    // forward-compat intent (mirrors services/egress_proxy/src/sidecar_client.rs).
    #[allow(clippy::needless_update)]
    let req = HandshakeRequest {
        sdk_version: format!("cursor-mitm/{}", env!("CARGO_PKG_VERSION")),
        runtime_kind: "cursor-mitm".to_string(),
        runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        // L1_LLM_CALL: the MITM only intercepts the LLM call boundary
        // on the Cursor wire (no agent/step/tool visibility). Same
        // capability the egress proxy declares.
        capability_level: CapabilityLevel::L1LlmCall as i32,
        tenant_id_assertion: cfg.tenant_id.clone(),
        workload_instance_id: cfg.workload_instance_id.clone(),
        protocol_version: 1,
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

/// Build a tonic `Channel` over a Unix Domain Socket.
///
/// Mirrors `services/egress_proxy/src/sidecar_client.rs::build_uds_channel`
/// verbatim — same `service_fn` UDS connector trick, same placeholder
/// URI (tonic's transport layer is HTTP/2-over-TCP by default; UDS
/// support is achieved by handing tonic a custom connector that
/// produces `tokio::net::UnixStream` wrapped in `hyper-util`'s
/// `TokioIo`).
pub async fn build_uds_channel(path: &std::path::Path) -> anyhow::Result<Channel> {
    let path = path.to_path_buf();
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
    fn sidecar_config_rejects_invalid_tenant_uuid() {
        let original = std::env::var("SPENDGUARD_CURSOR_MITM_TENANT_ID").ok();

        std::env::set_var("SPENDGUARD_CURSOR_MITM_TENANT_ID", "not-a-uuid");
        let result = SidecarConfig::from_env();
        assert!(result.is_err(), "invalid UUID must fail at startup");
        let msg = format!("{:#}", result.unwrap_err());
        assert!(msg.contains("not a valid UUID"), "got: {msg}");

        if let Some(v) = original {
            std::env::set_var("SPENDGUARD_CURSOR_MITM_TENANT_ID", v);
        } else {
            std::env::remove_var("SPENDGUARD_CURSOR_MITM_TENANT_ID");
        }
    }

    #[test]
    fn sidecar_config_accepts_valid_tenant_uuid() {
        let original = std::env::var("SPENDGUARD_CURSOR_MITM_TENANT_ID").ok();
        let original_wid = std::env::var("SPENDGUARD_CURSOR_MITM_WORKLOAD_INSTANCE_ID").ok();

        std::env::set_var(
            "SPENDGUARD_CURSOR_MITM_TENANT_ID",
            "00000000-0000-4000-8000-000000000001",
        );
        std::env::remove_var("SPENDGUARD_CURSOR_MITM_WORKLOAD_INSTANCE_ID");
        let cfg = SidecarConfig::from_env().unwrap();
        assert_eq!(cfg.tenant_id, "00000000-0000-4000-8000-000000000001");
        assert!(cfg.workload_instance_id.starts_with("cursor-mitm-"));

        if let Some(v) = original {
            std::env::set_var("SPENDGUARD_CURSOR_MITM_TENANT_ID", v);
        } else {
            std::env::remove_var("SPENDGUARD_CURSOR_MITM_TENANT_ID");
        }
        if let Some(v) = original_wid {
            std::env::set_var("SPENDGUARD_CURSOR_MITM_WORKLOAD_INSTANCE_ID", v);
        }
    }
}
