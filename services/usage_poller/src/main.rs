//! Phase 5 GA hardening S11: OpenAI usage poller binary.
//!
//! Background worker: leader-elected, periodic poll, idempotent
//! insert into provider_usage_records. The actual matching SP that
//! converts records into ProviderReport calls is the S10-followup;
//! this binary's job is to KEEP THE RECORDS LANDING.

use anyhow::Context;
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use spendguard_usage_poller::{
    poll_once, AnthropicClient, MockProviderClient, OpenAiClient, PollWindow, ProviderClient,
};

#[derive(Debug, Deserialize)]
struct Config {
    database_url: String,
    /// `mock` | `openai` (default `mock` for demo and CI).
    #[serde(default = "default_provider_kind")]
    provider_kind: String,
    /// Poll interval seconds.
    #[serde(default = "default_poll_interval_seconds")]
    poll_interval_seconds: u64,
    /// How far behind now() the poll window's right edge sits — gives
    /// late-arriving provider records time to land before the cursor
    /// advances past them.
    #[serde(default = "default_safety_lag_seconds")]
    safety_lag_seconds: u64,
    /// How far back from cursor to re-poll on each cycle to catch
    /// updates to events that landed near the cursor.
    #[serde(default = "default_overlap_minutes")]
    overlap_minutes: i64,
    /// OpenAI keys (only when provider_kind=openai).
    #[serde(default)]
    openai_api_key: Option<String>,
    #[serde(default)]
    openai_org_id: Option<String>,
    #[serde(default)]
    openai_project_id: Option<String>,
    /// Anthropic keys (only when provider_kind=anthropic).
    #[serde(default)]
    anthropic_api_key: Option<String>,
    #[serde(default)]
    anthropic_workspace_id: Option<String>,
}

fn default_provider_kind() -> String {
    "mock".into()
}
fn default_poll_interval_seconds() -> u64 {
    60
}
fn default_safety_lag_seconds() -> u64 {
    300
}
fn default_overlap_minutes() -> i64 {
    5
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let envfilter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,spendguard_usage_poller=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(envfilter)
        .with_target(false)
        .json()
        .init();

    let cfg: Config =
        envy::prefixed("SPENDGUARD_USAGE_POLLER_").from_env().context("loading config")?;

    info!(
        provider_kind = %cfg.provider_kind,
        poll_interval = cfg.poll_interval_seconds,
        safety_lag = cfg.safety_lag_seconds,
        overlap_minutes = cfg.overlap_minutes,
        "S11: usage poller starting"
    );

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&cfg.database_url)
        .await
        .context("connecting to Postgres")?;

    // Codex round-10 P2: explicitly enumerate provider kinds. The
    // previous catch-all fell back to mock on any unrecognized value,
    // so a typo like `opneai` would silently disable real collection
    // and emit successful-poll metrics with zero records. Cost
    // accounting then stays blind until someone notices missing data.
    let client: Arc<dyn ProviderClient> = match cfg.provider_kind.as_str() {
        "openai" => {
            let api_key = cfg
                .openai_api_key
                .clone()
                .context("SPENDGUARD_USAGE_POLLER_OPENAI_API_KEY required when provider_kind=openai")?;
            Arc::new(OpenAiClient::new(api_key, cfg.openai_org_id.clone(), cfg.openai_project_id.clone()))
        }
        "anthropic" => {
            let api_key = cfg
                .anthropic_api_key
                .clone()
                .context("SPENDGUARD_USAGE_POLLER_ANTHROPIC_API_KEY required when provider_kind=anthropic")?;
            Arc::new(AnthropicClient::new(api_key, cfg.anthropic_workspace_id.clone()))
        }
        "mock" => Arc::new(MockProviderClient::new("mock")),
        other => anyhow::bail!(
            "unknown SPENDGUARD_USAGE_POLLER_PROVIDER_KIND={other:?}; \
             expected one of: mock, openai, anthropic"
        ),
    };

    // Codex round-5 P?: in-memory cursor with `now - safety_lag` seed
    // would lose every record landed during a restart longer than
    // safety_lag. The S11-followup persistent `provider_usage_poller_state`
    // table is still pending; until it ships, seed the cursor from
    // `MAX(observed_at) WHERE provider = ...` so a restart resumes
    // from where the last batch landed. ON CONFLICT on the insert
    // path makes wider re-polls idempotent, so an over-conservative
    // cursor is safe; an under-conservative one drops data.
    let max_observed: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT MAX(observed_at) FROM provider_usage_records WHERE provider = $1",
    )
    .bind(&cfg.provider_kind)
    .fetch_one(&pool)
    .await
    .unwrap_or(None);
    let now = chrono::Utc::now();
    let safety_lag_seed = now - chrono::Duration::seconds(cfg.safety_lag_seconds as i64);
    let mut cursor = match max_observed {
        Some(t) => {
            info!(
                seed = "max_observed",
                resume_from = %t,
                "S11: cursor resumed from prior provider_usage_records"
            );
            // Pick the older of (max_observed, now - safety_lag) so we
            // also catch records the previous instance was actively
            // polling around its termination.
            std::cmp::min(t, safety_lag_seed)
        }
        None => {
            info!(
                seed = "first_run",
                resume_from = %safety_lag_seed,
                "S11: no prior records; cursor seeded from now - safety_lag"
            );
            safety_lag_seed
        }
    };

    loop {
        let now = chrono::Utc::now();
        let window_to = now - chrono::Duration::seconds(cfg.safety_lag_seconds as i64);
        let window_from = cursor - chrono::Duration::minutes(cfg.overlap_minutes);
        let window = PollWindow {
            from: window_from,
            to: window_to,
        };

        match poll_once(&*client, &pool, window).await {
            Ok(outcome) => {
                info!(
                    fetched = outcome.fetched,
                    inserted = outcome.inserted,
                    deduped = outcome.deduped,
                    "S11: cycle ok"
                );
                cursor = window_to;
            }
            Err(e) => {
                warn!(err = %e, "S11: poll cycle failed; retaining last-success cursor");
            }
        }

        tokio::time::sleep(Duration::from_secs(cfg.poll_interval_seconds)).await;
    }
}
