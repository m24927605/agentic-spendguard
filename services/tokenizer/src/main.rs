//! SpendGuard tokenizer gRPC service entry point.
//!
//! Spec ref `tokenizer-service-spec-v1alpha1.md` §2.1(a).
//!
//! SLICE_03 boot sequence (round-2 fix B3 update):
//!
//!   1. Install rustls aws_lc_rs crypto provider (mirrors sidecar / ledger).
//!   2. Load env config via [`spendguard_tokenizer_service::config::Config`].
//!   3. Construct the in-process tokenizer (eager-loads encoder
//!      assets + verifies sha256 + cross-check vectors per spec §7.4
//!      fail-fast).
//!   4. Spawn the /metrics hyper server on `metrics_addr`.
//!   5. Bind the tonic gRPC server. Two modes are supported per
//!      spec §10.1:
//!       * UDS (preferred for on-node sidecar callers — no L4 hop)
//!         when `cfg.uds_path` is set.
//!       * TCP with mTLS when `cfg.tls_cert_pem` + `cfg.tls_key_pem`
//!         + `cfg.tls_ca_pem` are all set.
//!       * TCP plaintext as a demo-only fallback.
//!      Production Helm profile fails fast if neither UDS nor mTLS
//!      is configured (charts/spendguard/templates/tokenizer.yaml).
//!      Round-2 fix M6: server-side DoS limits (concurrency,
//!      message size, window) applied on both transports.
//!   6. Block on graceful shutdown signal.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use spendguard_signing::Signer;
use spendguard_tokenizer::{EncoderBootMetric, Tokenizer};
use spendguard_tokenizer_service::{
    config::Config,
    proto::tokenizer::v1::tokenizer_server::TokenizerServer,
    server::TokenizerSvc,
    shadow::{
        circuit_breaker::{CircuitBreakerConfig, CircuitBreakerState},
        persistence::SqlSamplePersister,
        provider_clients::{anthropic::AnthropicClient, gemini::GeminiClient},
        sample_rate_state::{SampleRateConfig, SampleRateState, ShadowKey},
        security::{
            LocalCountTokensQuota, PgCountTokensQuota, PgShadowSecurityStore,
            StaticShadowSecurityStore,
        },
        sink::{CanonicalIngestDriftAlertSink, SinkMTlsConfig},
        worker::{
            spawn_drop_handle, spawn_shadow_worker, DriftAlertSink, ProviderRoster,
            SamplePersister, SampleRateOverrideStore, ShadowWorkerDeps, ShadowWorkerHandle,
        },
    },
};

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls aws_lc_rs default provider"))?;

    init_tracing();

    let cfg = Config::from_env().context("loading tokenizer config")?;
    info!(
        listen = %cfg.listen_addr,
        uds = ?cfg.uds_path,
        mtls = cfg.tls_cert_pem.is_some(),
        metrics = %cfg.metrics_addr,
        tier3_threshold = %cfg.tier3_alert_threshold,
        region = %cfg.region,
        "starting spendguard-tokenizer-service"
    );

    // ── Construct the library handle (fail-fast on asset mismatch). ──
    let tokenizer = match Tokenizer::new_with_embedded_assets() {
        Ok(t) => Arc::new(t),
        Err(e) => {
            error!(
                error = ?e,
                "tokenizer asset boot failed (spec §7.4 fail-fast); refusing to start"
            );
            return Err(anyhow::Error::msg(e.to_string()));
        }
    };
    info!(
        entries = tokenizer.dispatch().len(),
        "tokenizer dispatch table compiled + encoder cache eager-loaded"
    );
    let encoder_boot_metrics = Arc::new(tokenizer.encoder_boot_durations().to_vec());

    // ── Spawn the metrics hyper server (best-effort). ─────────────
    //
    // POST_GA_03 / #96: /readyz must not report ready until the gRPC
    // listener is actually bound. The metrics server starts first so
    // kubelet startup probes can observe health, but readiness stays
    // false until bind_uds/bind_tcp flips this flag after listener bind.
    let grpc_ready = Arc::new(AtomicBool::new(false));
    if !cfg.metrics_addr.is_empty() {
        let metrics_addr: SocketAddr = cfg
            .metrics_addr
            .parse()
            .with_context(|| format!("invalid metrics_addr `{}`", cfg.metrics_addr))?;
        let metrics_ready = Arc::clone(&grpc_ready);
        let boot_metrics = Arc::clone(&encoder_boot_metrics);
        tokio::spawn(async move {
            if let Err(e) = run_metrics_server(metrics_addr, metrics_ready, boot_metrics).await {
                error!(?e, "metrics server exited with error");
            }
        });
        info!(addr = %cfg.metrics_addr, "metrics endpoint bound");
    }

    // SLICE_05 R2 B1: real shadow worker boot. The shadow worker is
    // wired with:
    //   * SqlSamplePersister against `database_url` for
    //     `tokenizer_t1_samples` writes (migration 0051).
    //   * CanonicalIngestDriftAlertSink (mTLS when sink_tls_* are set)
    //     for signed `spendguard.audit.tokenizer_drift_alert.v1alpha1`
    //     CloudEvents (audit-routed; see worker.rs::DRIFT_ALERT_EVENT_TYPE).
    //   * LocalEd25519Signer reading PKCS8 PEM from
    //     `SPENDGUARD_TOKENIZER_SIGNING_KEY_PATH` (via signer_from_env).
    //   * Provider clients for whichever vendor keys are present.
    //
    // Drop-handle fallback paths:
    //   * shadow_enabled=false                              → drop-only
    //   * no Anthropic + no Gemini key                      → drop-only
    //   * canonical_ingest_url empty OR database_url empty  → drop-only
    //
    // Production Helm profile rejects the drop-only fallback under
    // shadow_enabled=true via the required-input gate
    // (charts/spendguard/templates/tokenizer.yaml).
    let shadow_handle: ShadowWorkerHandle = boot_shadow_worker(&cfg)
        .await
        .context("boot shadow worker")?;

    let svc = TokenizerSvc::new(Arc::clone(&tokenizer))
        .with_encode_timeout(std::time::Duration::from_millis(cfg.encode_timeout_ms))
        .with_encode_max_concurrent(cfg.encode_max_concurrent)
        .with_shadow_worker(shadow_handle);
    let tonic_svc = TokenizerServer::new(svc)
        // Round-2 fix M6 + Round-3 fix N3 + POST_GA_03 / #114:
        // protocol-layer cap matches the field caps in server.rs (4 MiB
        // raw_text + 4 MiB per message). Anything bigger is rejected by tonic with
        // ResourceExhausted before deserialisation; the field caps
        // are redundant defense-in-depth that also defend the
        // in-process library form. See server.rs:50 area for the
        // layered design rationale.
        .max_decoding_message_size(
            spendguard_tokenizer_service::server::TOKENIZER_REQUEST_CAP_BYTES,
        );

    // ── Bind the gRPC server. ─────────────────────────────────────
    if let Some(uds_path) = cfg.uds_path.as_deref() {
        bind_uds(uds_path, tonic_svc, Arc::clone(&grpc_ready)).await?;
    } else {
        bind_tcp(&cfg, tonic_svc, Arc::clone(&grpc_ready)).await?;
    }

    info!("spendguard-tokenizer-service shut down cleanly");
    Ok(())
}

/// R2 B1 — wire the real shadow worker when all required inputs are
/// present; otherwise return a drop-only handle that drains events
/// silently (preserves the hot-path try_send invariant under tests +
/// demo mode).
async fn boot_shadow_worker(cfg: &Config) -> Result<ShadowWorkerHandle> {
    if !cfg.shadow_enabled {
        info!("shadow_enabled=false — shadow worker started in drop-only mode");
        return Ok(spawn_drop_handle(0));
    }

    let no_provider_keys = cfg.anthropic_api_key.is_empty() && cfg.gemini_api_key.is_empty();
    if no_provider_keys {
        info!("no provider API keys configured — shadow worker in drop-only mode (demo)");
        return Ok(spawn_drop_handle(0));
    }

    if cfg.canonical_ingest_url.is_empty() || cfg.database_url.is_empty() {
        warn!(
            canonical_ingest = !cfg.canonical_ingest_url.is_empty(),
            database = !cfg.database_url.is_empty(),
            "shadow_enabled but canonical_ingest_url or database_url missing — \
             shadow worker started in drop-only mode (Helm production profile \
             rejects this fallback via required-input gate)"
        );
        return Ok(spawn_drop_handle(0));
    }

    info!(
        anthropic_api_key_present = !cfg.anthropic_api_key.is_empty(),
        gemini_api_key_present = !cfg.gemini_api_key.is_empty(),
        canonical_ingest_url = %cfg.canonical_ingest_url,
        database_url_present = !cfg.database_url.is_empty(),
        sink_mtls = cfg.sink_tls_cert_pem.is_some(),
        "booting real shadow worker"
    );

    // Signer — same env contract as the rest of the audit-producing
    // services (sidecar / canonical_ingest / ledger). The signer must
    // be ready before the canonical_ingest sink so the AppendEvents
    // envelope uses the same producer identity and key id as the
    // per-event CloudEvent signatures.
    let signer: Arc<dyn Signer> = Arc::from(
        spendguard_signing::signer_from_env("SPENDGUARD_TOKENIZER")
            .await
            .context("load Ed25519 signer for drift_alert CloudEvent signing")?,
    );

    // SQL persister.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&cfg.database_url)
        .await
        .context("connect to ledger DB for t1_samples persistence")?;
    let persister: Arc<dyn SamplePersister> = Arc::new(SqlSamplePersister::new(pool));

    // canonical_ingest sink (R2 B4 mTLS).
    let schema_bundle_ref = build_schema_bundle_ref(cfg)?;
    let mtls = build_sink_mtls_config(cfg)?;
    let alert_sink: Arc<dyn DriftAlertSink> = Arc::new(
        CanonicalIngestDriftAlertSink::connect(
            cfg.canonical_ingest_url.clone(),
            signer.producer_identity().to_string(),
            schema_bundle_ref,
            signer.key_id().to_string(),
            mtls,
        )
        .await
        .context("connect canonical_ingest for drift_alert sink")?,
    );

    // Provider roster.
    let providers = ProviderRoster {
        anthropic: if cfg.anthropic_api_key.is_empty() {
            None
        } else {
            Some(
                AnthropicClient::new(cfg.anthropic_api_key.clone())
                    .map_err(|e| anyhow::anyhow!("build Anthropic client: {e}"))?,
            )
        },
        gemini: if cfg.gemini_api_key.is_empty() {
            None
        } else {
            Some(
                GeminiClient::new(cfg.gemini_api_key.clone())
                    .map_err(|e| anyhow::anyhow!("build Gemini client: {e}"))?,
            )
        },
    };

    let event_source = if cfg.event_source_override.is_empty() {
        format!("spendguard://tokenizer-service/{}", cfg.region)
    } else {
        cfg.event_source_override.clone()
    };

    let sample_rate = SampleRateState::new(SampleRateConfig {
        default_rate: cfg.shadow_default_sample_rate,
        ..SampleRateConfig::default()
    });
    let sample_rate_overrides = build_sampling_override_store(cfg).await?;
    let security = build_shadow_security_store(cfg).await?;
    let count_tokens_quota = build_count_tokens_quota(cfg).await?;
    let circuit_breaker = CircuitBreakerState::new(CircuitBreakerConfig::default());

    let deps = ShadowWorkerDeps {
        sample_rate,
        circuit_breaker,
        providers,
        persister,
        alert_sink,
        sample_rate_overrides,
        security,
        count_tokens_quota,
        signer,
        event_source,
        channel_capacity: 1024,
    };
    let handle = spawn_shadow_worker(deps);
    info!("shadow worker spawned (real provider clients + SQL persister + canonical_ingest sink)");
    Ok(handle)
}

async fn build_shadow_security_store(
    cfg: &Config,
) -> Result<Arc<dyn spendguard_tokenizer_service::shadow::security::ShadowSecurityStore>> {
    if cfg.sampling_override_database_url.is_empty() {
        warn!(
            "tokenizer control-plane DB URL unset; raw-text shadow provider calls are default-denied"
        );
        return Ok(Arc::new(StaticShadowSecurityStore::deny_all()));
    }
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&cfg.sampling_override_database_url)
        .await
        .context("connect tokenizer shadow security control_plane DB")?;
    info!("tokenizer shadow security DB connected");
    Ok(Arc::new(PgShadowSecurityStore::new(pool)))
}

async fn build_count_tokens_quota(
    cfg: &Config,
) -> Result<Arc<dyn spendguard_tokenizer_service::shadow::security::CountTokensQuota>> {
    if cfg.sampling_override_database_url.is_empty() {
        return Ok(Arc::new(LocalCountTokensQuota::default()));
    }
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&cfg.sampling_override_database_url)
        .await
        .context("connect tokenizer count_tokens quota control_plane DB")?;
    info!("tokenizer count_tokens quota DB connected");
    Ok(Arc::new(PgCountTokensQuota::new(pool)))
}

async fn build_sampling_override_store(
    cfg: &Config,
) -> Result<Option<Arc<dyn SampleRateOverrideStore>>> {
    if cfg.sampling_override_database_url.is_empty() {
        info!(
            "tokenizer sampling override DB URL unset; using configured default sample rates only"
        );
        return Ok(None);
    }
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&cfg.sampling_override_database_url)
        .await
        .context("connect tokenizer sampling override control_plane DB")?;
    info!("tokenizer sampling override DB connected");
    Ok(Some(Arc::new(PgSamplingOverrideStore { pool })))
}

struct PgSamplingOverrideStore {
    pool: PgPool,
}

#[async_trait]
impl SampleRateOverrideStore for PgSamplingOverrideStore {
    async fn load_override(&self, key: &ShadowKey) -> Result<Option<f64>> {
        let Ok(tenant_id) = Uuid::parse_str(&key.tenant_id) else {
            return Ok(None);
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
            .bind(tenant_id.to_string())
            .execute(&mut *tx)
            .await?;
        let row: Option<(f64,)> = sqlx::query_as(
            r#"
            SELECT rate
              FROM tokenizer_sampling_rate_overrides
             WHERE tenant_id = $1 AND model = $2
            "#,
        )
        .bind(tenant_id)
        .bind(&key.model)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.map(|r| r.0))
    }
}

fn build_schema_bundle_ref(
    cfg: &Config,
) -> Result<spendguard_tokenizer_service::proto::common::v1::SchemaBundleRef> {
    if cfg.schema_bundle_id.is_empty() {
        anyhow::bail!(
            "SPENDGUARD_TOKENIZER_SCHEMA_BUNDLE_ID required when \
             SPENDGUARD_TOKENIZER_CANONICAL_INGEST_URL is set \
             (canonical_ingest rejects AppendEventsRequest without schema_bundle)"
        );
    }
    if cfg.schema_bundle_hash_hex.is_empty() {
        anyhow::bail!(
            "SPENDGUARD_TOKENIZER_SCHEMA_BUNDLE_HASH_HEX required when \
             SPENDGUARD_TOKENIZER_CANONICAL_INGEST_URL is set"
        );
    }
    let bundle_hash = hex::decode(&cfg.schema_bundle_hash_hex)
        .context("SPENDGUARD_TOKENIZER_SCHEMA_BUNDLE_HASH_HEX must be hex-encoded")?;
    Ok(
        spendguard_tokenizer_service::proto::common::v1::SchemaBundleRef {
            schema_bundle_id: cfg.schema_bundle_id.clone(),
            schema_bundle_hash: bundle_hash.into(),
            canonical_schema_version: cfg.canonical_schema_version.clone(),
        },
    )
}

/// R2 B4 — build the optional SinkMTlsConfig from cfg. All three paths
/// must be present together; partial config returns an error so we
/// fail closed against accidental plaintext.
fn build_sink_mtls_config(cfg: &Config) -> Result<Option<SinkMTlsConfig>> {
    match (
        cfg.sink_tls_cert_pem.as_deref(),
        cfg.sink_tls_key_pem.as_deref(),
        cfg.sink_tls_ca_pem.as_deref(),
    ) {
        (None, None, None) => Ok(None),
        (Some(cert), Some(key), Some(ca)) => Ok(Some(SinkMTlsConfig {
            workload_cert_pem: PathBuf::from(cert),
            workload_key_pem: PathBuf::from(key),
            trust_ca_pem: PathBuf::from(ca),
            sni_domain: cfg.sink_tls_sni.clone(),
        })),
        _ => Err(anyhow::anyhow!(
            "partial sink mTLS config: must set all of \
             sink_tls_cert_pem / sink_tls_key_pem / sink_tls_ca_pem, or none"
        )),
    }
}

/// Round-3 fix N2 (security, Major): symlink-safe stale-socket cleanup.
///
/// `Path::exists` follows symlinks; `symlink_metadata` returns metadata
/// for the link itself. Refuse to remove anything that is not a regular
/// socket file → blocks the symlink TOCTOU attack vector where an
/// adversary plants a symlink at the UDS path and the previous code
/// would `unlink(2)` the symlink target.
///
/// Branch matrix:
///   - NotFound → no-op (clean slate, expected first boot).
///   - Socket   → unlink (recover from crashed-prior-run state).
///   - Symlink  → refuse (attack).
///   - Other    → refuse (paranoia: don't blow away unrelated files).
async fn cleanup_stale_uds(path: &Path) -> Result<()> {
    use std::os::unix::fs::FileTypeExt;
    match tokio::fs::symlink_metadata(path).await {
        Ok(m) if m.file_type().is_socket() => {
            tokio::fs::remove_file(path)
                .await
                .with_context(|| format!("remove stale uds socket `{}`", path.display()))?;
            info!(uds_path = %path.display(), "removed stale uds socket");
            Ok(())
        }
        Ok(m) if m.file_type().is_symlink() => {
            anyhow::bail!(
                "uds path `{}` is a symlink; refusing to follow (symlink attack defense per round-3 N2)",
                path.display()
            );
        }
        Ok(_) => {
            anyhow::bail!(
                "uds path `{}` exists and is not a socket; refusing to overwrite",
                path.display()
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Round-2 fix B3.1: UDS bind path. Spec §10.1 hot-path — sidecar pods
/// on the same node reach the tokenizer without an L4 hop. Precedent:
/// services/sidecar/src/main.rs:262-296 adapter UDS binding.
///
/// Round-3 fix N1+N2 (security, Major):
///   - N1: After [`UnixListener::bind`] the socket inherits the
///         process umask (typically 0644 on Linux → world-readable).
///         We chmod to 0660 so only the owning UID + gid 65532 (Helm
///         `fsGroup`) can speak gRPC; any other UID on the host
///         that mounts the hostPath sees EACCES.
///   - N2: Stale-socket removal uses [`std::fs::symlink_metadata`]
///         instead of [`std::path::Path::exists`] (which follows
///         symlinks). If an attacker plants a symlink at the UDS
///         path, the previous code would `unlink(2)` the symlink
///         target — potentially anywhere on the filesystem the
///         process can write. We refuse to follow symlinks and
///         only `unlink(2)` regular socket files.
async fn bind_uds(
    uds_path: &str,
    tonic_svc: TokenizerServer<TokenizerSvc>,
    grpc_ready: Arc<AtomicBool>,
) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixListener;
    let path = Path::new(uds_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("mkdir uds parent for `{uds_path}`"))?;
    }

    // Round-3 N2: symlink-safe stale-socket cleanup.
    cleanup_stale_uds(path).await?;

    let listener =
        UnixListener::bind(path).with_context(|| format!("bind uds listener `{uds_path}`"))?;

    // Round-3 N1: socket file perms 0660. Default umask leaves the
    // socket world-readable; under hostPath mount this lets any
    // UID on the host speak gRPC. 0660 = rw for owner + group;
    // requires the caller pod to share `fsGroup: 65532` (set by
    // Helm in tokenizer.yaml pod-level securityContext).
    let perms = std::fs::Permissions::from_mode(0o660);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("set perms on uds `{}`", path.display()))?;
    info!(
        uds_path = %path.display(),
        mode = "0660",
        "tokenizer UDS socket permissions set"
    );
    grpc_ready.store(true, Ordering::Release);

    let incoming = async_stream::stream! {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => yield Ok::<_, std::io::Error>(stream),
                Err(e) => yield Err(e),
            }
        }
    };

    info!(uds = %uds_path, "binding tokenizer gRPC server (UDS, no mTLS — kernel-enforced trust)");
    Server::builder()
        .concurrency_limit_per_connection(32)
        .max_concurrent_streams(64)
        .initial_connection_window_size(8 << 20)
        .initial_stream_window_size(2 << 20)
        .add_service(tonic_svc)
        .serve_with_incoming_shutdown(incoming, shutdown_signal())
        .await
        .context("tonic UDS gRPC server failed")
}

/// TCP bind path. mTLS when cert+key+ca are all configured; plaintext
/// otherwise (with a loud warn — production Helm profile rejects this).
async fn bind_tcp(
    cfg: &Config,
    tonic_svc: TokenizerServer<TokenizerSvc>,
    grpc_ready: Arc<AtomicBool>,
) -> Result<()> {
    let listen_addr: SocketAddr = cfg
        .listen_addr
        .parse()
        .with_context(|| format!("invalid listen_addr `{}`", cfg.listen_addr))?;

    let tls = build_server_tls_config(cfg).context("loading mTLS server config")?;

    info!(
        addr = %cfg.listen_addr,
        mtls = tls.is_some(),
        "binding tokenizer gRPC server (TCP)"
    );

    let mut builder = Server::builder()
        .concurrency_limit_per_connection(32)
        .max_concurrent_streams(64)
        .initial_connection_window_size(8 << 20)
        .initial_stream_window_size(2 << 20);
    if let Some(tls_cfg) = tls {
        builder = builder
            .tls_config(tls_cfg)
            .context("apply server TLS config")?;
    } else {
        warn!(
            "tokenizer server starting WITHOUT mTLS — only acceptable in \
             POC / demo mode. Set SPENDGUARD_TOKENIZER_TLS_{{CERT,KEY,CA}}_PEM \
             for production-correct mTLS (Helm production profile rejects this)."
        );
    }

    let listener = tokio::net::TcpListener::bind(listen_addr)
        .await
        .with_context(|| format!("bind tokenizer tcp listener `{}`", cfg.listen_addr))?;
    grpc_ready.store(true, Ordering::Release);

    let incoming = async_stream::stream! {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => yield Ok::<_, std::io::Error>(stream),
                Err(e) => yield Err(e),
            }
        }
    };

    builder
        .add_service(tonic_svc)
        .serve_with_incoming_shutdown(incoming, shutdown_signal())
        .await
        .context("tonic TCP gRPC server failed")
}

/// Round-2 fix B3.2: build the server-side mTLS config when all three
/// of cert/key/ca paths are set; return None to fall back to plaintext.
/// Partial config (e.g., cert without ca) is rejected as an error to
/// fail closed against accidental production deployments missing CA
/// pinning. Precedent: services/ledger/src/main.rs:152-172.
fn build_server_tls_config(cfg: &Config) -> Result<Option<ServerTlsConfig>> {
    match (&cfg.tls_cert_pem, &cfg.tls_key_pem, &cfg.tls_ca_pem) {
        (None, None, None) => Ok(None),
        (Some(cert_path), Some(key_path), Some(ca_path)) => {
            let cert =
                std::fs::read(cert_path).with_context(|| format!("read tls cert {cert_path}"))?;
            let key =
                std::fs::read(key_path).with_context(|| format!("read tls key {key_path}"))?;
            let ca = std::fs::read(ca_path).with_context(|| format!("read tls ca {ca_path}"))?;
            Ok(Some(
                ServerTlsConfig::new()
                    .identity(Identity::from_pem(cert, key))
                    .client_ca_root(Certificate::from_pem(ca)),
            ))
        }
        _ => Err(anyhow::anyhow!(
            "partial mTLS config: must set all of tls_cert_pem / tls_key_pem / tls_ca_pem, or none"
        )),
    }
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("spendguard_tokenizer=info,info"));
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .json()
        .init();
}

/// R2 M5 + M10 + M11 — render Prometheus payload including the SLICE_05
/// shadow worker counters. Held in a separate fn so tests can exercise
/// rendering without binding a socket.
fn render_metrics(encoder_boot_metrics: &[EncoderBootMetric]) -> String {
    use spendguard_tokenizer_service::server::{
        ENCODE_CONCURRENCY_LIMITED_TOTAL, ENCODE_TIMEOUT_TOTAL, INVALID_REQUEST_ID_TOTAL,
        REQUEST_ID_V4_ACCEPTED_TOTAL,
    };
    use spendguard_tokenizer_service::shadow::worker::{
        ALERT_ONCALL_ESCALATION_TOTAL, PROVIDER_COUNT_TOKENS_SCHEMA_DRIFT, SHADOW_DROPPED_FULL,
        SHADOW_SKIPPED_CHAT_SHAPE, SHADOW_WORKER_DEAD, TOKENIZER_RATE_LIMITED_TOTAL,
    };
    use std::sync::atomic::Ordering;
    let dropped_full = SHADOW_DROPPED_FULL.load(Ordering::Relaxed);
    let worker_dead = SHADOW_WORKER_DEAD.load(Ordering::Relaxed);
    let chat_skipped = SHADOW_SKIPPED_CHAT_SHAPE.load(Ordering::Relaxed);
    let schema_drift = PROVIDER_COUNT_TOKENS_SCHEMA_DRIFT.load(Ordering::Relaxed);
    let escalations = ALERT_ONCALL_ESCALATION_TOTAL.load(Ordering::Relaxed);
    let invalid_request_ids = INVALID_REQUEST_ID_TOTAL.load(Ordering::Relaxed);
    let request_id_v4 = REQUEST_ID_V4_ACCEPTED_TOTAL.load(Ordering::Relaxed);
    let encode_timeouts = ENCODE_TIMEOUT_TOTAL.load(Ordering::Relaxed);
    let encode_concurrency_limited = ENCODE_CONCURRENCY_LIMITED_TOTAL.load(Ordering::Relaxed);
    let rate_limited = TOKENIZER_RATE_LIMITED_TOTAL.load(Ordering::Relaxed);
    let mut body = format!(
        "# HELP spendguard_tokenizer_tier3_hit_total \
         Number of Tier 3 fallback hits (spec §5.2).\n\
         # TYPE spendguard_tokenizer_tier3_hit_total counter\n\
         spendguard_tokenizer_tier3_hit_total 0\n\
         # HELP spendguard_tokenizer_total_calls Total tokenize calls.\n\
         # TYPE spendguard_tokenizer_total_calls counter\n\
         spendguard_tokenizer_total_calls 0\n\
         # HELP spendguard_tokenizer_shadow_dropped_full_total \
         Shadow events dropped because the worker channel was full (R2 M5).\n\
         # TYPE spendguard_tokenizer_shadow_dropped_full_total counter\n\
         spendguard_tokenizer_shadow_dropped_full_total {dropped_full}\n\
         # HELP spendguard_tokenizer_shadow_worker_dead_total \
         Shadow events dropped because the worker task died — drift detection offline (R2 M5).\n\
         # TYPE spendguard_tokenizer_shadow_worker_dead_total counter\n\
         spendguard_tokenizer_shadow_worker_dead_total {worker_dead}\n\
         # HELP spendguard_tokenizer_shadow_skipped_chat_shape_total \
         Shadow events skipped because the caller used the chat-shape messages array (R2 M2; SLICE-extra wires honest per-vendor chat shadowing).\n\
         # TYPE spendguard_tokenizer_shadow_skipped_chat_shape_total counter\n\
         spendguard_tokenizer_shadow_skipped_chat_shape_total {chat_skipped}\n\
         # HELP spendguard_tokenizer_provider_count_tokens_schema_drift_total \
         Provider count_tokens response failed to match the documented schema (R2 M11; spec §7 — likely vendor API drift).\n\
         # TYPE spendguard_tokenizer_provider_count_tokens_schema_drift_total counter\n\
         spendguard_tokenizer_provider_count_tokens_schema_drift_total {schema_drift}\n\
         # HELP spendguard_tokenizer_drift_alert_oncall_escalation_total \
         Per-(tenant, model) drift alert reached the on-call escalation threshold (≥3 within 1h; R2 M10).\n\
         # TYPE spendguard_tokenizer_drift_alert_oncall_escalation_total counter\n\
         spendguard_tokenizer_drift_alert_oncall_escalation_total {escalations}\n\
         # HELP spendguard_tokenizer_invalid_request_id_total \
         Caller supplied an invalid or unsupported request_id UUID version.\n\
         # TYPE spendguard_tokenizer_invalid_request_id_total counter\n\
         spendguard_tokenizer_invalid_request_id_total {invalid_request_ids}\n\
         # HELP spendguard_tokenizer_request_id_v4_accepted_total \
         Caller supplied a valid UUIDv4 request_id accepted for backward compatibility.\n\
         # TYPE spendguard_tokenizer_request_id_v4_accepted_total counter\n\
         spendguard_tokenizer_request_id_v4_accepted_total {request_id_v4}\n\
         # HELP spendguard_tokenizer_encode_timeout_total \
         Tokenize requests that exceeded the service encode timeout.\n\
         # TYPE spendguard_tokenizer_encode_timeout_total counter\n\
         spendguard_tokenizer_encode_timeout_total {encode_timeouts}\n\
         # HELP spendguard_tokenizer_encode_concurrency_limited_total \
         Tokenize requests rejected because the encode work budget was exhausted.\n\
         # TYPE spendguard_tokenizer_encode_concurrency_limited_total counter\n\
         spendguard_tokenizer_encode_concurrency_limited_total {encode_concurrency_limited}\n\
         # HELP spendguard_tokenizer_rate_limited_total \
         Tier 1 count_tokens shadow samples skipped by per-tenant quota.\n\
         # TYPE spendguard_tokenizer_rate_limited_total counter\n\
         spendguard_tokenizer_rate_limited_total{{reason=\"count_tokens_quota\"}} {rate_limited}\n",
    );
    body.push_str(
        "# HELP spendguard_tokenizer_encoder_boot_duration_ms \
         Encoder eager-load duration captured at process startup.\n\
         # TYPE spendguard_tokenizer_encoder_boot_duration_ms gauge\n",
    );
    for metric in encoder_boot_metrics {
        body.push_str(&format!(
            "spendguard_tokenizer_encoder_boot_duration_ms{{encoder=\"{}\"}} {}\n",
            prometheus_label_value(metric.encoder_name),
            metric.duration_ms
        ));
    }
    body
}

fn prometheus_label_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!("ctrl_c received — initiating graceful shutdown");
}

/// Minimal /metrics + /healthz + /readyz hyper server. Mirrors the
/// raw-hyper pattern used by services/canonical_ingest and
/// services/ledger so the chart can scrape Prometheus + run the
/// startup probe without an additional crate dependency.
async fn run_metrics_server(
    addr: SocketAddr,
    grpc_ready: Arc<AtomicBool>,
    encoder_boot_metrics: Arc<Vec<EncoderBootMetric>>,
) -> Result<()> {
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response, StatusCode};
    use hyper_util::rt::TokioIo;

    let listener = tokio::net::TcpListener::bind(addr).await?;

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let ready_for_conn = Arc::clone(&grpc_ready);
        let boot_metrics_for_conn = Arc::clone(&encoder_boot_metrics);
        tokio::spawn(async move {
            let svc = service_fn(move |req: Request<hyper::body::Incoming>| {
                let ready = Arc::clone(&ready_for_conn);
                let boot_metrics = Arc::clone(&boot_metrics_for_conn);
                async move {
                    let (status, content_type, body): (StatusCode, &str, String) =
                        match (req.method(), req.uri().path()) {
                            (&Method::GET, "/metrics") => (
                                StatusCode::OK,
                                "text/plain; version=0.0.4; charset=utf-8",
                                render_metrics(&boot_metrics),
                            ),
                            (&Method::GET, "/healthz") => (
                                StatusCode::OK,
                                "text/plain; charset=utf-8",
                                "ok".to_string(),
                            ),
                            (&Method::GET, "/readyz") => {
                                if ready.load(Ordering::Acquire) {
                                    (
                                        StatusCode::OK,
                                        "text/plain; charset=utf-8",
                                        "ready".to_string(),
                                    )
                                } else {
                                    (
                                        StatusCode::SERVICE_UNAVAILABLE,
                                        "text/plain; charset=utf-8",
                                        "not ready: grpc listener not bound".to_string(),
                                    )
                                }
                            }
                            _ => (
                                StatusCode::NOT_FOUND,
                                "text/plain; charset=utf-8",
                                "not found".to_string(),
                            ),
                        };
                    Ok::<_, std::convert::Infallible>(
                        Response::builder()
                            .status(status)
                            .header("content-type", content_type)
                            .body(Full::new(Bytes::from(body)))
                            .unwrap(),
                    )
                }
            });
            if let Err(err) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, svc)
                .await
            {
                error!(?err, "metrics conn error");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    //! Round-3 fix N2 tests: symlink-safe UDS cleanup. We exercise
    //! `cleanup_stale_uds` directly (the rest of `bind_uds` requires
    //! a tonic server runtime and is exercised end-to-end in the
    //! sidecar contract tests). Test surface:
    //!
    //!   1. NotFound → Ok(())
    //!   2. Real socket → unlinked, Ok(())
    //!   3. Symlink (dangling) → Err containing "symlink attack defense"
    //!   4. Symlink (pointing at /etc/passwd) → Err; symlink target
    //!      MUST NOT be touched (verifies we don't follow the link).
    //!   5. Regular file → Err containing "refusing to overwrite"
    //!
    //! Cases 3+4 are the actual attack defense; case 5 is paranoia.
    use super::*;
    use spendguard_tokenizer_service::proto::canonical_ingest::v1::{
        canonical_ingest_server::{CanonicalIngest, CanonicalIngestServer},
        AuditChainEvent, EventResult, QueryAuditChainRequest, VerifySchemaBundleRequest,
        VerifySchemaBundleResponse,
    };
    use std::pin::Pin;
    use tempfile::TempDir;
    use testcontainers::runners::AsyncRunner;
    use testcontainers::ImageExt;
    use testcontainers_modules::postgres::Postgres;
    use tokio::net::UnixListener;
    use tonic::{Request, Response, Status};

    #[tokio::test]
    async fn cleanup_uds_notfound_is_noop() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("does_not_exist.sock");
        cleanup_stale_uds(&path).await.expect("noop");
        assert!(!path.exists(), "noop should not create");
    }

    #[tokio::test]
    async fn cleanup_uds_real_socket_is_unlinked() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("real.sock");
        // Create + drop a listener to leave behind a stale socket
        // inode mimicking a crashed prior tokenizer run.
        let _listener = UnixListener::bind(&path).expect("bind");
        drop(_listener);
        assert!(
            path.exists(),
            "sanity: socket file should exist pre-cleanup"
        );
        cleanup_stale_uds(&path).await.expect("unlink stale socket");
        assert!(!path.exists(), "socket should be removed");
    }

    #[tokio::test]
    async fn cleanup_uds_rejects_symlink_attack() {
        // Round-3 N2 attack scenario: attacker plants a symlink at
        // the UDS path that points at a sensitive file. Naive
        // `remove_file` would follow + unlink the target. We must
        // refuse + leave the target intact.
        use std::os::unix::fs::symlink;
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("sensitive.txt");
        std::fs::write(&target, b"do-not-delete").expect("write target");
        let link_path = dir.path().join("attack.sock");
        symlink(&target, &link_path).expect("symlink");

        let err = cleanup_stale_uds(&link_path)
            .await
            .expect_err("symlink path must be rejected");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("symlink attack defense"),
            "error must mention attack defense, got: {msg}"
        );
        // Critical: the symlink target MUST still exist and be
        // untouched after the rejection.
        assert!(target.exists(), "symlink target must not be unlinked");
        assert_eq!(
            std::fs::read(&target).expect("read"),
            b"do-not-delete",
            "symlink target must not be corrupted"
        );
        // The symlink itself is still there — we did not follow it.
        assert!(link_path.is_symlink(), "symlink itself should remain");
    }

    #[tokio::test]
    async fn cleanup_uds_rejects_regular_file() {
        // Paranoia: a regular file at the UDS path means something
        // else owns it. Refuse rather than risk data loss.
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("not_a_socket.txt");
        std::fs::write(&path, b"i am a regular file").expect("write");
        let err = cleanup_stale_uds(&path)
            .await
            .expect_err("regular file must be rejected");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("refusing to overwrite"),
            "error must mention refusing to overwrite, got: {msg}"
        );
        assert!(path.exists(), "regular file must remain");
    }

    // ── R2 B1 — boot_shadow_worker conditional gates ──────────────
    fn minimal_cfg() -> Config {
        envy::prefixed("TEST_BOOTCFG_")
            .from_iter::<_, Config>(vec![(
                "TEST_BOOTCFG_LISTEN_ADDR".to_string(),
                "127.0.0.1:0".to_string(),
            )])
            .expect("config loads")
    }

    #[tokio::test]
    async fn boot_drop_only_when_shadow_disabled() {
        let mut cfg = minimal_cfg();
        cfg.shadow_enabled = false;
        cfg.anthropic_api_key = "x".into();
        cfg.gemini_api_key = "y".into();
        cfg.canonical_ingest_url = "http://localhost:1".into();
        cfg.database_url = "postgres://nowhere".into();
        // Even with everything set, shadow_enabled=false short-circuits
        // to drop-only — no DB/canonical connection attempted.
        let h = boot_shadow_worker(&cfg).await.expect("drop-only boots");
        // Try-send succeeds without a worker because spawn_drop_handle
        // drains.
        let ev = make_shadow_event();
        h.try_send(ev).expect("drop handle accepts");
    }

    #[tokio::test]
    async fn boot_drop_only_when_no_provider_keys() {
        let mut cfg = minimal_cfg();
        cfg.shadow_enabled = true;
        cfg.anthropic_api_key = "".into();
        cfg.gemini_api_key = "".into();
        cfg.canonical_ingest_url = "http://localhost:1".into();
        cfg.database_url = "postgres://nowhere".into();
        let h = boot_shadow_worker(&cfg).await.expect("drop-only boots");
        let ev = make_shadow_event();
        h.try_send(ev).expect("drop handle accepts");
    }

    #[tokio::test]
    async fn boot_drop_only_when_database_url_missing() {
        let mut cfg = minimal_cfg();
        cfg.shadow_enabled = true;
        cfg.anthropic_api_key = "ant".into();
        cfg.canonical_ingest_url = "http://localhost:1".into();
        cfg.database_url = "".into();
        // Missing database_url even with enabled + key + canonical URL
        // → drop-only (no DB connection attempted).
        let h = boot_shadow_worker(&cfg).await.expect("drop-only boots");
        let ev = make_shadow_event();
        h.try_send(ev).expect("drop handle accepts");
    }

    #[tokio::test]
    async fn boot_drop_only_when_canonical_ingest_missing() {
        let mut cfg = minimal_cfg();
        cfg.shadow_enabled = true;
        cfg.anthropic_api_key = "ant".into();
        cfg.canonical_ingest_url = "".into();
        cfg.database_url = "postgres://nowhere".into();
        let h = boot_shadow_worker(&cfg).await.expect("drop-only boots");
        let ev = make_shadow_event();
        h.try_send(ev).expect("drop handle accepts");
    }

    #[derive(Clone, Default)]
    struct BootCanonicalIngest;

    #[tonic::async_trait]
    impl CanonicalIngest for BootCanonicalIngest {
        async fn verify_schema_bundle(
            &self,
            _request: Request<VerifySchemaBundleRequest>,
        ) -> Result<Response<VerifySchemaBundleResponse>, Status> {
            Err(Status::unimplemented("not used by boot_shadow_worker test"))
        }

        type QueryAuditChainStream =
            Pin<Box<dyn tokio_stream::Stream<Item = Result<AuditChainEvent, Status>> + Send>>;

        async fn query_audit_chain(
            &self,
            _request: Request<QueryAuditChainRequest>,
        ) -> Result<Response<Self::QueryAuditChainStream>, Status> {
            Ok(Response::new(Box::pin(tokio_stream::empty())))
        }

        async fn append_events(
            &self,
            request: Request<
                spendguard_tokenizer_service::proto::canonical_ingest::v1::AppendEventsRequest,
            >,
        ) -> Result<
            Response<
                spendguard_tokenizer_service::proto::canonical_ingest::v1::AppendEventsResponse,
            >,
            Status,
        > {
            let event_id = request
                .into_inner()
                .events
                .first()
                .map(|event| event.id.clone())
                .unwrap_or_default();
            Ok(Response::new(spendguard_tokenizer_service::proto::canonical_ingest::v1::AppendEventsResponse {
                results: vec![EventResult {
                    event_id,
                    status: spendguard_tokenizer_service::proto::canonical_ingest::v1::event_result::Status::Appended as i32,
                    ingest_position: None,
                    error: None,
                }],
            }))
        }
    }

    #[tokio::test]
    async fn boot_shadow_worker_real_wiring_connects_postgres_and_canonical_ingest() {
        let container = match Postgres::default().with_tag("16-alpine").start().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[tokenizer boot] Postgres not available: {e}");
                return;
            }
        };
        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("postgres host port");
        let database_url =
            format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres?sslmode=disable");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test canonical_ingest");
        let addr = listener.local_addr().expect("local addr");
        let incoming = async_stream::stream! {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => yield Ok::<_, std::io::Error>(stream),
                    Err(e) => yield Err(e),
                }
            }
        };
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(CanonicalIngestServer::new(BootCanonicalIngest))
                .serve_with_incoming(incoming)
                .await
                .expect("test canonical_ingest server");
        });

        std::env::set_var("SPENDGUARD_PROFILE", "demo");
        std::env::set_var("SPENDGUARD_TOKENIZER_SIGNING_MODE", "disabled");
        std::env::set_var(
            "SPENDGUARD_TOKENIZER_SIGNING_PRODUCER_IDENTITY",
            "tokenizer-service:boot-test",
        );

        let mut cfg = minimal_cfg();
        cfg.shadow_enabled = true;
        cfg.anthropic_api_key = "test-anthropic-key".into();
        cfg.canonical_ingest_url = format!("http://{addr}");
        cfg.database_url = database_url;
        cfg.sampling_override_database_url = String::new();
        cfg.schema_bundle_id = "22222222-2222-7222-8222-222222222222".into();
        cfg.schema_bundle_hash_hex = "ab".repeat(32);
        cfg.canonical_schema_version = "spendguard.v1alpha1".into();

        let handle = boot_shadow_worker(&cfg)
            .await
            .expect("real shadow worker boots with DB + canonical_ingest");
        handle
            .try_send(make_shadow_event())
            .expect("real worker handle accepts");

        std::env::remove_var("SPENDGUARD_TOKENIZER_SIGNING_PRODUCER_IDENTITY");
        std::env::remove_var("SPENDGUARD_TOKENIZER_SIGNING_MODE");
        std::env::remove_var("SPENDGUARD_PROFILE");
    }

    #[test]
    fn build_sink_mtls_config_all_or_none() {
        let mut cfg = minimal_cfg();
        // None set → Ok(None).
        assert!(build_sink_mtls_config(&cfg).expect("ok").is_none());
        // All three set → Ok(Some).
        cfg.sink_tls_cert_pem = Some("/tmp/cert.pem".into());
        cfg.sink_tls_key_pem = Some("/tmp/key.pem".into());
        cfg.sink_tls_ca_pem = Some("/tmp/ca.pem".into());
        let some = build_sink_mtls_config(&cfg).expect("ok");
        assert!(some.is_some());
        // Partial → Err.
        cfg.sink_tls_ca_pem = None;
        let err = build_sink_mtls_config(&cfg).expect_err("partial rejected");
        let msg = format!("{err:#}");
        assert!(msg.contains("partial sink mTLS config"), "got: {msg}");
    }

    #[test]
    fn render_metrics_contains_shadow_counters() {
        let body = render_metrics(&[]);
        // R2 M5 + M10 + M11 names are stable for chart scraping.
        assert!(body.contains("spendguard_tokenizer_shadow_dropped_full_total"));
        assert!(body.contains("spendguard_tokenizer_shadow_worker_dead_total"));
        assert!(body.contains("spendguard_tokenizer_shadow_skipped_chat_shape_total"));
        assert!(body.contains("spendguard_tokenizer_provider_count_tokens_schema_drift_total"));
        assert!(body.contains("spendguard_tokenizer_drift_alert_oncall_escalation_total"));
        assert!(body.contains("spendguard_tokenizer_invalid_request_id_total"));
        assert!(body.contains("spendguard_tokenizer_request_id_v4_accepted_total"));
        assert!(body.contains("spendguard_tokenizer_encode_timeout_total"));
        assert!(body.contains("spendguard_tokenizer_encode_concurrency_limited_total"));
        assert!(body.contains("spendguard_tokenizer_rate_limited_total"));
        assert!(body.contains("spendguard_tokenizer_encoder_boot_duration_ms"));
    }

    #[test]
    fn render_metrics_contains_encoder_boot_duration_gauge() {
        let samples = vec![EncoderBootMetric {
            encoder_name: "llama-sentencepiece",
            duration_ms: 42,
        }];
        let body = render_metrics(&samples);
        assert!(body.contains("# TYPE spendguard_tokenizer_encoder_boot_duration_ms gauge"));
        assert!(body.contains(
            "spendguard_tokenizer_encoder_boot_duration_ms{encoder=\"llama-sentencepiece\"} 42"
        ));
    }

    fn make_shadow_event() -> spendguard_tokenizer_service::shadow::worker::ShadowEvent {
        use spendguard_tokenizer::encoders::EncoderKind;
        spendguard_tokenizer_service::shadow::worker::ShadowEvent {
            tenant_id: uuid::Uuid::parse_str("01918000-0000-7c10-8c10-0000000000bb").unwrap(),
            model: "claude-3-5-sonnet-20241022".into(),
            encoder_kind: EncoderKind::Anthropic,
            t2_input_tokens: 10,
            t2_tokenizer_version_id: "01918000-0000-7c10-8c10-000000000010".into(),
            raw_text: "x".into(),
        }
    }
}
