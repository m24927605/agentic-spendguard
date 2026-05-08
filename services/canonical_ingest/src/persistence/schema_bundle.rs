use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::error::{map_pg_error, DomainError};

#[derive(Debug, Clone)]
pub struct CachedBundle {
    pub schema_bundle_id: Uuid,
    pub schema_bundle_hash: Vec<u8>,
}

/// Look up a schema bundle by id and verify hash matches.
pub async fn lookup(
    pool: &PgPool,
    bundle_id: Uuid,
    expected_hash: &[u8],
) -> Result<Option<CachedBundle>, DomainError> {
    let row: Option<(Uuid, Vec<u8>)> = sqlx::query_as(
        "SELECT schema_bundle_id, schema_bundle_hash
           FROM schema_bundles
          WHERE schema_bundle_id = $1",
    )
    .bind(bundle_id)
    .fetch_optional(pool)
    .await
    .map_err(map_pg_error)?;

    match row {
        None => Ok(None),
        Some((id, hash)) => {
            if hash.as_slice() == expected_hash {
                Ok(Some(CachedBundle {
                    schema_bundle_id: id,
                    schema_bundle_hash: hash,
                }))
            } else {
                Err(DomainError::SchemaBundleHashMismatch(format!(
                    "id={}, expected={}, got={}",
                    id,
                    hex::encode(expected_hash),
                    hex::encode(&hash)
                )))
            }
        }
    }
}

/// Insert a newly-discovered bundle (from Bundle Registry pull). Idempotent.
pub async fn upsert(
    pool: &PgPool,
    bundle_id: Uuid,
    bundle_hash: &[u8],
    canonical_schema_version: &str,
) -> Result<(), DomainError> {
    sqlx::query(
        "INSERT INTO schema_bundles
            (schema_bundle_id, schema_bundle_hash, canonical_schema_version)
         VALUES ($1, $2, $3)
         ON CONFLICT (schema_bundle_id, schema_bundle_hash) DO NOTHING",
    )
    .bind(bundle_id)
    .bind(bundle_hash)
    .bind(canonical_schema_version)
    .execute(pool)
    .await
    .map_err(map_pg_error)?;
    Ok(())
}
