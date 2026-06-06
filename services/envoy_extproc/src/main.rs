//! `spendguard-envoy-extproc` — Envoy AI Gateway ExtProc sidecar (D01).
//!
//! Translates Envoy ExternalProcessor gRPC calls into SpendGuard sidecar
//! adapter calls. SLICE 1 ships the binary skeleton: gRPC server bound
//! on the configured address, ExternalProcessor.Process Handshake-frame
//! ACK, sidecar UDS startup dial.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md
//!   - docs/specs/coverage/D01_envoy_extproc/implementation.md §3
//!   - docs/slices/COV_01_envoy_extproc_skeleton.md
//!
//! Fail-closed on the sidecar UDS handshake: if the sidecar isn't
//! reachable within the startup deadline, this process exits non-zero
//! so a Kubernetes restart loop / docker-compose surfaces the failure.
//! Review standards §2.2 + design §3.4.

use anyhow::Context;
use tonic::transport::Server;
use tracing::{info, warn};

use spendguard_envoy_extproc::{
    config::Config, handshake,
    proto::envoy::service::ext_proc::v3::external_processor_server::ExternalProcessorServer,
    server::ExtProcService,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cfg = Config::from_env().context("loading envoy_extproc config")?;
    info!(
        bind_addr = %cfg.bind_addr,
        sidecar_uds = %cfg.sidecar_uds_path.display(),
        tenant = %cfg.tenant_id,
        workload = %cfg.workload_instance_id,
        "spendguard-envoy-extproc starting (SLICE 1 skeleton)"
    );

    // COV_01: install no-op routing extractors so the shared crate is
    // valid even though SLICE 1 doesn't dispatch on route() yet. SLICE 2
    // replaces these with real per-provider response shape extractors
    // (which live in services/egress_proxy/src/providers/ today; the
    // envoy_extproc adapter can pull them in via a small adapter
    // facade once Request-Body translation lands).
    install_noop_extractors();

    // Fail-closed startup dial of the sidecar UDS — matches the
    // egress_proxy SLICE 4a pattern. If the sidecar isn't reachable
    // within the deadline, we exit non-zero and let the orchestrator
    // restart us; the design §3.4 fail-closed posture is preserved.
    match handshake::dial_sidecar_with_retry(&cfg).await {
        Ok(_) => info!("sidecar UDS reachable; ready to serve"),
        Err(e) => {
            warn!(err = %e, "sidecar UDS unreachable — exiting non-zero (fail-closed)");
            return Err(anyhow::Error::new(e).context("sidecar handshake"));
        }
    }

    let svc = ExtProcService::new(cfg.tenant_id.clone());

    info!(addr = %cfg.bind_addr, "binding ExternalProcessor gRPC server");
    Server::builder()
        .add_service(ExternalProcessorServer::new(svc))
        .serve_with_shutdown(cfg.bind_addr, async {
            tokio::signal::ctrl_c().await.ok();
            info!("shutdown signal received; draining");
        })
        .await
        .context("ExternalProcessor server terminated")?;

    Ok(())
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
    // `init_extractors` returns Err if called twice; SLICE 1 only calls
    // it from main() so a duplicate would be a bug — log it but proceed.
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
