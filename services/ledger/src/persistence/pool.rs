//! Postgres pool + durability config verification.

use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool, Row,
};
use tracing::{error, info, warn};

use crate::config::Config;

pub async fn connect(cfg: &Config) -> Result<PgPool, sqlx::Error> {
    let opts: PgConnectOptions = cfg.database_url.parse()?;

    // acquire_timeout bumped from 5s → 30s to match the parallel bump in
    // services/outbox_forwarder + services/ttl_sweeper (commit a539c27).
    // Ledger startup races with bundles-init + canonical-seed-init for
    // postgres connections during the demo bring-up burst; 5s wasn't
    // enough for ledger to acquire its first connection when those init
    // scripts hold connections.
    PgPoolOptions::new()
        .max_connections(cfg.db_max_connections)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .connect_with(opts)
        .await
}

/// Verify Postgres durability config matches Stage 2 §4.2 + Ledger §19.
///
/// Hard-fails startup unless either:
///   - `SPENDGUARD_LEDGER_ALLOW_UNSAFE_DURABILITY=true` is set (POC unit
///     tests / CI without a sync replica), or
///   - `synchronous_commit` is `on`/`remote_apply`/`remote_write` AND
///     `synchronous_standby_names` is non-empty AND
///     `default_transaction_isolation` is `serializable`.
///
/// The audit invariant ("no audit, no effect") cannot be honored if the
/// ReserveSet response is allowed to return before sync-replica ack.
pub async fn verify_durability_settings(pool: &PgPool) -> Result<(), DurabilityError> {
    let row = sqlx::query(
        "SELECT
            current_setting('synchronous_commit')             AS sc,
            current_setting('synchronous_standby_names', true) AS sn,
            current_setting('default_transaction_isolation')   AS iso",
    )
    .fetch_one(pool)
    .await
    .map_err(DurabilityError::Db)?;

    let sc: String = row.get("sc");
    let sn: Option<String> = row.try_get("sn").ok();
    let iso: String = row.get("iso");

    info!(
        synchronous_commit = %sc,
        synchronous_standby_names = ?sn,
        default_transaction_isolation = %iso,
        "Postgres durability settings"
    );

    let allow_unsafe = std::env::var("SPENDGUARD_LEDGER_ALLOW_UNSAFE_DURABILITY")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let mut violations = Vec::new();

    if sc != "on" && sc != "remote_apply" && sc != "remote_write" {
        violations.push(format!(
            "synchronous_commit = '{}'; expected 'on' / 'remote_apply' / 'remote_write'",
            sc
        ));
    }
    match sn.as_deref() {
        Some(s) if !s.is_empty() => {}
        _ => violations.push(
            "synchronous_standby_names is empty; expected at least 1 sync replica \
             (e.g., 'ANY 1 (replica_b, replica_c)')"
                .to_string(),
        ),
    }
    if iso != "serializable" {
        violations.push(format!(
            "default_transaction_isolation = '{}'; expected 'serializable' (Ledger §19)",
            iso
        ));
    }

    if !violations.is_empty() {
        if allow_unsafe {
            // Fail-closed gate: the unsafe-durability escape hatch is a
            // dev/CI affordance ONLY. It must never silently apply in
            // production — a single env var would otherwise flip the ledger
            // from fail-closed (refuse to start without a sync replica +
            // serializable isolation) to fail-open (ACK reserves before
            // sync-replica durability, run at READ COMMITTED). We key off the
            // existing SPENDGUARD_PROFILE (set to "production" in the
            // production Helm chart) rather than introducing a new profile
            // concept. mTLS material being configured is treated as a
            // production signal too (defense in depth), but ONLY when the
            // profile is not an explicitly-declared non-production profile —
            // the demo compose runs mTLS + this opt-out against a single
            // Postgres and must keep starting. See is_production_like.
            let profile = std::env::var("SPENDGUARD_PROFILE").unwrap_or_default();
            let mtls_configured = std::env::var("SPENDGUARD_LEDGER_TLS_CERT_PEM")
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            if is_production_like(&profile, mtls_configured) {
                error!(
                    profile = %profile,
                    mtls_configured,
                    "REFUSING SPENDGUARD_LEDGER_ALLOW_UNSAFE_DURABILITY in a \
                     production profile — the audit invariant (Stage 2 §4) \
                     cannot be waived in production"
                );
                return Err(DurabilityError::UnsafeDurabilityForbiddenInProduction {
                    profile,
                    mtls_configured,
                    violations: violations.join("; "),
                });
            }
            for v in &violations {
                warn!("UNSAFE durability acknowledged via env var: {}", v);
            }
            warn!(
                "SPENDGUARD_LEDGER_ALLOW_UNSAFE_DURABILITY=true — service will \
                 NOT honor the audit invariant under failure (Stage 2 §4)"
            );
            return Ok(());
        }
        return Err(DurabilityError::Misconfigured(violations.join("; ")));
    }

    Ok(())
}

/// Explicitly-declared NON-production profiles. A deployment that names one of
/// these is intentionally a dev/CI/demo run and may legitimately combine mTLS
/// material with the unsafe-durability opt-out (the demo compose does exactly
/// this against a single Postgres). An explicit non-production profile must
/// therefore WIN over the mTLS defense-in-depth heuristic.
const NON_PRODUCTION_PROFILES: &[&str] = &["dev", "ci", "test", "demo", "local"];

/// Whether the runtime looks like production, in which case the
/// unsafe-durability escape hatch must be refused.
///
/// Precedence (fail-closed but demo-safe):
///   1. `profile == "production"`            => production (refuse). Always.
///   2. an explicit non-production profile   => NOT production, even with mTLS.
///      Demo/CI may run mTLS + unsafe durability against a single Postgres;
///      breaking that would take down the demo-as-quality-gate.
///   3. unknown/empty profile WITH mTLS      => production-like (refuse).
///      Defense in depth: real workload certs present but the profile env was
///      forgotten almost certainly means an accidental prod-shaped deployment.
fn is_production_like(profile: &str, mtls_configured: bool) -> bool {
    if profile == "production" {
        return true;
    }
    if NON_PRODUCTION_PROFILES.contains(&profile) {
        return false;
    }
    // Unknown or empty profile: treat configured mTLS material as a
    // production signal so a forgotten profile env can't silently waive the
    // audit invariant.
    mtls_configured
}

#[derive(Debug, thiserror::Error)]
pub enum DurabilityError {
    #[error("durability misconfigured: {0}")]
    Misconfigured(String),

    #[error(
        "SPENDGUARD_LEDGER_ALLOW_UNSAFE_DURABILITY is forbidden in production \
         (profile={profile}, mtls_configured={mtls_configured}); the audit \
         invariant cannot be waived. Underlying violations: {violations}"
    )]
    UnsafeDurabilityForbiddenInProduction {
        profile: String,
        mtls_configured: bool,
        violations: String,
    },

    #[error("postgres error reading durability settings: {0}")]
    Db(#[from] sqlx::Error),
}

#[cfg(test)]
mod tests {
    use super::is_production_like;

    #[test]
    fn unsafe_durability_refused_in_production_profile() {
        // The production profile always refuses the escape hatch, with or
        // without mTLS material configured.
        assert!(is_production_like("production", false));
        assert!(is_production_like("production", true));
    }

    #[test]
    fn unsafe_durability_refused_when_mtls_configured_and_profile_unknown() {
        // mTLS material configured with NO (or an unrecognized) profile is a
        // production signal — a forgotten profile env must not silently waive
        // the audit invariant. Fail closed.
        assert!(is_production_like("", true));
        assert!(is_production_like("staging-unknown", true));
    }

    #[test]
    fn unsafe_durability_allowed_for_explicit_non_production_profiles() {
        // An explicitly-declared non-production profile WINS over the mTLS
        // heuristic: the demo compose legitimately runs mTLS + unsafe
        // durability against a single Postgres, and must keep starting.
        for profile in ["dev", "ci", "test", "demo", "local"] {
            assert!(
                !is_production_like(profile, false),
                "{profile} (no mTLS) must be allowed"
            );
            assert!(
                !is_production_like(profile, true),
                "{profile} (with mTLS, as the demo runs) must be allowed"
            );
        }
    }

    #[test]
    fn unsafe_durability_allowed_when_no_profile_and_no_mtls() {
        // Empty/unknown profile without mTLS material is a bare dev run.
        assert!(!is_production_like("", false));
        assert!(!is_production_like("staging-unknown", false));
    }
}
