use axum_server::tls_rustls::RustlsConfig;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

use spendguard_webhook_receiver::{
    config::Config,
    metrics::WebhookReceiverMetrics,
    persistence::sequence::{recover_max_seq, SequenceAllocator},
    server::{build_health_router, build_https_router, build_ledger_client, build_pg_pool, AppState},
};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls aws_lc_rs default provider"))?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .init();

    let config = Config::from_env()?;
    info!(
        bind_addr = %config.bind_addr,
        ledger_url = %config.ledger_url,
        "webhook-receiver starting"
    );

    // 1. Postgres pool.
    let pg = build_pg_pool(&config.database_url).await?;

    // 2. Recover producer_sequence (cold-path safety; v3 §E).
    let tenant_uuid = Uuid::parse_str(&config.tenant_id)?;
    let max_seq = recover_max_seq(&pg, tenant_uuid, &config.workload_instance_id).await?;
    let seq_start = max_seq + 1;
    info!(workload = %config.workload_instance_id, max_seq, seq_start, "producer_sequence recovered");
    let seq = SequenceAllocator::new(seq_start);

    // 3. Ledger gRPC client.
    let ledger_client = build_ledger_client(&config).await?;

    // 4. Phase 5 GA hardening S6: producer signer. Built BEFORE binding
    //    listeners so a misconfiguration crashes startup rather than
    //    serving unsigned audit events.
    let signer = std::sync::Arc::<dyn spendguard_signing::Signer>::from(
        spendguard_signing::signer_from_env("SPENDGUARD_WEBHOOK_RECEIVER")
            .map_err(|e| anyhow::anyhow!("S6: build signer: {e}"))?,
    );
    info!(
        key_id = %signer.key_id(),
        algorithm = %signer.algorithm(),
        producer = %signer.producer_identity(),
        "S6: producer signer initialized"
    );

    // 5. Shared state.
    let state = Arc::new(AppState {
        config: config.clone(),
        pg,
        ledger_client,
        seq,
        signer,
    });

    // Round-2 #11: Prometheus metrics counter store + HTTP server.
    let metrics = WebhookReceiverMetrics::new();
    if !config.metrics_addr.is_empty() {
        let metrics_addr = config.metrics_addr.clone();
        let metrics_handle = metrics.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_metrics(metrics_addr, metrics_handle).await {
                tracing::warn!(err = %e, "metrics server terminated");
            }
        });
        info!(addr = %config.metrics_addr, "metrics server bound");
    }

    // 5. Healthz HTTP server (plain HTTP).
    let health_addr = SocketAddr::from_str(&config.health_addr)?;
    let health_router = build_health_router(state.clone(), metrics.clone());
    let health_listener = tokio::net::TcpListener::bind(health_addr).await?;
    info!(addr = %health_addr, "healthz listening (HTTP)");
    let health_handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(health_listener, health_router).await {
            tracing::error!("health server error: {}", e);
        }
    });

    // 6. HTTPS server with TLS termination via demo PKI.
    let bind_addr = SocketAddr::from_str(&config.bind_addr)?;
    let tls = RustlsConfig::from_pem_file(&config.tls_server_cert, &config.tls_server_key).await?;
    let app = build_https_router(state, metrics);
    info!(addr = %bind_addr, "webhook receiver listening (HTTPS)");

    let server = axum_server::bind_rustls(bind_addr, tls).serve(app.into_make_service());

    tokio::select! {
        r = server => {
            tracing::error!("https server exited: {:?}", r);
        }
        r = health_handle => {
            tracing::error!("health server exited: {:?}", r);
        }
        _ = tokio::signal::ctrl_c() => {
            info!("ctrl-c received; shutting down");
        }
    }

    Ok(())
}

/// Round-2 #11: minimal HTTP /metrics endpoint.
async fn serve_metrics(addr: String, metrics: WebhookReceiverMetrics) -> anyhow::Result<()> {
    use std::convert::Infallible;
    use hyper::body::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Request, Response};
    use hyper_util::rt::TokioIo;
    use http_body_util::Full;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(&addr).await?;
    info!(addr = %addr, "webhook-receiver metrics listening");

    loop {
        let (stream, _peer) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let metrics = metrics.clone();
        tokio::task::spawn(async move {
            let svc = service_fn(move |req: Request<hyper::body::Incoming>| {
                let metrics = metrics.clone();
                async move {
                    let body = if req.uri().path() == "/metrics" {
                        metrics.render()
                    } else {
                        "".to_string()
                    };
                    Ok::<_, Infallible>(
                        Response::builder()
                            .header(
                                "content-type",
                                "text/plain; version=0.0.4; charset=utf-8",
                            )
                            .body(Full::new(Bytes::from(body)))
                            .unwrap(),
                    )
                }
            });
            if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                tracing::debug!(err = %e, "metrics conn closed");
            }
        });
    }
}
