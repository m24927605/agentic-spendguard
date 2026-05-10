//! Phase 5 GA hardening S17: OIDC/SSO foundation.
//!
//! Replaces the single static admin bearer token with OIDC-based JWT
//! validation in dashboard + control_plane. Two modes:
//!
//!   * `jwt` (default for production) — incoming `Authorization: Bearer
//!     <jwt>` is validated against an OIDC issuer's JWKS. Claims map
//!     into a `Principal` (subject, issuer, groups, tenant_ids, roles).
//!   * `static_token` (demo profile only) — exact-match `Authorization:
//!     Bearer <token>` against a configured constant string. Refuses to
//!     construct unless `SPENDGUARD_PROFILE=demo` so an operator can't
//!     ship a chart with mode=static_token to a non-demo cluster.
//!
//! Tenant claim mapping. The `tenant_ids` field on `Principal` is
//! populated from a configurable JWT claim (default
//! `spendguard:tenant_ids`). Groups likewise (default `groups`). The
//! mapping is intentionally simple — S18 (RBAC) wires roles + tenant
//! scope into queries; S17 only does authentication.
//!
//! JWKS caching. Keys are fetched on first miss + refreshed every
//! `jwks_refresh_seconds` (default 3600). A failed refresh keeps the
//! existing cache (fail-open for liveness; the operator gets a metric
//! / log line) — but a cold start with unreachable JWKS hard-fails.
//!
//! Out of scope for S17 (S18 covers): per-route role enforcement,
//! tenant-scoped DB queries, audit log of mutating actions.

pub mod rbac;
pub use rbac::{GroupPolicy, Permission, Role};

use async_trait::async_trait;
use axum::{
    body::Body,
    extract::Request,
    http::{header::AUTHORIZATION, StatusCode},
    middleware::Next,
    response::Response,
};
use chrono::{DateTime, Utc};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, info, warn};

// ============================================================================
// Public types
// ============================================================================

/// Authenticated principal; placed into axum request extensions by the
/// middleware. Handlers downstream read it via
/// `Extension<Principal>`.
#[derive(Debug, Clone, Serialize)]
pub struct Principal {
    /// JWT `iss` (or `"static-token"` for static-token mode).
    pub issuer: String,
    /// JWT `sub`. For static-token mode, this is the configured
    /// `static_token_subject` value (defaults to "operator").
    pub subject: String,
    /// Groups claim (default name `groups`). Empty list when missing.
    pub groups: Vec<String>,
    /// Tenant ids claim (default name `spendguard:tenant_ids`).
    /// Empty list means "no tenant scope" — handlers must reject.
    pub tenant_ids: Vec<String>,
    /// Roles. S17 leaves this empty; S18 populates from groups +
    /// policy mapping.
    pub roles: Vec<String>,
    /// Auth mode that admitted the request (`jwt` | `static_token`).
    /// Useful for audit logs.
    pub mode: String,
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("missing or malformed Authorization header")]
    MissingAuthHeader,
    #[error("invalid JWT: {0}")]
    InvalidToken(String),
    #[error("token issuer mismatch")]
    IssuerMismatch,
    #[error("token audience mismatch")]
    AudienceMismatch,
    #[error("token expired")]
    Expired,
    #[error("unknown JWT signing key (kid={0})")]
    UnknownKid(String),
    #[error("JWKS fetch failed: {0}")]
    JwksFetch(String),
    #[error("static-token mode is only allowed when SPENDGUARD_PROFILE=demo")]
    StaticTokenOutsideDemo,
    #[error("static token mismatch")]
    StaticTokenMismatch,
    #[error("infra: {0}")]
    Infra(String),
}

impl AuthError {
    fn status_code(&self) -> StatusCode {
        match self {
            AuthError::MissingAuthHeader
            | AuthError::InvalidToken(_)
            | AuthError::IssuerMismatch
            | AuthError::AudienceMismatch
            | AuthError::Expired
            | AuthError::UnknownKid(_)
            | AuthError::StaticTokenMismatch => StatusCode::UNAUTHORIZED,
            AuthError::JwksFetch(_) | AuthError::Infra(_) => StatusCode::SERVICE_UNAVAILABLE,
            AuthError::StaticTokenOutsideDemo => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Public-safe message. NEVER reveals tenant or user existence —
    /// per the spec's "auth failures must not reveal tenant existence"
    /// requirement.
    fn safe_public_message(&self) -> &'static str {
        match self {
            AuthError::MissingAuthHeader => "missing authorization",
            AuthError::JwksFetch(_) | AuthError::Infra(_) => "service temporarily unavailable",
            AuthError::StaticTokenOutsideDemo => "auth misconfigured",
            _ => "unauthorized",
        }
    }
}

// ============================================================================
// JWT claims (deserialized from token body)
// ============================================================================

#[derive(Debug, Deserialize)]
struct JwtClaims {
    iss: String,
    sub: String,
    aud: serde_json::Value, // string or array of strings
    exp: i64,
    /// Default groups claim — operators can override the claim name
    /// via `JwtConfig::groups_claim` when their IdP uses something
    /// else (e.g. Entra: `roles`, Auth0: `https://example.com/groups`).
    #[serde(default)]
    groups: Vec<String>,
    /// SpendGuard-specific tenant scope claim. Operators populate
    /// this via Entra app role mapping or claim transformation rules.
    #[serde(rename = "spendguard:tenant_ids", default)]
    tenant_ids: Vec<String>,
}

// ============================================================================
// Config
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMode {
    Jwt,
    StaticToken,
}

impl AuthMode {
    pub fn parse(s: &str) -> Result<Self, AuthError> {
        match s {
            "jwt" => Ok(Self::Jwt),
            "static_token" => Ok(Self::StaticToken),
            other => Err(AuthError::Infra(format!(
                "unknown AUTH_MODE: {other:?} (expected jwt|static_token)"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct JwtConfig {
    pub issuer: String,
    pub audience: String,
    pub jwks_url: String,
    pub clock_skew_seconds: u64,
    pub jwks_refresh_seconds: u64,
    pub groups_claim: String,
    pub tenant_ids_claim: String,
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            issuer: String::new(),
            audience: String::new(),
            jwks_url: String::new(),
            clock_skew_seconds: 60,
            jwks_refresh_seconds: 3600,
            groups_claim: "groups".into(),
            tenant_ids_claim: "spendguard:tenant_ids".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StaticTokenConfig {
    pub token: String,
    pub subject: String,
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub kind: AuthConfigKind,
    /// Phase 5 GA hardening S18: group→roles policy applied to
    /// every authenticated principal. Empty by default
    /// (fail-closed for unconfigured prod). Demo profile defaults
    /// to `GroupPolicy::demo_default`.
    pub policy: GroupPolicy,
    /// S18: static-token principals get this tenant scope (since
    /// they have no JWT claims). Comma-separated list from the
    /// `STATIC_TOKEN_TENANT_IDS` env var. Empty list means
    /// "demo principal sees no tenants" — fail-closed.
    pub static_token_tenant_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum AuthConfigKind {
    Jwt(JwtConfig),
    StaticToken(StaticTokenConfig),
}

impl AuthConfig {
    /// Construct from prefixed env vars. Mode defaults to `jwt`.
    /// Loads S18 group→roles policy from `<PREFIX>_GROUP_POLICY_JSON`
    /// (or, in demo profile with no env override, falls back to the
    /// builtin demo policy that maps `demo-admins` to all roles).
    pub fn from_env(prefix: &str, profile: &str) -> Result<Self, AuthError> {
        let mode = std::env::var(format!("{prefix}_AUTH_MODE")).unwrap_or_else(|_| "jwt".into());
        let policy = load_policy(prefix, profile)?;
        let static_token_tenant_ids = std::env::var(format!("{prefix}_STATIC_TOKEN_TENANT_IDS"))
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let kind = match AuthMode::parse(&mode)? {
            AuthMode::Jwt => {
                let issuer = std::env::var(format!("{prefix}_OIDC_ISSUER"))
                    .map_err(|_| AuthError::Infra(format!("{prefix}_OIDC_ISSUER required")))?;
                let audience = std::env::var(format!("{prefix}_OIDC_AUDIENCE"))
                    .map_err(|_| AuthError::Infra(format!("{prefix}_OIDC_AUDIENCE required")))?;
                let jwks_url = std::env::var(format!("{prefix}_OIDC_JWKS_URL"))
                    .map_err(|_| AuthError::Infra(format!("{prefix}_OIDC_JWKS_URL required")))?;
                let clock_skew_seconds = std::env::var(format!("{prefix}_OIDC_CLOCK_SKEW_SECONDS"))
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(60);
                let jwks_refresh_seconds =
                    std::env::var(format!("{prefix}_OIDC_JWKS_REFRESH_SECONDS"))
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(3600);
                let groups_claim = std::env::var(format!("{prefix}_OIDC_GROUPS_CLAIM"))
                    .unwrap_or_else(|_| "groups".into());
                let tenant_ids_claim =
                    std::env::var(format!("{prefix}_OIDC_TENANT_IDS_CLAIM"))
                        .unwrap_or_else(|_| "spendguard:tenant_ids".into());
                AuthConfigKind::Jwt(JwtConfig {
                    issuer,
                    audience,
                    jwks_url,
                    clock_skew_seconds,
                    jwks_refresh_seconds,
                    groups_claim,
                    tenant_ids_claim,
                })
            }
            AuthMode::StaticToken => {
                if profile != "demo" {
                    return Err(AuthError::StaticTokenOutsideDemo);
                }
                let token = std::env::var(format!("{prefix}_STATIC_TOKEN")).map_err(|_| {
                    AuthError::Infra(format!("{prefix}_STATIC_TOKEN required for static_token mode"))
                })?;
                let subject = std::env::var(format!("{prefix}_STATIC_TOKEN_SUBJECT"))
                    .unwrap_or_else(|_| "operator".into());
                AuthConfigKind::StaticToken(StaticTokenConfig { token, subject })
            }
        };

        Ok(Self {
            kind,
            policy,
            static_token_tenant_ids,
        })
    }

    pub fn mode_str(&self) -> &'static str {
        match &self.kind {
            AuthConfigKind::Jwt(_) => "jwt",
            AuthConfigKind::StaticToken(_) => "static_token",
        }
    }
}

fn load_policy(prefix: &str, profile: &str) -> Result<GroupPolicy, AuthError> {
    if let Ok(raw) = std::env::var(format!("{prefix}_GROUP_POLICY_JSON")) {
        return GroupPolicy::parse_json(&raw)
            .map_err(|e| AuthError::Infra(format!("group policy: {e}")));
    }
    if profile == "demo" {
        Ok(GroupPolicy::demo_default())
    } else {
        // Production with no policy configured: empty mapping +
        // no fallback. Every authenticated principal will have
        // roles=empty and every permission check will deny. This
        // is the correct fail-closed default — operators must
        // configure GROUP_POLICY_JSON before enabling auth in
        // production.
        Ok(GroupPolicy::empty())
    }
}

// ============================================================================
// JWKS cache
// ============================================================================

/// In-memory cache of JWKS keys keyed by `kid`. Refreshed at
/// `jwks_refresh_seconds` cadence on first access after expiry.
#[async_trait]
pub trait JwksProvider: Send + Sync {
    async fn key_for(&self, kid: &str) -> Result<DecodingKey, AuthError>;
}

pub struct HttpJwksProvider {
    cfg: JwtConfig,
    inner: Arc<RwLock<JwksState>>,
    client: reqwest::Client,
}

struct JwksState {
    keys: std::collections::HashMap<String, DecodingKey>,
    fetched_at: Option<DateTime<Utc>>,
}

impl HttpJwksProvider {
    pub fn new(cfg: JwtConfig) -> Self {
        Self {
            cfg,
            inner: Arc::new(RwLock::new(JwksState {
                keys: std::collections::HashMap::new(),
                fetched_at: None,
            })),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
        }
    }

    async fn refresh_if_stale(&self) -> Result<(), AuthError> {
        let needs_refresh = {
            let s = self.inner.read();
            match s.fetched_at {
                None => true,
                Some(t) => {
                    (Utc::now() - t).num_seconds() > self.cfg.jwks_refresh_seconds as i64
                }
            }
        };
        if !needs_refresh {
            return Ok(());
        }
        match self.fetch().await {
            Ok(new_keys) => {
                let mut s = self.inner.write();
                s.keys = new_keys;
                s.fetched_at = Some(Utc::now());
                debug!(
                    keys = s.keys.len(),
                    url = %self.cfg.jwks_url,
                    "JWKS refresh ok"
                );
                Ok(())
            }
            Err(e) => {
                let s = self.inner.read();
                if s.fetched_at.is_none() {
                    return Err(e);
                }
                warn!(error = %e, "JWKS refresh failed; serving stale cache");
                Ok(())
            }
        }
    }

    async fn fetch(&self) -> Result<std::collections::HashMap<String, DecodingKey>, AuthError> {
        let resp = self
            .client
            .get(&self.cfg.jwks_url)
            .send()
            .await
            .map_err(|e| AuthError::JwksFetch(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(AuthError::JwksFetch(format!(
                "JWKS HTTP {} from {}",
                status, self.cfg.jwks_url
            )));
        }
        let body: JwksDoc = resp
            .json()
            .await
            .map_err(|e| AuthError::JwksFetch(format!("decode: {e}")))?;
        let mut out = std::collections::HashMap::new();
        for k in body.keys {
            if let Some(dk) = jwk_to_decoding_key(&k) {
                out.insert(k.kid, dk);
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl JwksProvider for HttpJwksProvider {
    async fn key_for(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        self.refresh_if_stale().await?;
        let s = self.inner.read();
        s.keys
            .get(kid)
            .cloned()
            .ok_or_else(|| AuthError::UnknownKid(kid.to_string()))
    }
}

#[derive(Deserialize)]
struct JwksDoc {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
struct Jwk {
    kid: String,
    kty: String,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
    #[serde(default)]
    x: Option<String>,
    #[serde(default)]
    y: Option<String>,
    #[serde(default)]
    crv: Option<String>,
}

fn jwk_to_decoding_key(k: &Jwk) -> Option<DecodingKey> {
    match k.kty.as_str() {
        "RSA" => {
            let n = k.n.as_deref()?;
            let e = k.e.as_deref()?;
            DecodingKey::from_rsa_components(n, e).ok()
        }
        "EC" => {
            let x = k.x.as_deref()?;
            let y = k.y.as_deref()?;
            DecodingKey::from_ec_components(x, y).ok()
        }
        _ => None,
    }
}

// ============================================================================
// Validator (JWT mode)
// ============================================================================

pub struct JwtValidator {
    cfg: JwtConfig,
    jwks: Arc<dyn JwksProvider>,
}

impl JwtValidator {
    pub fn new(cfg: JwtConfig) -> Self {
        let jwks: Arc<dyn JwksProvider> = Arc::new(HttpJwksProvider::new(cfg.clone()));
        Self { cfg, jwks }
    }

    /// Test-only: inject a custom JwksProvider.
    #[doc(hidden)]
    pub fn with_jwks(cfg: JwtConfig, jwks: Arc<dyn JwksProvider>) -> Self {
        Self { cfg, jwks }
    }

    pub async fn validate(&self, token: &str) -> Result<Principal, AuthError> {
        let header = decode_header(token).map_err(|e| AuthError::InvalidToken(e.to_string()))?;
        let kid = header
            .kid
            .ok_or_else(|| AuthError::InvalidToken("missing kid".into()))?;
        let key = self.jwks.key_for(&kid).await?;

        let mut validation = Validation::new(detect_alg(&header.alg));
        validation.set_audience(&[self.cfg.audience.clone()]);
        validation.set_issuer(&[self.cfg.issuer.clone()]);
        validation.leeway = self.cfg.clock_skew_seconds;

        let data = decode::<serde_json::Value>(token, &key, &validation).map_err(|e| {
            // jsonwebtoken collapses the failure mode into a single
            // error type — translate into our typed reasons.
            use jsonwebtoken::errors::ErrorKind;
            match e.kind() {
                ErrorKind::InvalidAudience => AuthError::AudienceMismatch,
                ErrorKind::InvalidIssuer => AuthError::IssuerMismatch,
                ErrorKind::ExpiredSignature => AuthError::Expired,
                _ => AuthError::InvalidToken(e.to_string()),
            }
        })?;

        let raw = data.claims;
        let claims: JwtClaims = serde_json::from_value(raw.clone())
            .map_err(|e| AuthError::InvalidToken(format!("claims shape: {e}")))?;

        // Custom claim names if operator overrode defaults.
        let groups = if self.cfg.groups_claim == "groups" {
            claims.groups
        } else {
            extract_string_array(&raw, &self.cfg.groups_claim)
        };
        let tenant_ids = if self.cfg.tenant_ids_claim == "spendguard:tenant_ids" {
            claims.tenant_ids
        } else {
            extract_string_array(&raw, &self.cfg.tenant_ids_claim)
        };

        Ok(Principal {
            issuer: claims.iss,
            subject: claims.sub,
            groups,
            tenant_ids,
            roles: Vec::new(),
            mode: "jwt".into(),
        })
    }
}

fn detect_alg(alg: &Algorithm) -> Algorithm {
    *alg
}

fn extract_string_array(claims: &serde_json::Value, name: &str) -> Vec<String> {
    claims
        .get(name)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

// ============================================================================
// Validator dispatch (jwt or static_token)
// ============================================================================

pub struct Authenticator {
    inner: AuthenticatorInner,
    policy: Arc<GroupPolicy>,
    /// Tenant scope applied to static-token principals (since they
    /// have no JWT claims). Empty list = no tenant scope =
    /// fail-closed under `assert_tenant`. Operators set this only
    /// in demo profile.
    static_token_tenant_ids: Vec<String>,
}

enum AuthenticatorInner {
    Jwt(Arc<JwtValidator>),
    StaticToken(Arc<StaticTokenConfig>),
}

impl Authenticator {
    pub async fn from_config(cfg: AuthConfig) -> Result<Self, AuthError> {
        let policy = Arc::new(cfg.policy);
        let inner = match cfg.kind {
            AuthConfigKind::Jwt(j) => {
                info!(
                    mode = "jwt",
                    issuer = %j.issuer,
                    audience = %j.audience,
                    jwks_url = %j.jwks_url,
                    policy_groups = policy.mapping.len(),
                    "auth initialized"
                );
                AuthenticatorInner::Jwt(Arc::new(JwtValidator::new(j)))
            }
            AuthConfigKind::StaticToken(s) => {
                warn!(
                    subject = %s.subject,
                    policy_groups = policy.mapping.len(),
                    static_tenant_count = cfg.static_token_tenant_ids.len(),
                    "auth initialized in static_token mode (DEMO ONLY)"
                );
                AuthenticatorInner::StaticToken(Arc::new(s))
            }
        };
        Ok(Self {
            inner,
            policy,
            static_token_tenant_ids: cfg.static_token_tenant_ids,
        })
    }

    /// Authenticate a raw `Authorization: Bearer ...` value (without
    /// the "Bearer " prefix). Populates `Principal.roles` from the
    /// configured group policy.
    pub async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        let mut principal = match &self.inner {
            AuthenticatorInner::Jwt(v) => v.validate(token).await?,
            AuthenticatorInner::StaticToken(s) => {
                if subtle_eq(token.as_bytes(), s.token.as_bytes()) {
                    let mut p = Principal {
                        issuer: "static-token".into(),
                        subject: s.subject.clone(),
                        // For policy resolution: static-token
                        // principals belong to a synthetic group
                        // named "demo-admins" that the demo policy
                        // maps to all roles.
                        groups: vec!["demo-admins".to_string()],
                        tenant_ids: self.static_token_tenant_ids.clone(),
                        roles: Vec::new(),
                        mode: "static_token".into(),
                    };
                    p.set_roles(self.policy.roles_for_groups(&p.groups));
                    return Ok(p);
                } else {
                    return Err(AuthError::StaticTokenMismatch);
                }
            }
        };
        // S18: apply group policy to populate roles.
        let roles = self.policy.roles_for_groups(&principal.groups);
        principal.set_roles(roles);
        Ok(principal)
    }
}

fn subtle_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ============================================================================
// Axum middleware
// ============================================================================

/// Tower/axum middleware wrapper. Inserts `Principal` into request
/// extensions on success; returns the safe-public message on failure.
pub async fn require_auth(
    auth: axum::extract::State<Arc<Authenticator>>,
    mut req: Request,
    next: Next,
) -> Response {
    let token = match extract_bearer(&req) {
        Ok(t) => t,
        Err(e) => return reject(&e),
    };
    match auth.authenticate(token.as_str()).await {
        Ok(principal) => {
            tracing::Span::current().record("subject", tracing::field::display(&principal.subject));
            req.extensions_mut().insert(principal);
            next.run(req).await
        }
        Err(e) => {
            warn!(error = %e, "auth rejected");
            reject(&e)
        }
    }
}

fn extract_bearer(req: &Request) -> Result<String, AuthError> {
    let header = req
        .headers()
        .get(AUTHORIZATION)
        .ok_or(AuthError::MissingAuthHeader)?;
    let s = header
        .to_str()
        .map_err(|_| AuthError::MissingAuthHeader)?;
    let bearer = s
        .strip_prefix("Bearer ")
        .ok_or(AuthError::MissingAuthHeader)?;
    Ok(bearer.to_string())
}

fn reject(err: &AuthError) -> Response {
    let body = serde_json::json!({ "error": err.safe_public_message() });
    let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
    Response::builder()
        .status(err.status_code())
        .header("content-type", "application/json")
        .body(Body::from(body_bytes))
        .expect("reject response")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_mode_parse_known_values() {
        assert_eq!(AuthMode::parse("jwt").unwrap(), AuthMode::Jwt);
        assert_eq!(
            AuthMode::parse("static_token").unwrap(),
            AuthMode::StaticToken
        );
        assert!(AuthMode::parse("invalid").is_err());
    }

    #[tokio::test]
    async fn static_token_authenticator_accepts_correct_token() {
        let auth = Authenticator::from_config(AuthConfig {
            kind: AuthConfigKind::StaticToken(StaticTokenConfig {
                token: "abc-123".into(),
                subject: "operator".into(),
            }),
            policy: GroupPolicy::demo_default(),
            static_token_tenant_ids: vec!["t1".into()],
        })
        .await
        .unwrap();

        let p = auth.authenticate("abc-123").await.unwrap();
        assert_eq!(p.mode, "static_token");
        assert_eq!(p.subject, "operator");
        assert_eq!(p.issuer, "static-token");
    }

    #[tokio::test]
    async fn static_token_authenticator_rejects_wrong_token() {
        let auth = Authenticator::from_config(AuthConfig {
            kind: AuthConfigKind::StaticToken(StaticTokenConfig {
                token: "right".into(),
                subject: "operator".into(),
            }),
            policy: GroupPolicy::empty(),
            static_token_tenant_ids: vec![],
        })
        .await
        .unwrap();
        let err = auth.authenticate("wrong").await.unwrap_err();
        assert!(matches!(err, AuthError::StaticTokenMismatch));
    }

    #[tokio::test]
    async fn static_token_constant_time_comparison_handles_length_mismatch() {
        // Different-length inputs short-circuit but must still
        // return the StaticTokenMismatch error, not panic.
        let auth = Authenticator::from_config(AuthConfig {
            kind: AuthConfigKind::StaticToken(StaticTokenConfig {
                token: "a-much-longer-secret-token-string".into(),
                subject: "operator".into(),
            }),
            policy: GroupPolicy::empty(),
            static_token_tenant_ids: vec![],
        })
        .await
        .unwrap();
        let err = auth.authenticate("short").await.unwrap_err();
        assert!(matches!(err, AuthError::StaticTokenMismatch));
    }

    #[test]
    fn safe_public_messages_dont_reveal_internals() {
        // Spec: "auth failures must not reveal tenant existence".
        // The public-safe text is generic on every variant.
        let all = [
            AuthError::MissingAuthHeader,
            AuthError::InvalidToken("kid foo missing".into()),
            AuthError::IssuerMismatch,
            AuthError::AudienceMismatch,
            AuthError::Expired,
            AuthError::UnknownKid("abc".into()),
            AuthError::JwksFetch("network down".into()),
            AuthError::StaticTokenMismatch,
        ];
        for e in &all {
            let msg = e.safe_public_message();
            // None of the safe messages contain dynamic content like
            // kids, issuer URLs, or tenant ids.
            assert!(!msg.contains("kid"));
            assert!(!msg.contains("network"));
            assert!(!msg.contains("issuer"));
            // All are short and well-known.
            assert!(msg.len() < 64, "msg too long: {msg}");
        }
    }

    #[test]
    fn static_token_outside_demo_profile_refuses_to_construct() {
        // Direct test of AuthConfig::from_env's profile gate.
        std::env::set_var("TEST_AUTH_MODE", "static_token");
        std::env::set_var("TEST_STATIC_TOKEN", "abc");
        let result = AuthConfig::from_env("TEST", "production");
        assert!(matches!(result, Err(AuthError::StaticTokenOutsideDemo)));
        // Demo profile allows it.
        let result = AuthConfig::from_env("TEST", "demo");
        assert!(result.is_ok(), "demo profile must allow static_token: {result:?}");
        std::env::remove_var("TEST_AUTH_MODE");
        std::env::remove_var("TEST_STATIC_TOKEN");
    }

    #[test]
    fn auth_mode_string_matches_principal_mode_field() {
        // Operators rely on the mode tag in audit logs being stable.
        assert_eq!(
            AuthConfig {
                kind: AuthConfigKind::StaticToken(StaticTokenConfig {
                    token: "x".into(),
                    subject: "x".into()
                }),
                policy: GroupPolicy::empty(),
                static_token_tenant_ids: vec![],
            }
            .mode_str(),
            "static_token"
        );
    }

    // --------------------------------------------------------------
    // S18 integration tests across Authenticator + Principal +
    // GroupPolicy.
    // --------------------------------------------------------------

    #[tokio::test]
    async fn static_token_principal_in_demo_profile_inherits_demo_admin_roles() {
        let auth = Authenticator::from_config(AuthConfig {
            kind: AuthConfigKind::StaticToken(StaticTokenConfig {
                token: "demo-token".into(),
                subject: "demo-operator".into(),
            }),
            policy: GroupPolicy::demo_default(),
            static_token_tenant_ids: vec!["t1".into(), "t2".into()],
        })
        .await
        .unwrap();
        let p = auth.authenticate("demo-token").await.unwrap();
        // Demo policy maps "demo-admins" -> all roles. The
        // synthetic group is added inside Authenticator.
        for r in ["admin", "operator", "auditor", "approver", "viewer"] {
            assert!(p.roles.iter().any(|x| x == r), "missing role: {r}");
        }
        assert_eq!(p.tenant_ids, vec!["t1".to_string(), "t2".to_string()]);
        // Admin can do every permission.
        for perm in [
            Permission::ReadView,
            Permission::TenantWrite,
            Permission::ApprovalResolve,
            Permission::AuditExport,
            Permission::BudgetWrite,
        ] {
            assert!(p.has_permission(perm), "demo admin should have {perm:?}");
        }
    }

    #[tokio::test]
    async fn static_token_principal_with_empty_policy_has_zero_permissions() {
        // Production-shaped fail-closed: even though static_token
        // is admitted in demo profile, an empty policy means the
        // resulting principal can't do anything.
        let auth = Authenticator::from_config(AuthConfig {
            kind: AuthConfigKind::StaticToken(StaticTokenConfig {
                token: "t".into(),
                subject: "s".into(),
            }),
            policy: GroupPolicy::empty(),
            static_token_tenant_ids: vec!["t1".into()],
        })
        .await
        .unwrap();
        let p = auth.authenticate("t").await.unwrap();
        assert!(p.roles.is_empty());
        for perm in [
            Permission::ReadView,
            Permission::TenantWrite,
            Permission::ApprovalResolve,
            Permission::AuditExport,
            Permission::BudgetWrite,
        ] {
            assert!(!p.has_permission(perm));
        }
        // Tenant scope still applied.
        p.assert_tenant("t1").unwrap();
        assert!(p.assert_tenant("t2").is_err());
    }

    #[tokio::test]
    async fn jwt_principal_roles_populated_from_group_policy() {
        let mut policy_map = HashMap::new();
        policy_map.insert("eng-readers".to_string(), vec![Role::Viewer]);
        policy_map.insert(
            "platform-admins".to_string(),
            vec![Role::Admin, Role::Operator],
        );
        let policy = GroupPolicy {
            mapping: policy_map,
            default_viewer_on_miss: false,
        };
        let (validator, enc, kid) = make_validator("aud", "iss");
        let auth = Authenticator {
            inner: AuthenticatorInner::Jwt(Arc::new(validator)),
            policy: Arc::new(policy),
            static_token_tenant_ids: vec![],
        };
        let exp = (Utc::now() + chrono::Duration::seconds(60)).timestamp();
        let token = issue_jwt(
            &enc,
            &kid,
            "iss",
            "aud",
            "u@example.com",
            exp,
            vec!["platform-admins"],
            vec!["t1"],
        );
        let p = auth.authenticate(&token).await.unwrap();
        assert!(p.roles.contains(&"admin".to_string()));
        assert!(p.roles.contains(&"operator".to_string()));
        assert!(p.has_permission(Permission::TenantWrite));
        p.assert_tenant("t1").unwrap();
        assert!(matches!(
            p.assert_tenant("t-other").unwrap_err(),
            crate::rbac::AuthzError::CrossTenant { .. }
        ));
    }

    // -----------------------------------------------------------------
    // JWT path tests use a fake JwksProvider so we don't need a real
    // OIDC server.
    // -----------------------------------------------------------------

    use jsonwebtoken::{encode, EncodingKey, Header};
    use rand::rngs::OsRng;
    use std::collections::HashMap;

    struct FakeJwks {
        keys: HashMap<String, DecodingKey>,
    }

    #[async_trait]
    impl JwksProvider for FakeJwks {
        async fn key_for(&self, kid: &str) -> Result<DecodingKey, AuthError> {
            self.keys
                .get(kid)
                .cloned()
                .ok_or_else(|| AuthError::UnknownKid(kid.to_string()))
        }
    }

    fn make_validator(audience: &str, issuer: &str) -> (JwtValidator, EncodingKey, String) {
        // Generate ed25519 key for HS-style fake JWT — but jsonwebtoken
        // doesn't support ed25519 with arbitrary kids out of the box.
        // Use HS256 with a shared secret for these tests; we still
        // exercise issuer / audience / expiry / kid lookup logic.
        let secret = b"test-shared-secret-for-jwt-tests-32";
        let kid = "test-kid-1";
        let cfg = JwtConfig {
            issuer: issuer.into(),
            audience: audience.into(),
            jwks_url: "https://unused".into(),
            clock_skew_seconds: 60,
            jwks_refresh_seconds: 3600,
            groups_claim: "groups".into(),
            tenant_ids_claim: "spendguard:tenant_ids".into(),
        };
        let mut keys = HashMap::new();
        keys.insert(kid.to_string(), DecodingKey::from_secret(secret));
        let jwks = Arc::new(FakeJwks { keys });
        let validator = JwtValidator::with_jwks(cfg, jwks);
        (
            validator,
            EncodingKey::from_secret(secret),
            kid.to_string(),
        )
    }

    fn issue_jwt(
        enc: &EncodingKey,
        kid: &str,
        iss: &str,
        aud: &str,
        sub: &str,
        exp: i64,
        groups: Vec<&str>,
        tenant_ids: Vec<&str>,
    ) -> String {
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some(kid.to_string());
        let claims = serde_json::json!({
            "iss": iss,
            "aud": aud,
            "sub": sub,
            "exp": exp,
            "groups": groups,
            "spendguard:tenant_ids": tenant_ids,
        });
        encode(&header, &claims, enc).unwrap()
    }

    #[tokio::test]
    async fn jwt_validator_accepts_well_formed_token() {
        let (v, enc, kid) =
            make_validator("api://spendguard", "https://example.com/issuer");
        let exp = (Utc::now() + chrono::Duration::seconds(60)).timestamp();
        let token = issue_jwt(
            &enc,
            &kid,
            "https://example.com/issuer",
            "api://spendguard",
            "user@example.com",
            exp,
            vec!["admins"],
            vec!["tenant-1", "tenant-2"],
        );
        let principal = v.validate(&token).await.unwrap();
        assert_eq!(principal.subject, "user@example.com");
        assert_eq!(principal.groups, vec!["admins".to_string()]);
        assert_eq!(
            principal.tenant_ids,
            vec!["tenant-1".to_string(), "tenant-2".to_string()]
        );
        assert_eq!(principal.mode, "jwt");
    }

    #[tokio::test]
    async fn jwt_validator_rejects_wrong_issuer() {
        let (v, enc, kid) = make_validator("aud", "https://expected/iss");
        let exp = (Utc::now() + chrono::Duration::seconds(60)).timestamp();
        let token = issue_jwt(
            &enc,
            &kid,
            "https://attacker/iss",
            "aud",
            "u",
            exp,
            vec![],
            vec![],
        );
        let err = v.validate(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::IssuerMismatch));
    }

    #[tokio::test]
    async fn jwt_validator_rejects_wrong_audience() {
        let (v, enc, kid) = make_validator("expected-aud", "iss");
        let exp = (Utc::now() + chrono::Duration::seconds(60)).timestamp();
        let token = issue_jwt(&enc, &kid, "iss", "wrong-aud", "u", exp, vec![], vec![]);
        let err = v.validate(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::AudienceMismatch));
    }

    #[tokio::test]
    async fn jwt_validator_rejects_expired_token() {
        let (v, enc, kid) = make_validator("aud", "iss");
        // exp 5 minutes in the past — well past the 60s leeway.
        let exp = (Utc::now() - chrono::Duration::seconds(300)).timestamp();
        let token = issue_jwt(&enc, &kid, "iss", "aud", "u", exp, vec![], vec![]);
        let err = v.validate(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::Expired));
    }

    #[tokio::test]
    async fn jwt_validator_rejects_unknown_kid() {
        let (v, enc, _kid) = make_validator("aud", "iss");
        let exp = (Utc::now() + chrono::Duration::seconds(60)).timestamp();
        let token = issue_jwt(&enc, "kid-not-in-jwks", "iss", "aud", "u", exp, vec![], vec![]);
        let err = v.validate(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::UnknownKid(_)));
    }

    #[tokio::test]
    async fn jwt_validator_default_groups_claim_population() {
        let (v, enc, kid) = make_validator("aud", "iss");
        let exp = (Utc::now() + chrono::Duration::seconds(60)).timestamp();
        let token = issue_jwt(
            &enc,
            &kid,
            "iss",
            "aud",
            "u",
            exp,
            vec!["g1", "g2"],
            vec![],
        );
        let p = v.validate(&token).await.unwrap();
        assert_eq!(p.groups, vec!["g1".to_string(), "g2".to_string()]);
        // S17 leaves roles empty (S18 wires them).
        assert!(p.roles.is_empty());
    }

    #[test]
    fn extract_bearer_handles_well_formed_header() {
        let req = http::Request::builder()
            .header(AUTHORIZATION, "Bearer mytoken")
            .body(())
            .unwrap();
        // Convert to axum Request via body adapter.
        let (parts, _) = req.into_parts();
        let req = Request::from_parts(parts, Body::empty());
        let s = extract_bearer(&req).unwrap();
        assert_eq!(s, "mytoken");
    }

    #[test]
    fn extract_bearer_rejects_missing_or_malformed_header() {
        let req = http::Request::builder().body(()).unwrap();
        let (parts, _) = req.into_parts();
        let req = Request::from_parts(parts, Body::empty());
        let err = extract_bearer(&req).unwrap_err();
        assert!(matches!(err, AuthError::MissingAuthHeader));

        let req = http::Request::builder()
            .header(AUTHORIZATION, "Basic abc")
            .body(())
            .unwrap();
        let (parts, _) = req.into_parts();
        let req = Request::from_parts(parts, Body::empty());
        let err = extract_bearer(&req).unwrap_err();
        assert!(matches!(err, AuthError::MissingAuthHeader));
    }
}
