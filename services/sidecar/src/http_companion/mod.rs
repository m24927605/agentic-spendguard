//! HTTP companion listener for out-of-process plugins (D09 SLICE 1).
//!
//! ## What this module is
//!
//! A small axum + mTLS HTTP/1.1 surface that mirrors a subset of the
//! gRPC adapter UDS contract. It exists because some plugin runtimes —
//! the Kong Gateway Go-PDK worker (D09), the Coze plugin daemon (D31),
//! and the Botpress integration (D32) — live in a separate network
//! namespace from the SpendGuard sidecar pod and therefore cannot use
//! the SO_PEERCRED-authenticated UDS that the in-process adapter speaks.
//! All three need ALLOW/DENY/DEGRADE guidance + a downstream commit
//! event, and shipping a fresh gRPC client into each of them is
//! significantly more friction than shipping JSON-over-HTTP.
//!
//! ## What this module is NOT
//!
//! A new decision engine. Per `docs/specs/coverage/D09_kong_ai_gateway/
//! review-standards.md` §1.1 every handler in here is a *thin wrapper*
//! over the same primitives the gRPC adapter uses (`decision::
//! transaction::run_through_reserve`, `decision::transaction::
//! run_commit_estimated`, `decision::transaction::run_release`). There
//! is no audit row that originates from this module — every audit
//! emission flows back through `decision::transaction`.
//!
//! ## Endpoints
//!
//! | Endpoint            | Body shape                       | Maps to            |
//! |---------------------|----------------------------------|--------------------|
//! | `POST /v1/tokenize` | `{provider, model, prompt}`      | tokenizer (SLICE 3 wires) |
//! | `POST /v1/decision` | `{tenant_id, claim_estimate, …}` | `run_through_reserve` |
//! | `POST /v1/trace`    | `{reservation_id, outcome, …}`   | `run_commit_estimated` / `run_release` |
//!
//! ## Default posture (review-standards §1.4, §2.8, §1.6)
//!
//! * mTLS-only — no plaintext listener, ever.
//! * Loopback bind by default (`127.0.0.1`). Binding `0.0.0.0` requires
//!   an explicit `http_companion_allow_pod_network` flag AND emits a
//!   startup log so the deviation is auditable.
//! * Fail-closed default. Decision-path errors return 503 on the wire;
//!   the Kong plugin (D09 SLICE 3) translates 503 into its own
//!   fail-closed `kong.response.exit(503)` unless the operator has
//!   flipped `fail_open: true` on the plugin side.
//! * Body-size cap (4 MiB) enforced at the axum extractor level
//!   (review-standards §2.4).
//!
//! ## Test seam
//!
//! The router is wired through the [`DecisionService`] trait so unit
//! tests can verify routing, mTLS, loopback enforcement, and JSON
//! schema validation without standing up a real ledger / canonical
//! ingest / contract bundle. SLICE 3 introduces the production impl
//! that delegates to `decision::transaction::run_through_reserve`.

pub mod cidr;
pub mod handlers;
pub mod mtls;
pub mod service;

// SLICE 1: `test_support` ships behind the
// `http-companion-test-support` feature so release builds never link
// `rcgen`. Unit tests inside the lib enable the feature explicitly
// when run via `cargo test --features http-companion-test-support`.
#[cfg(feature = "http-companion-test-support")]
pub mod test_support;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use axum::{extract::DefaultBodyLimit, routing::post, Router};
use tracing::{info, warn};

pub use handlers::{
    DecisionRequest as HttpDecisionRequest, DecisionResponse as HttpDecisionResponse,
    DecisionVerdict, TokenizeRequest, TokenizeResponse, TraceAck, TraceRequest, TraceVerdict,
};
pub use service::{DecisionService, DecisionServiceError, NoopDecisionService};

/// Per-listener runtime configuration. Populated from
/// [`crate::config::Config`] in `main.rs` when the companion is enabled.
#[derive(Clone)]
pub struct HttpCompanionConfig {
    /// `(host, port)`. `port == 0` means "do not start the listener" and
    /// callers in `main.rs` guard on that explicitly before invoking
    /// [`run_companion`]. Host MUST be `127.0.0.1` or `::1` unless
    /// `allow_pod_network` is true (`bind_listener` enforces this and
    /// returns an error otherwise).
    pub host: String,
    pub port: u16,
    /// Loopback override. Per review-standards §2.8 + §1.4 even with
    /// this flag mTLS is mandatory.
    pub allow_pod_network: bool,
    /// `DefaultBodyLimit` for axum. Defends against body-streaming DoS.
    pub max_body_bytes: usize,
    /// mTLS server config (cert, key, client CA roots). Built by the
    /// caller via [`mtls::ServerTlsConfig::from_pem_files`].
    pub tls: Arc<mtls::ServerTlsConfig>,
}

/// Build the axum router with the supplied [`DecisionService`].
///
/// Exposed `pub(crate)` so the in-tree unit tests can spin up a
/// router; integration tests under `services/sidecar/tests/` reach it
/// via [`build_router_for_tests`]. Production callers should use
/// [`run_companion`].
pub(crate) fn build_router<S>(service: Arc<S>, max_body_bytes: usize) -> Router
where
    S: DecisionService + 'static,
{
    Router::new()
        .route("/v1/tokenize", post(handlers::tokenize_handler::<S>))
        .route("/v1/decision", post(handlers::decision_handler::<S>))
        .route("/v1/trace", post(handlers::trace_handler::<S>))
        .layer(DefaultBodyLimit::max(max_body_bytes))
        .with_state(service)
}

/// `cfg(test)` re-export of [`build_router`] for integration tests
/// living in `services/sidecar/tests/`. Lets the test file drop the
/// listener inside a `tokio::spawn` without depending on the runtime
/// bind helper [`run_companion`].
#[cfg(any(test, feature = "http-companion-test-support"))]
pub fn build_router_for_tests<S>(service: Arc<S>, max_body_bytes: usize) -> Router
where
    S: DecisionService + 'static,
{
    build_router(service, max_body_bytes)
}

/// Start the listener. Returns when the listener exits (typically on
/// shutdown). `main.rs` invokes this inside a `tokio::spawn` so the
/// existing UDS server keeps running concurrently.
pub async fn run_companion<S>(cfg: HttpCompanionConfig, service: Arc<S>) -> Result<()>
where
    S: DecisionService + 'static,
{
    if cfg.port == 0 {
        return Err(anyhow!("http_companion_port==0; refusing to start"));
    }

    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port)
        .parse()
        .with_context(|| format!("invalid http_companion bind '{}:{}'", cfg.host, cfg.port))?;

    if !addr.ip().is_loopback() && !cfg.allow_pod_network {
        return Err(anyhow!(
            "http_companion bind '{}' is not a loopback address; \
             set SPENDGUARD_SIDECAR_HTTP_COMPANION_ALLOW_POD_NETWORK=true \
             to expose on the pod network",
            addr
        ));
    }
    if cfg.allow_pod_network && !addr.ip().is_loopback() {
        warn!(
            addr = %addr,
            "D09 SLICE 1: http_companion pod-network exposure enabled; \
             mTLS remains mandatory but the listener is reachable beyond \
             the local pod"
        );
    }

    let router = build_router(service, cfg.max_body_bytes);

    let tcp = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind http_companion listener {addr}"))?;
    info!(
        addr = %addr,
        max_body_bytes = cfg.max_body_bytes,
        "D09 SLICE 1: http_companion listener bound (mTLS)"
    );

    mtls::serve_with_mtls(tcp, cfg.tls.clone(), router)
        .await
        .context("http_companion listener terminated")?;
    Ok(())
}

#[cfg(all(test, feature = "http-companion-test-support"))]
mod tests {
    //! Pure-unit coverage of the loopback / pod-network gate. The wire
    //! tests live in `tests/http_companion_test.rs` so they can drive
    //! real reqwest clients against the running listener.
    //!
    //! Gated on `http-companion-test-support` because the placeholder
    //! TLS config calls into the optional rcgen dep.

    use super::*;
    use std::sync::Arc;

    fn install_test_crypto_provider() {
        // rustls::ServerConfig::builder() panics if no global
        // CryptoProvider is installed (the test process bypasses the
        // main.rs install_default call). Best-effort install: ignore
        // the error returned when another test already installed it.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }

    fn dummy_tls() -> Arc<mtls::ServerTlsConfig> {
        install_test_crypto_provider();
        Arc::new(mtls::ServerTlsConfig::placeholder_for_tests())
    }

    #[tokio::test]
    async fn rejects_zero_port() {
        let cfg = HttpCompanionConfig {
            host: "127.0.0.1".into(),
            port: 0,
            allow_pod_network: false,
            max_body_bytes: 4096,
            tls: dummy_tls(),
        };
        let svc = Arc::new(NoopDecisionService::default());
        let err = run_companion(cfg, svc).await.unwrap_err();
        assert!(format!("{err}").contains("port==0"));
    }

    #[tokio::test]
    async fn rejects_non_loopback_without_flag() {
        let cfg = HttpCompanionConfig {
            host: "0.0.0.0".into(),
            port: 18443,
            allow_pod_network: false,
            max_body_bytes: 4096,
            tls: dummy_tls(),
        };
        let svc = Arc::new(NoopDecisionService::default());
        let err = run_companion(cfg, svc).await.unwrap_err();
        assert!(
            format!("{err}").contains("loopback"),
            "expected loopback rejection, got: {err}"
        );
    }
}
