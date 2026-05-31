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
//!
//! ## R2 M9 semantics — two-step alert tracking
//!
//! `persist` writes the row with `drift_alert_decided=BOOL,
//! drift_alert_emitted_at=NULL`. `mark_drift_alert_emitted` issues an
//! `UPDATE drift_alert_emitted_at = $1 WHERE sample_id = $2 AND
//! sampled_at = $3` AFTER the CloudEvent successfully lands in
//! canonical_ingest. The `sampled_at` predicate is required because
//! the table is partitioned by sampled_at (R2 M8) — without it, the
//! UPDATE would have to scan every partition.

use sqlx::PgPool;

use super::worker::{SamplePersister, SampleRow};

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
        let tokenizer_version_id = uuid::Uuid::parse_str(sample.t2_tokenizer_version_id.as_str())
            .map_err(|e| {
            anyhow::anyhow!(
                "parse tokenizer_version_id `{}`: {e}",
                sample.t2_tokenizer_version_id
            )
        })?;

        sqlx::query(
            r#"
            INSERT INTO tokenizer_t1_samples (
                sample_id,
                tenant_id,
                model,
                sampled_at,
                t1_input_tokens,
                t2_input_tokens,
                t2_tokenizer_version_id,
                drift_ratio,
                drift_alert_decided,
                provider_request_id
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(sample.sample_id)
        .bind(sample.tenant_id)
        .bind(sample.model)
        .bind(sample.sampled_at)
        .bind(sample.t1_input_tokens as i32)
        .bind(sample.t2_input_tokens as i32)
        .bind(tokenizer_version_id)
        .bind(sample.drift_ratio)
        .bind(sample.drift_alert_decided)
        .bind(sample.provider_request_id)
        .execute(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("INSERT tokenizer_t1_samples: {e}"))?;
        Ok(())
    }

    async fn mark_drift_alert_emitted(
        &self,
        sample_id: uuid::Uuid,
        sampled_at: chrono::DateTime<chrono::Utc>,
        emitted_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), anyhow::Error> {
        let result = sqlx::query(
            r#"
            UPDATE tokenizer_t1_samples
               SET drift_alert_emitted_at = $1
             WHERE sample_id   = $2
               AND sampled_at  = $3
            "#,
        )
        .bind(emitted_at)
        .bind(sample_id)
        .bind(sampled_at)
        .execute(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("UPDATE tokenizer_t1_samples emit ack: {e}"))?;

        if result.rows_affected() != 1 {
            return Err(anyhow::anyhow!(
                "mark_drift_alert_emitted: expected 1 row updated, got {} \
                 (sample_id={sample_id}, sampled_at={sampled_at})",
                result.rows_affected()
            ));
        }
        Ok(())
    }
}
