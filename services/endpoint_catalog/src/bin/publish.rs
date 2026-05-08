//! `spendguard-catalog-publish` CLI: builds a new catalog version, signs a
//! fresh manifest pointer, and atomically publishes both to the store.
//!
//! Concurrency: the storage layer does NOT prevent multiple concurrent
//! publishers from clobbering one another's manifests. Operators MUST run
//! publishes through a single-writer pipeline (e.g., a serialized GHA
//! workflow with a concurrency group, or a manual lockfile). Phase 2+
//! should add If-None-Match conditional create + manifest CAS.
//!
//! Usage:
//!   SPENDGUARD_ENDPOINT_CATALOG_FILESYSTEM_ROOT=/var/lib/sg-catalog \
//!   SPENDGUARD_ENDPOINT_CATALOG_REGION=us-west-2 \
//!   SPENDGUARD_ENDPOINT_CATALOG_PUBLIC_BASE_URL=https://catalog.us-west-2.spendguard.ai \
//!   SPENDGUARD_ENDPOINT_CATALOG_SIGNING_KEY_PEM_PATH=/etc/sg/manifest.pem \
//!   SPENDGUARD_ENDPOINT_CATALOG_SIGNING_KEY_ID=key-2026-Q2 \
//!     spendguard-catalog-publish path/to/catalog-body.json
//!
//! The catalog-body.json file is the content of `catalog.schema.json`'s
//! body MINUS `catalog_version_id` (the publisher mints a UUIDv7-derived
//! version_id).

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use std::env;
use std::path::PathBuf;
use tracing::info;

use spendguard_endpoint_catalog::{
    config::PublisherConfig,
    domain::{
        manifest::{Manifest, ManifestSigningBody},
        signing::{canonical_sha256_hex, load_signing_key_pem, sign_manifest_body},
    },
    persistence::store::make_store,
};

const CATALOG_SCHEMA: &str =
    include_str!("../../../../proto/spendguard/endpoint_catalog/v1/catalog.schema.json");

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cfg = PublisherConfig::from_env().context("loading publisher config")?;
    let key = load_signing_key_pem(&cfg.signing_key_pem_path)
        .context("load signing key")?;

    let body_path: PathBuf = env::args()
        .nth(1)
        .context("usage: publish <path/to/catalog-body.json>")?
        .into();

    let body_bytes =
        std::fs::read(&body_path).with_context(|| format!("read {}", body_path.display()))?;
    let mut body_value: serde_json::Value =
        serde_json::from_slice(&body_bytes).context("parse catalog body json")?;

    // Mint a stable, monotonic, collision-free version_id from a UUIDv7.
    // UUIDv7's first 48 bits are millisecond unix time so timestamp is
    // still readable in the id; the rest is random.
    let now = Utc::now();
    let v7 = uuid::Uuid::now_v7();
    let version_id = format!(
        "ctlg-{}-{}",
        now.format("%Y-%m-%dT%H:%M:%SZ"),
        v7.simple()
    );

    if let Some(map) = body_value.as_object_mut() {
        map.insert(
            "catalog_version_id".to_string(),
            serde_json::Value::String(version_id.clone()),
        );
    } else {
        anyhow::bail!("catalog body must be a JSON object");
    }

    // Validate against catalog.schema.json before publishing.
    validate_against_schema(&body_value)?;

    // Serialize once, canonical bytes used for both hash and storage so
    // the served bytes are byte-identical to what was hashed/signed.
    let canonical_bytes = canonical_json_bytes(&body_value)?;
    let canonical_hash = canonical_sha256_hex(&body_value)?;

    let store = make_store(&cfg.storage)?;

    // 1) Write versioned immutable catalog object.
    let catalog_key = format!("catalogs/{}.json", version_id);
    store
        .put(&catalog_key, &canonical_bytes, "application/json")
        .await
        .context("put catalog")?;
    info!(version_id = %version_id, "catalog written");

    // 2) Build + sign manifest with absolute HTTPS URL.
    let issued_at = now;
    let valid_until =
        issued_at + chrono::Duration::seconds(cfg.manifest_validity_seconds as i64);

    let public_base = validate_base_url(&cfg.public_base_url, "PUBLIC_BASE_URL", true)?;
    let catalog_url = format!("{}/v1/catalog/{}", public_base, version_id);

    let body = ManifestSigningBody {
        manifest_version: "1.0.0",
        current_catalog_version_id: &version_id,
        current_catalog_url: &catalog_url,
        current_catalog_hash: &canonical_hash,
        issued_at,
        valid_until,
        signing_key_id: &cfg.signing_key_id,
        tenant_overrides: &[],
    };
    let signature = sign_manifest_body(&key, &body)?;

    let manifest = Manifest {
        manifest_version: body.manifest_version.into(),
        current_catalog_version_id: body.current_catalog_version_id.into(),
        current_catalog_url: body.current_catalog_url.into(),
        current_catalog_hash: body.current_catalog_hash.into(),
        issued_at,
        valid_until,
        signing_key_id: body.signing_key_id.into(),
        signature,
        tenant_overrides: vec![],
    };
    let manifest_bytes = serde_json::to_vec(&manifest)?;

    // 3) Atomic publish manifest pointer.
    store
        .put("manifest.json", &manifest_bytes, "application/json")
        .await
        .context("put manifest")?;
    info!(
        version_id = %version_id,
        valid_until = %valid_until,
        "manifest signed + published"
    );

    // 4) Optional: notify the running endpoint catalog HTTP server to
    //    fan out an SSE invalidation hint. notify_base_url is a BASE URL;
    //    publisher appends the internal path here, so callers MUST NOT
    //    include a path in the env var.
    if let (Some(base), Some(token)) = (&cfg.notify_base_url, &cfg.notify_token) {
        let normalized_base = validate_base_url(base, "NOTIFY_BASE_URL", true)?;
        let notify_url = format!("{}/v1/internal/notify-catalog-change", normalized_base);
        let payload = serde_json::json!({
            "current_catalog_version_id": version_id,
            "issued_at": issued_at,
        });
        let client = reqwest::Client::new();
        match client
            .post(&notify_url)
            .bearer_auth(token)
            .json(&payload)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                info!(notify_url = %notify_url, "SSE invalidation broadcast");
            }
            Ok(resp) => {
                tracing::warn!(
                    status = %resp.status(),
                    "SSE notify returned non-2xx; sidecars will fall back to pull"
                );
            }
            Err(e) => {
                tracing::warn!(err = %e, "SSE notify failed; sidecars will fall back to pull");
            }
        }
    }

    Ok(())
}

fn canonical_json_bytes(value: &serde_json::Value) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&sort_keys(value.clone()))?)
}

fn sort_keys(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = serde_json::Map::with_capacity(entries.len());
            for (k, v) in entries {
                out.insert(k, sort_keys(v));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_keys).collect()),
        other => other,
    }
}

/// Validate a base URL: requires `https://` (or `http://` if not required),
/// requires a non-empty host, rejects path / query / fragment so callers
/// cannot bury operator confusion (`https://host/foo` then auto-appended path).
/// Returns the normalized form (trimmed, without trailing '/'). Callers
/// MUST use the returned String, not the original input.
///
/// Authority validation is deliberately conservative — anything that
/// looks like `https://@`, `https://:port`, `https://user@`, or contains
/// whitespace is rejected. For richer URL parsing we'd pull in `url`,
/// but POC keeps the dependency surface small.
fn validate_base_url(s: &str, env_var_label: &str, require_https: bool) -> Result<String> {
    let raw = s.trim();
    if raw.is_empty() {
        anyhow::bail!(
            "SPENDGUARD_ENDPOINT_CATALOG_{} must be a non-empty URL",
            env_var_label
        );
    }
    if raw.chars().any(char::is_whitespace) {
        anyhow::bail!(
            "SPENDGUARD_ENDPOINT_CATALOG_{} ('{}') must not contain whitespace",
            env_var_label, raw
        );
    }

    let (scheme, rest) = if let Some(stripped) = raw.strip_prefix("https://") {
        ("https", stripped)
    } else if let Some(stripped) = raw.strip_prefix("http://") {
        if require_https {
            anyhow::bail!(
                "SPENDGUARD_ENDPOINT_CATALOG_{} must be https:// (require_https=true)",
                env_var_label
            );
        }
        ("http", stripped)
    } else {
        anyhow::bail!(
            "SPENDGUARD_ENDPOINT_CATALOG_{} must start with https:// or http://; got '{}'",
            env_var_label,
            raw
        );
    };

    // Strip optional trailing '/' so "https://host/" is acceptable; reject
    // anything beyond root.
    let authority = rest.trim_end_matches('/');
    if authority.is_empty() {
        anyhow::bail!(
            "SPENDGUARD_ENDPOINT_CATALOG_{} ('{}') must include a host",
            env_var_label,
            raw
        );
    }
    if authority.contains('/') || authority.contains('?') || authority.contains('#') {
        anyhow::bail!(
            "SPENDGUARD_ENDPOINT_CATALOG_{} ('{}') is a base URL — it MUST NOT include path / query / fragment (publisher appends path itself)",
            env_var_label,
            raw
        );
    }
    // Reject empty user / empty host shapes: `@host`, `user@`, `:port`, `host:`.
    if authority.starts_with('@') || authority.ends_with('@') {
        anyhow::bail!(
            "SPENDGUARD_ENDPOINT_CATALOG_{} ('{}') has empty userinfo or host around '@'",
            env_var_label, raw
        );
    }
    if authority.starts_with(':') || authority.ends_with(':') {
        anyhow::bail!(
            "SPENDGUARD_ENDPOINT_CATALOG_{} ('{}') has empty host or port around ':'",
            env_var_label, raw
        );
    }
    // After optional userinfo, the host portion must be non-empty.
    let host_portion = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let host_only = host_portion.split_once(':').map(|(h, _)| h).unwrap_or(host_portion);
    if host_only.is_empty() {
        anyhow::bail!(
            "SPENDGUARD_ENDPOINT_CATALOG_{} ('{}') resolves to empty host",
            env_var_label, raw
        );
    }

    Ok(format!("{}://{}", scheme, authority))
}

#[cfg(test)]
mod tests {
    use super::validate_base_url;

    fn ok(s: &str) {
        assert!(validate_base_url(s, "TEST", true).is_ok(), "expected ok: {}", s);
    }
    fn err(s: &str) {
        assert!(validate_base_url(s, "TEST", true).is_err(), "expected err: {}", s);
    }

    #[test]
    fn validate_base_url_cases() {
        ok("https://host");
        ok("https://host/");
        ok("https://host:8443");
        ok("https://user:pass@host");
        ok("  https://host  ");

        err("");
        err("https://");
        err("https:// ");
        err("https://host/path");
        err("https://host?x=1");
        err("https://host#frag");
        err("https://@host");
        err("https://user@");
        err("https://:443");
        err("https://host:");
        err("http://host");
        err("ftp://host");
        err("host");
    }
}

/// Validate the catalog body against `proto/.../catalog.schema.json`.
///
/// Uses jsonschema crate v0.20+ `validator_for` API. On any validation
/// error, returns a multi-line error listing every violation.
fn validate_against_schema(body: &serde_json::Value) -> Result<()> {
    let schema_value: serde_json::Value =
        serde_json::from_str(CATALOG_SCHEMA).context("parse catalog.schema.json")?;
    let validator = jsonschema::validator_for(&schema_value)
        .map_err(|e| anyhow!("compile catalog schema: {}", e))?;

    let errors: Vec<String> = validator
        .iter_errors(body)
        .map(|e| format!("{} at {}", e, e.instance_path))
        .collect();
    if !errors.is_empty() {
        return Err(anyhow!(
            "catalog body fails catalog.schema.json validation:\n  - {}",
            errors.join("\n  - ")
        ));
    }
    Ok(())
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}
