//! Versioned catalog object endpoint.
//!
//! GET /v1/catalog/{version_id}
//!   Cache-Control: max-age=86400, immutable (Stage 2 §8.2.4)
//!   Returns the immutable catalog object.

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::server::AppState;

const VERSION_ID_PATTERN_DESC: &str =
    "ctlg-<rfc3339-utc>-rev<int>";

pub async fn get_catalog(
    State(state): State<AppState>,
    Path(version_id): Path<String>,
) -> Response {
    if !is_safe_version_id(&version_id) {
        return (
            StatusCode::BAD_REQUEST,
            format!(
                "version_id must match {} ({})",
                VERSION_ID_PATTERN_DESC,
                "alphanumeric, ':', '-', 'T', 'Z' only"
            ),
        )
            .into_response();
    }

    let key = format!("catalogs/{}.json", version_id);
    match state.store.get(&key).await {
        Ok(Some(bytes)) => {
            let mut resp = (StatusCode::OK, bytes).into_response();
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                "application/json".parse().unwrap(),
            );
            // Catalog versions are immutable; CDN-cacheable for 24h.
            resp.headers_mut().insert(
                header::CACHE_CONTROL,
                "public, max-age=86400, immutable".parse().unwrap(),
            );
            resp
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            format!("catalog version_id {} not found", version_id),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(?e, version_id, "catalog fetch failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "catalog fetch failed").into_response()
        }
    }
}

/// Strict whitelist for catalog version_id path. Prevents path traversal /
/// arbitrary key access. The version_id format is
/// `ctlg-<rfc3339-utc>-<32-char-uuid-simple>` (publisher mints UUIDv7 simple).
fn is_safe_version_id(s: &str) -> bool {
    if s.is_empty() || s.len() > 128 {
        return false;
    }
    if !s.starts_with("ctlg-") {
        return false;
    }
    // Reject any '.' to prevent dotted segments / extension confusion.
    if s.contains('.') {
        return false;
    }
    // Reject leading/trailing '-' or ':' to disallow weird shapes.
    let first = s.chars().next().unwrap();
    let last = s.chars().last().unwrap();
    if !first.is_ascii_alphabetic() || matches!(last, '-' | ':') {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | ':' | 'T' | 'Z'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_traversal() {
        assert!(!is_safe_version_id(".."));
        assert!(!is_safe_version_id("../etc/passwd"));
        assert!(!is_safe_version_id("../../catalogs/x"));
        assert!(!is_safe_version_id("a/b"));
        assert!(!is_safe_version_id(""));
        assert!(!is_safe_version_id("ctlg-..-rev1"));
        assert!(!is_safe_version_id("nope-2026-05-07T10:00Z-x"));
    }

    #[test]
    fn accepts_canonical_version() {
        assert!(is_safe_version_id("ctlg-2026-05-07T10:00:00Z-019e0103a0aabb"));
    }
}
