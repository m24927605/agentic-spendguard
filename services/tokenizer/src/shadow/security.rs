//! Tenant-level security controls for Tier 1 shadow provider calls.
//!
//! HARDEN_05 closes the production blocker where the shadow worker could
//! send raw prompt text to provider `count_tokens` APIs solely because a
//! sample passed rate-gating. This module makes raw-text egress default-deny
//! per tenant and enforces a per-(tenant, provider) minute quota.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowSecuritySettings {
    pub pii_shadow_enabled: bool,
    pub count_tokens_quota_per_minute: u32,
}

impl Default for ShadowSecuritySettings {
    fn default() -> Self {
        Self {
            pii_shadow_enabled: false,
            count_tokens_quota_per_minute: 0,
        }
    }
}

#[async_trait]
pub trait ShadowSecurityStore: Send + Sync {
    async fn load_settings(&self, tenant_id: Uuid) -> anyhow::Result<ShadowSecuritySettings>;
}

#[derive(Debug, Clone)]
pub struct StaticShadowSecurityStore {
    settings: ShadowSecuritySettings,
}

impl StaticShadowSecurityStore {
    pub fn deny_all() -> Self {
        Self {
            settings: ShadowSecuritySettings::default(),
        }
    }

    pub fn allow_all_for_tests(count_tokens_quota_per_minute: u32) -> Self {
        Self {
            settings: ShadowSecuritySettings {
                pii_shadow_enabled: true,
                count_tokens_quota_per_minute,
            },
        }
    }
}

#[async_trait]
impl ShadowSecurityStore for StaticShadowSecurityStore {
    async fn load_settings(&self, _tenant_id: Uuid) -> anyhow::Result<ShadowSecuritySettings> {
        Ok(self.settings)
    }
}

#[derive(Debug, Clone)]
pub struct PgShadowSecurityStore {
    pool: PgPool,
}

impl PgShadowSecurityStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ShadowSecurityStore for PgShadowSecurityStore {
    async fn load_settings(&self, tenant_id: Uuid) -> anyhow::Result<ShadowSecuritySettings> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
            .bind(tenant_id.to_string())
            .execute(&mut *tx)
            .await?;
        let row: Option<(bool, i32)> = sqlx::query_as(
            r#"
            SELECT pii_shadow_enabled, count_tokens_quota_per_minute
              FROM tokenizer_shadow_security_settings
             WHERE tenant_id = $1
            "#,
        )
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;

        let Some((pii_shadow_enabled, quota)) = row else {
            return Ok(ShadowSecuritySettings::default());
        };
        Ok(ShadowSecuritySettings {
            pii_shadow_enabled,
            count_tokens_quota_per_minute: u32::try_from(quota).unwrap_or(0),
        })
    }
}

#[async_trait]
pub trait CountTokensQuota: Send + Sync {
    async fn try_acquire(
        &self,
        tenant_id: Uuid,
        provider: &str,
        limit_per_minute: u32,
    ) -> anyhow::Result<bool>;
}

#[derive(Debug, Default)]
pub struct LocalCountTokensQuota {
    windows: Mutex<HashMap<QuotaKey, QuotaWindow>>,
}

#[async_trait]
impl CountTokensQuota for LocalCountTokensQuota {
    async fn try_acquire(
        &self,
        tenant_id: Uuid,
        provider: &str,
        limit_per_minute: u32,
    ) -> anyhow::Result<bool> {
        if limit_per_minute == 0 {
            return Ok(false);
        }

        let now = Instant::now();
        let mut windows = self.windows.lock();
        windows.retain(|_, window| now.duration_since(window.started_at) < WINDOW);

        let key = QuotaKey {
            tenant_id,
            provider: provider.to_owned(),
        };
        let window = windows.entry(key).or_insert_with(|| QuotaWindow {
            started_at: now,
            used: 0,
        });
        if now.duration_since(window.started_at) >= WINDOW {
            window.started_at = now;
            window.used = 0;
        }
        if window.used >= limit_per_minute {
            return Ok(false);
        }
        window.used += 1;
        Ok(true)
    }
}

#[derive(Debug, Clone)]
pub struct PgCountTokensQuota {
    pool: PgPool,
}

impl PgCountTokensQuota {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CountTokensQuota for PgCountTokensQuota {
    async fn try_acquire(
        &self,
        tenant_id: Uuid,
        provider: &str,
        limit_per_minute: u32,
    ) -> anyhow::Result<bool> {
        if limit_per_minute == 0 {
            return Ok(false);
        }
        let limit = i32::try_from(limit_per_minute)
            .map_err(|_| anyhow::anyhow!("count_tokens quota exceeds i32"))?;

        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
            .bind(tenant_id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            r#"
            DELETE FROM tokenizer_count_tokens_quota_usage
             WHERE tenant_id = $1
               AND provider = $2
               AND window_start < date_trunc('minute', clock_timestamp()) - INTERVAL '5 minutes'
            "#,
        )
        .bind(tenant_id)
        .bind(provider)
        .execute(&mut *tx)
        .await?;

        let acquired: bool = sqlx::query_scalar(
            r#"
            WITH bucket AS (
                SELECT date_trunc('minute', clock_timestamp()) AS window_start
            ),
            claim AS (
                INSERT INTO tokenizer_count_tokens_quota_usage (
                    tenant_id, provider, window_start, used_count, updated_at
                )
                SELECT $1, $2, bucket.window_start, 1, clock_timestamp()
                  FROM bucket
                ON CONFLICT (tenant_id, provider, window_start) DO UPDATE
                    SET used_count = tokenizer_count_tokens_quota_usage.used_count + 1,
                        updated_at = clock_timestamp()
                  WHERE tokenizer_count_tokens_quota_usage.used_count < $3
                RETURNING used_count
            )
            SELECT EXISTS (SELECT 1 FROM claim)
            "#,
        )
        .bind(tenant_id)
        .bind(provider)
        .bind(limit)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(acquired)
    }
}

const WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct QuotaKey {
    tenant_id: Uuid,
    provider: String,
}

#[derive(Debug, Clone, Copy)]
struct QuotaWindow {
    started_at: Instant,
    used: u32,
}

pub type DynShadowSecurityStore = Arc<dyn ShadowSecurityStore>;
pub type DynCountTokensQuota = Arc<dyn CountTokensQuota>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    use sqlx::postgres::PgPoolOptions;
    use testcontainers::runners::AsyncRunner;
    use testcontainers::ImageExt;
    use testcontainers_modules::postgres::Postgres;

    async fn setup_control_plane_postgres() -> Option<PgPool> {
        let container = match Postgres::default().with_tag("16-alpine").start().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[tokenizer quota] Postgres not available: {e}");
                return None;
            }
        };
        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("postgres host port");
        let url =
            format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres?sslmode=disable");

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(10))
            .connect(&url)
            .await
            .expect("connect owner pool");

        sqlx::raw_sql(
            r#"
            DO $$
            BEGIN
                IF NOT EXISTS (
                    SELECT 1 FROM pg_roles WHERE rolname = 'control_plane_application_role'
                ) THEN
                    CREATE ROLE control_plane_application_role NOLOGIN;
                END IF;
                IF NOT EXISTS (
                    SELECT 1 FROM pg_roles WHERE rolname = 'control_plane_reader_role'
                ) THEN
                    CREATE ROLE control_plane_reader_role NOLOGIN;
                END IF;
            END $$;

            CREATE TABLE control_plane_audit_outbox (
                audit_outbox_id UUID PRIMARY KEY,
                tenant_id UUID NOT NULL,
                event_type TEXT NOT NULL
                    CONSTRAINT control_plane_audit_outbox_event_type_check
                    CHECK (event_type ~ '^spendguard\.audit\.plugin_'),
                cloudevent_payload JSONB NOT NULL,
                cloudevent_payload_signature_hex TEXT NOT NULL DEFAULT '',
                producer_sequence BIGINT NOT NULL,
                forwarded_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
            );
            "#,
        )
        .execute(&pool)
        .await
        .expect("bootstrap minimal control_plane schema");

        let migration = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../control_plane/migrations/0004_tokenizer_shadow_security_settings.sql");
        let sql = fs::read_to_string(&migration)
            .unwrap_or_else(|e| panic!("read migration {}: {e}", migration.display()));
        sqlx::raw_sql(&sql)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("apply migration {}: {e}", migration.display()));

        Box::leak(Box::new(container));
        Some(pool)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pg_count_tokens_quota_is_shared_across_workers() {
        let Some(pool) = setup_control_plane_postgres().await else {
            return;
        };
        let tenant_id =
            Uuid::parse_str("01918000-0000-7c10-8c10-0000000000b5").expect("fixed tenant uuid");
        let first_worker = PgCountTokensQuota::new(pool.clone());
        let second_worker = PgCountTokensQuota::new(pool);

        assert!(first_worker
            .try_acquire(tenant_id, "anthropic", 1)
            .await
            .expect("first quota claim"));
        assert!(
            !second_worker
                .try_acquire(tenant_id, "anthropic", 1)
                .await
                .expect("second quota claim"),
            "second worker bypassed the shared DB-backed quota"
        );
    }
}
