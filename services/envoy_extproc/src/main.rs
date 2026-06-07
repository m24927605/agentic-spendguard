//! `spendguard-envoy-extproc` — Envoy AI Gateway ExtProc sidecar (D01).
//!
//! Translates Envoy ExternalProcessor gRPC calls into SpendGuard sidecar
//! adapter calls. SLICE 6 hard-switches production transport from UDS
//! (SLICE 1-5 carve-out) to mTLS-over-TCP per design §3.3 and lands the
//! `/readyz` + `/livez` HTTP probe alongside.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.3 (transport hard-switch), §3.4 (fail-closed)
//!   - docs/specs/coverage/D01_envoy_extproc/implementation.md §3 + §11
//!   - docs/slices/COV_06_envoy_extproc_helm.md
//!
//! Fail-closed on the sidecar handshake: if the sidecar isn't reachable
//! within the startup deadline, the process exits non-zero so a
//! Kubernetes restart loop / docker-compose surfaces the failure.
//! Review standards §2.2 + design §3.4.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Context;
use tonic::transport::Server;
use tracing::{info, warn};

use spendguard_envoy_extproc::{
    config::{Config, Transport},
    handshake,
    proto::envoy::service::ext_proc::v3::external_processor_server::ExternalProcessorServer,
    readyz,
    server::ExtProcService,
    sidecar_client::SidecarClient,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cfg = Config::from_env().context("loading envoy_extproc config")?;
    info!(
        bind_addr = %cfg.bind_addr,
        readyz_addr = %cfg.readyz_addr,
        transport = %transport_summary(&cfg.transport),
        tenant = %cfg.tenant_id,
        workload = %cfg.workload_instance_id,
        "spendguard-envoy-extproc starting"
    );

    // COV_01: install no-op routing extractors so the shared crate is
    // valid even though SLICE 1 doesn't dispatch on route() yet.
    install_noop_extractors();

    // SLICE 6 — start the `/readyz` + `/livez` HTTP probe immediately so
    // Kubernetes can distinguish "boot in progress" (503) from "ready"
    // (200) without waiting for the sidecar handshake to complete.
    // The probe flips to 200 once we set `ready = true` below.
    let ready = Arc::new(AtomicBool::new(false));
    let readyz_handle = readyz::spawn(cfg.readyz_addr, ready.clone()).await?;

    // Fail-closed startup dial of the sidecar — for TCP (production)
    // this is a TCP SYN against the parsed `https://host:port`; for the
    // SLICE 1-5 UDS carve-out it's a `UnixStream::connect`. If the
    // sidecar isn't reachable within the deadline, exit non-zero and
    // let the orchestrator restart us (design §3.4).
    match handshake::dial_sidecar_with_retry(&cfg).await {
        Ok(_) => info!("sidecar reachable; building transport client"),
        Err(e) => {
            warn!(err = %e, "sidecar unreachable — exiting non-zero (fail-closed)");
            return Err(anyhow::Error::new(e).context("sidecar handshake"));
        }
    }

    // SLICE 6 — build the sidecar client via `connect_transport`. For
    // TCP this constructs the mTLS rustls channel with SPIFFE URI SAN
    // pinning; for UDS (dev) it falls through to the SLICE 1-5
    // `connect` path.
    let sidecar = SidecarClient::connect_transport(
        &cfg.transport,
        &cfg.tenant_id,
        cfg.sidecar_request_timeout,
    )
    .await
    .context("building sidecar client")?;
    info!(
        identity = %sidecar.identity(),
        timeout_ms = cfg.sidecar_request_timeout.as_millis() as u64,
        "SLICE 6 sidecar client wired"
    );

    // Mark ready — both the sidecar handshake succeeded AND the
    // transport client is built. Kubernetes `readinessProbe` flips to
    // 200 on the next scrape.
    ready.store(true, Ordering::SeqCst);

    let svc = ExtProcService::new(cfg.tenant_id.clone()).with_sidecar(sidecar);

    info!(addr = %cfg.bind_addr, "binding ExternalProcessor gRPC server");
    let serve_result = Server::builder()
        .add_service(ExternalProcessorServer::new(svc))
        .serve_with_shutdown(cfg.bind_addr, async {
            tokio::signal::ctrl_c().await.ok();
            info!("shutdown signal received; draining");
        })
        .await;

    // Mark not-ready before the readyz task is dropped so any in-flight
    // probe sees 503. The task is then aborted (background JoinHandle).
    ready.store(false, Ordering::SeqCst);
    readyz_handle.abort();

    serve_result.context("ExternalProcessor server terminated")?;
    Ok(())
}

/// SLICE 6 — log-friendly summary of the active transport. The full
/// URL is fine at INFO (the operator sees the same value in their
/// rendered Helm Deployment); cert paths are NOT included.
fn transport_summary(transport: &Transport) -> String {
    match transport {
        Transport::Tcp { sidecar_url, .. } => format!("tcp:{sidecar_url}"),
        Transport::Uds { socket_path } => format!("uds:{}", socket_path.display()),
    }
}

fn init_tracing() {
    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .init();
}

/// SLICE 1 placeholder. SLICE 2 / SLICE 4 will register the real per-provider
/// response shape extractors from services/egress_proxy/src/providers/ via
/// a small adapter facade. Until then we register zero-returning stubs so
/// the shared crate's OnceLock is populated and any spurious route() call
/// from a future code path does not panic.
fn install_noop_extractors() {
    use spendguard_provider_routing::{init_extractors, RoutingExtractors, UsageMetrics};
    fn noop(_: &serde_json::Value) -> UsageMetrics {
        UsageMetrics::default()
    }
    // `init_extractors` returns Err if called twice; main() only calls
    // it once so a duplicate would be a bug — log it but proceed.
    if let Err(e) = init_extractors(RoutingExtractors {
        openai: noop,
        anthropic: noop,
        bedrock: noop,
        vertex: noop,
        azure_openai: noop,
    }) {
        warn!(
            err = e,
            "routing extractor re-registration (SLICE 1 placeholder)"
        );
    }
}
