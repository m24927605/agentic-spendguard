//! SQL persister for `tokenizer_t1_samples`.
//!
//! Spec ref `tokenizer-service-spec-v1alpha1.md` §4.4 — schema lives
//! in migration `services/ledger/migrations/0051_tokenizer_t1_samples.sql`.
//!
//! ## Why a thin adapter rather than the worker holding sqlx directly
//!
//! The worker is generic over the [`super::worker::SamplePersister`]
//! trait so tests can plug in an in-memory recorder. The SQL adapter
//! lives here; SLICE-extra may add a buffered batch persister behind
//! the same trait without touching the worker.

use sqlx::PgPool;

use super::worker::{SampleRow, SamplePersister};

/// Persister that writes one row per sample directly to Postgres.
#[derive(Debug, Clone)]
pub struct SqlSamplePersister {
    pool: PgPool,
}

impl SqlSamplePersister {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl SamplePersister for SqlSamplePersister {
    async fn persist(&self, sample: SampleRow) -> Result<(), anyhow::Error> {
        // The migration declares `t2_tokenizer_version_id UUID NOT NULL`
        // — we parse the string here and surface a typed error so the
        // worker logs the failure cleanly.
        let tokenizer_version_id = uuid::Uuid::parse_str(
            sample.t2_tokenizer_version_id.as_str(),
        )
        .map_err(|e| anyhow::anyhow!(
            "parse tokenizer_version_id `{}`: {e}",
            sample.t2_tokenizer_version_id
        ))?;

        sqlx::query(
            r#"
            INSERT INTO tokenizer_t1_samples (
                sample_id,
                tenant_id,
                model,
                t1_input_tokens,
                t2_input_tokens,
                t2_tokenizer_version_id,
                drift_ratio,
                drift_alert_emitted,
                provider_request_id
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(sample.sample_id)
        .bind(sample.tenant_id)
        .bind(sample.model)
        .bind(sample.t1_input_tokens as i32)
        .bind(sample.t2_input_tokens as i32)
        .bind(tokenizer_version_id)
        .bind(sample.drift_ratio)
        .bind(sample.drift_alert_emitted)
        .bind(sample.provider_request_id)
        .execute(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("INSERT tokenizer_t1_samples: {e}"))?;
        Ok(())
    }
}
