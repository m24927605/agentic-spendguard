use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::error::{map_pg_error, DomainError};

#[derive(Debug, Clone)]
pub struct CachedBundle {
    pub schema_bundle_id: Uuid,
    pub schema_bundle_hash: Vec<u8>,
}

/// Round-4 fix M6: sha256("spendguard.v1alpha1+prediction") — the
/// placeholder bundle hash that round-2's deleted
/// services/canonical_ingest/migrations/0014_schema_bundle_prediction_v1alpha1.sql
/// INSERTed. Round-3 deleted the migration file but the row persists
/// in any cluster that ran round-2 between 2026-05-29 and 2026-05-30.
/// The schema_bundles_no_update_delete trigger forbids DELETE, so the
/// row stays forever.
///
/// We reject lookups matching this hash at the application layer as
/// defense-in-depth. The placeholder hash is reversible from a public
/// string — any attacker holding the producer signing key could
/// synthesise events that the placeholder bundle "verifies". By
/// refusing to treat it as a valid bundle here, we ensure no
/// production cluster can accidentally accept producer events signed
/// against the deprecated bundle.
///
/// Choosing code-level rejection over a migration cleanup avoids the
/// trigger-DROP→cleanup→trigger-RESTORE dance required to delete the
/// row (the schema_bundles_no_update_delete trigger forbids DELETE
/// outright). Application-side rejection works regardless of which
/// migration history the cluster has applied.
const ROUND2_PLACEHOLDER_BUNDLE_HASH: [u8; 32] = [
    0xe9, 0x22, 0x91, 0x88, 0x45, 0x8e, 0xd1, 0x2e, 0xb4, 0x97, 0x96, 0xcb, 0x23, 0x42, 0x20, 0x80,
    0xb9, 0xb6, 0x8d, 0xdf, 0x57, 0x1f, 0xc7, 0xae, 0x7d, 0xb7, 0x9b, 0xcc, 0x3b, 0xe1, 0x75, 0x76,
];

/// Look up a schema bundle by id and verify hash matches.
///
/// Round-4 fix M6: refuses to return a bundle with the deprecated
/// round-2 placeholder hash. Such a bundle was supply-chain hostile
/// (reversible from a public string + NULL cosign_verified_at) and
/// canonical_ingest MUST treat it as if it does not exist. The
/// fall-through return is `Err(SchemaBundleHashMismatch)` — same code
/// path as a genuine hash mismatch, so callers don't need to special-
/// case the rejection.
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
            // Round-4 fix M6: reject the deprecated round-2 placeholder
            // bundle BEFORE the hash match. Even if the producer's
            // expected_hash matches the stored hash, we refuse to treat
            // it as a valid bundle.
            if hash.as_slice() == ROUND2_PLACEHOLDER_BUNDLE_HASH {
                return Err(DomainError::SchemaBundleHashMismatch(format!(
                    "id={}, refusing deprecated round-2 placeholder bundle (sha256 of \"spendguard.v1alpha1+prediction\"); SLICE_06 producer slice must register a cosigned bundle row before any producer writes tag-300+ fields",
                    id
                )));
            }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_hash_constant_matches_public_string_sha256() {
        // Anchor test: if anyone "fixes" the constant without
        // understanding the round-2 history, this test fails before
        // the bundle starts silently accepting the deprecated hash.
        // sha256("spendguard.v1alpha1+prediction") computed offline:
        let expected_hex =
            "e9229188458ed12eb49796cb23422080b9b68ddf571fc7ae7db79bcc3be17576";
        let actual_hex = hex::encode(ROUND2_PLACEHOLDER_BUNDLE_HASH);
        assert_eq!(actual_hex, expected_hex);
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
