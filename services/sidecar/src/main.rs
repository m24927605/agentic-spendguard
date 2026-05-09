use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use tonic::transport::Server;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use spendguard_sidecar::{
    bootstrap::{catalog, trust},
    clients::{
        canonical_ingest::CanonicalIngestClient, ledger::LedgerClient, mtls::MTlsPaths,
    },
    config::Config,
    domain::state::SidecarState,
    drain,
    proto::sidecar_adapter::v1::sidecar_adapter_server::SidecarAdapterServer,
    server::adapter_uds,
};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cfg = Config::from_env().context("loading sidecar config")?;
    info!(
        tenant = %cfg.tenant_id,
        workload = %cfg.workload_instance_id,
        region = %cfg.region,
        capability = %cfg.capability_level,
        enforcement = %cfg.enforcement_strength,
        "starting spendguard-sidecar"
    );

    // 1) Trust bootstrap — verify the Helm-pinned root CA SPKI hash.
    trust::verify_root_ca_pin(&cfg.trust_root_ca_pem, &cfg.trust_root_spki_sha256_hex)
        .context("trust bootstrap: root CA pin verification")?;
    info!("trust bootstrap: root CA pin verified");

    // 2) Build mTLS clients (cert-manager external issuer is expected to
    //    have populated /var/run/secrets/spendguard/{tls.crt,tls.key,ca.crt}
    //    before the sidecar container starts; the readiness probe below
    //    waits for these files).
    let mtls = MTlsPaths::default();
    wait_for_workload_cert(&mtls).await?;

    // POC: derive endpoint URLs from env (vertical slice will read from
    // catalog after manifest verify; see below for the catalog refresh
    // background task).
    let ledger_url = std::env::var("SPENDGUARD_SIDECAR_LEDGER_URL")
        .map_err(|_| anyhow!("SPENDGUARD_SIDECAR_LEDGER_URL required"))?;
    let ci_url = std::env::var("SPENDGUARD_SIDECAR_CANONICAL_INGEST_URL")
        .map_err(|_| anyhow!("SPENDGUARD_SIDECAR_CANONICAL_INGEST_URL required"))?;

    let ledger = LedgerClient::connect(
        ledger_url.clone(),
        spendguard_sidecar::clients::ledger::default_sni(&ledger_url),
        &mtls,
    )
    .await
    .context("connect ledger")?;
    let canonical_ingest = CanonicalIngestClient::connect(
        ci_url.clone(),
        spendguard_sidecar::clients::canonical_ingest::default_sni(&ci_url),
        &mtls,
    )
    .await
    .context("connect canonical_ingest")?;

    // 2a) Bootstrap producer_sequence from ledger replay so a restart
    //     doesn't collide with previously-emitted audit_outbox rows
    //     (Stage 2 §4.3 UNIQUE per (tenant, workload_instance_id, sequence)).
    //     POC: best-effort query; on error fall back to 1 with a warning.
    let producer_sequence_start = bootstrap_producer_sequence(&ledger, &cfg).await;
    info!(start = producer_sequence_start, "producer_sequence initialized");

    let idempotency = spendguard_sidecar::decision::idempotency::IdempotencyCache::new(
        cfg.idempotency_cache_size,
        cfg.idempotency_cache_ttl_secs,
    );

    // Keep a separate handle to `ledger` for the fencing-lease loop below
    // (S4): SidecarState owns one copy, the lease acquire/renewer holds
    // another. LedgerClient wraps Arc<LedgerProtoClient<Channel>>, so the
    // clone is cheap.
    let ledger_for_lease = ledger.clone();
    let state = SidecarState::new(
        ledger,
        canonical_ingest,
        idempotency,
        producer_sequence_start,
        cfg.reservation_ttl_seconds,
    );

    // 2b) Install pre-pulled bundles from disk (Helm init container loads
    //     them; Phase 2 will pull from Bundle Registry at startup).
    install_bundles(&cfg, &state)?;
    info!("contract + schema bundles installed");

    // 2c) Phase 5 S4: acquire fencing lease via Ledger RPC.
    //     SPENDGUARD_SIDECAR_LEASE_MODE controls behavior:
    //       rpc    — call Ledger.AcquireFencingLease (production)
    //       static — install seeded values without RPC (legacy demos
    //                that pre-seed fencing_scopes with epoch=1)
    //     Default `rpc`. Static path uses fencing_initial_epoch +
    //     fencing_ttl_seconds env values like before.
    let fencing_scope_id = uuid::Uuid::parse_str(&cfg.fencing_scope_id)
        .context("parse fencing_scope_id")?;
    let lease_mode = std::env::var("SPENDGUARD_SIDECAR_LEASE_MODE")
        .unwrap_or_else(|_| "rpc".into());
    if lease_mode == "static" {
        spendguard_sidecar::fencing::install_active(
            &state,
            fencing_scope_id,
            cfg.fencing_initial_epoch,
            cfg.fencing_ttl_seconds,
        );
        info!(
            scope = %fencing_scope_id,
            epoch = cfg.fencing_initial_epoch,
            "fencing scope installed (lease_mode=static; legacy demo path)"
        );
    } else {
        let lease_ttl_secs: u32 = cfg
            .fencing_ttl_seconds
            .max(1)
            .min(3600) as u32;
        spendguard_sidecar::fencing::rpc_acquire(
            &state,
            &ledger_for_lease,
            fencing_scope_id,
            &cfg.tenant_id,
            &cfg.workload_instance_id,
            lease_ttl_secs,
        )
        .await
        .context("acquire fencing lease at startup (S4)")?;

        // Spawn renewer at 1/3 of TTL with 2/3 grace window.
        let renew_interval = std::time::Duration::from_secs((lease_ttl_secs / 3).max(1) as u64);
        let grace_window = std::time::Duration::from_secs((lease_ttl_secs * 2 / 3).max(2) as u64);
        spendguard_sidecar::fencing::spawn_renewer(
            state.clone(),
            ledger_for_lease,
            fencing_scope_id,
            cfg.tenant_id.clone(),
            cfg.workload_instance_id.clone(),
            lease_ttl_secs,
            renew_interval,
            grace_window,
        );
        info!(
            scope = %fencing_scope_id,
            renew_interval_ms = renew_interval.as_millis() as u64,
            grace_window_ms = grace_window.as_millis() as u64,
            "fencing scope acquired via Ledger.AcquireFencingLease (S4)"
        );
    }

    // 3) Endpoint catalog manifest verify + atomic swap.
    let manifest_signing_key = load_manifest_signing_key()
        .context("load manifest signing public key")?;
    catalog::refresh_once(&cfg, &state, &manifest_signing_key)
        .await
        .context("initial manifest refresh")?;
    info!("endpoint catalog manifest verified at startup");

    // Spawn background refresh.
    {
        let cfg = cfg.clone();
        let state = state.clone();
        let key = manifest_signing_key;
        tokio::spawn(async move {
            catalog::refresh_loop(cfg, state, key).await;
        });
    }

    // 3a) Spawn health probe HTTP server (kubelet readiness/liveness).
    {
        let health_addr = cfg.health_addr.clone();
        let state = state.clone();
        tokio::spawn(async move {
            run_health_server(health_addr, state).await;
        });
    }

    // 4) Bind UDS for the in-process adapter.
    let uds_path = PathBuf::from(&cfg.uds_path);
    if let Some(parent) = uds_path.parent() {
        tokio::fs::create_dir_all(parent).await.context("mkdir uds parent")?;
    }
    if uds_path.exists() {
        tokio::fs::remove_file(&uds_path).await.context("remove stale uds")?;
    }
    let uds_listener =
        tokio::net::UnixListener::bind(&uds_path).context("bind uds listener")?;
    let incoming = async_stream::stream! {
        loop {
            match uds_listener.accept().await {
                Ok((stream, _addr)) => yield Ok::<_, std::io::Error>(stream),
                Err(e) => yield Err(e),
            }
        }
    };
    info!(uds = %uds_path.display(), "adapter UDS listener bound");

    let svc = adapter_uds::make_service(state.clone(), cfg.clone());
    let svc = SidecarAdapterServer::new(svc);

    // 5) preStop drain: SIGTERM → mark draining → wait drain_window.
    let drain_state = state.clone();
    let drain_secs = cfg.drain_window_seconds;
    let shutdown = async move {
        let mut sigterm = match tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::terminate(),
        ) {
            Ok(s) => s,
            Err(e) => {
                error!(?e, "failed to install SIGTERM handler");
                return;
            }
        };
        sigterm.recv().await;
        drain::run_drain(drain_state, std::time::Duration::from_secs(drain_secs)).await;
    };

    Server::builder()
        .add_service(svc)
        .serve_with_incoming_shutdown(incoming, shutdown)
        .await
        .context("uds gRPC server terminated")?;

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .json()
        .init();
}

async fn wait_for_workload_cert(mtls: &MTlsPaths) -> Result<()> {
    use std::time::Duration;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while tokio::time::Instant::now() < deadline {
        if PathBuf::from(&mtls.workload_cert_pem).exists()
            && PathBuf::from(&mtls.workload_key_pem).exists()
            && PathBuf::from(&mtls.trust_ca_pem).exists()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(anyhow!(
        "timed out waiting for cert-manager to issue workload cert"
    ))
}

async fn bootstrap_producer_sequence(ledger: &LedgerClient, cfg: &Config) -> u64 {
    use spendguard_sidecar::proto::ledger::v1::ReplayAuditFromCursorRequest;
    // POC: query the ledger for our last-known producer_sequence by
    // opening a stream from offset 0 and walking until we have the max
    // (the stream is paged at limit=N, so this is best-effort).
    let req = ReplayAuditFromCursorRequest {
        tenant_id: cfg.tenant_id.clone(),
        workload_instance_id: cfg.workload_instance_id.clone(),
        fencing_epoch: cfg.fencing_initial_epoch,
        producer_sequence_after: 0,
        limit: 1024,
        fencing_scope_id: cfg.fencing_scope_id.clone(),
    };
    match ledger.replay_audit_from_cursor(req).await {
        Ok(mut stream) => {
            let mut max_seen: u64 = 0;
            while let Ok(Some(ev)) = stream.message().await {
                if ev.producer_sequence > max_seen {
                    max_seen = ev.producer_sequence;
                }
            }
            max_seen + 1
        }
        Err(e) => {
            tracing::warn!(err = %e, "replay_audit_from_cursor at startup failed; defaulting to 1");
            1
        }
    }
}

fn install_bundles(
    cfg: &Config,
    state: &SidecarState,
) -> Result<()> {
    use spendguard_sidecar::bootstrap::bundles::{
        install_contract_bundle, install_schema_bundle, load_contract_bundle, load_schema_bundle,
        BundleSource,
    };
    let source = BundleSource {
        root: PathBuf::from(&cfg.bundle_root),
    };
    let contract_id = uuid::Uuid::parse_str(&cfg.contract_bundle_id)
        .context("parse contract_bundle_id")?;
    let schema_id = uuid::Uuid::parse_str(&cfg.schema_bundle_id)
        .context("parse schema_bundle_id")?;

    let contract = load_contract_bundle(&source, contract_id, &cfg.contract_bundle_hash_hex)
        .map_err(|e| anyhow!("load contract bundle: {e}"))?;
    install_contract_bundle(state, contract);

    let schema = load_schema_bundle(&source, schema_id, &cfg.schema_bundle_canonical_version)
        .map_err(|e| anyhow!("load schema bundle: {e}"))?;
    install_schema_bundle(state, schema);

    Ok(())
}

async fn run_health_server(addr: String, state: SidecarState) {
    use std::convert::Infallible;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(addr = %addr, err = %e, "health bind failed");
            return;
        }
    };
    info!(addr = %addr, "health server listening");

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(err = %e, "health accept failed");
                continue;
            }
        };
        let io = TokioIo::new(stream);
        let state = state.clone();
        tokio::spawn(async move {
            let svc = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                let state = state.clone();
                async move {
                    let path = req.uri().path();
                    let (status, body) = match path {
                        "/healthz" => (200, "ok"),
                        "/readyz" => {
                            if state.inner.contract_bundle.read().is_some()
                                && state.inner.fencing.read().is_some()
                                && state.inner.last_manifest_verified_at.read().is_some()
                                && !state.is_draining()
                            {
                                (200, "ready")
                            } else {
                                (503, "not ready")
                            }
                        }
                        _ => (404, "not found"),
                    };
                    Ok::<_, Infallible>(
                        hyper::Response::builder()
                            .status(status)
                            .body(http_body_util::Full::new(hyper::body::Bytes::from(body)))
                            .unwrap(),
                    )
                }
            });
            if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                tracing::debug!(err = %e, "health connection ended");
            }
        });
    }
}

fn load_manifest_signing_key() -> Result<ed25519_dalek::VerifyingKey> {
    use ed25519_dalek::pkcs8::DecodePublicKey;
    let path = std::env::var("SPENDGUARD_SIDECAR_MANIFEST_VERIFY_KEY_PEM")
        .map_err(|_| anyhow!("SPENDGUARD_SIDECAR_MANIFEST_VERIFY_KEY_PEM required"))?;
    let pem = std::fs::read_to_string(&path)
        .with_context(|| format!("read manifest verify key {path}"))?;
    Ok(ed25519_dalek::VerifyingKey::from_public_key_pem(&pem)
        .context("parse pkcs8 public key")?)
}

// async_stream is needed for UDS incoming adapter.
#[allow(dead_code)]
fn _kept_for_compile_unit(_a: Arc<()>) {}
