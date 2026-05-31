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

#[derive(Debug, Default)]
pub struct CountTokensQuota {
    windows: Mutex<HashMap<QuotaKey, QuotaWindow>>,
}

impl CountTokensQuota {
    pub fn try_acquire(
        &self,
        tenant_id: Uuid,
        provider: &'static str,
        limit_per_minute: u32,
    ) -> bool {
        if limit_per_minute == 0 {
            return false;
        }

        let now = Instant::now();
        let mut windows = self.windows.lock();
        windows.retain(|_, window| now.duration_since(window.started_at) < WINDOW);

        let key = QuotaKey {
            tenant_id,
            provider,
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
            return false;
        }
        window.used += 1;
        true
    }
}

const WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct QuotaKey {
    tenant_id: Uuid,
    provider: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct QuotaWindow {
    started_at: Instant,
    used: u32,
}

pub type DynShadowSecurityStore = Arc<dyn ShadowSecurityStore>;
