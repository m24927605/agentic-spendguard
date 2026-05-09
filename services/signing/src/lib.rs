//! Phase 5 GA hardening S6: Producer signing abstraction.
//!
//! All audit-producing services (sidecar, webhook_receiver, ttl_sweeper,
//! ledger invoice_reconcile decision row) call `Signer::sign()` over the
//! canonical CloudEvent payload bytes before writing to `audit_outbox`.
//! The signature, key_id, and algorithm flow through the stored procedure
//! as new columns so operators can audit "which key signed which row"
//! without deserializing the CloudEvent BYTEA blob.
//!
//! Three modes:
//!
//!   * `local` — `LocalEd25519Signer` reads a PKCS8 PEM file from disk.
//!     Used in non-KMS deployments (single-region demos, on-prem
//!     installs without HSM access).
//!   * `kms` — `KmsSigner` is a stub that returns `ModeUnavailable`
//!     until S7 wires the AWS KMS / GCP KMS / Azure Key Vault clients.
//!     Kept as a typed surface so callers compile against the production
//!     shape today.
//!   * `disabled` — `DisabledSigner` returns an empty signature. Refuses
//!     to construct unless `SPENDGUARD_PROFILE=demo` is set, so an
//!     operator who copy-pastes a non-prod values.yaml fragment into a
//!     prod chart still gets fail-fast at process startup rather than
//!     silently writing unsigned audit rows.
//!
//! Canonical input contract: callers pass the CloudEvent's
//! serialized-as-protobuf payload bytes. The signer signs the SHA-256
//! hash (so signatures stay cheap regardless of payload size). Verifiers
//! reproduce the hash from the stored CloudEvent and use the public key
//! pinned by `signing_key_id` (S7 will register these in a key registry;
//! S8 wires strict canonical signature verification on the consumer
//! side). Until then, we still emit valid signatures so a future
//! verifier can backfill-validate.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signer as Ed25519SignerTrait, SigningKey};
use sha2::{Digest, Sha256};
use std::path::Path;
use thiserror::Error;
use tracing::{info, warn};

/// Output of `Signer::sign()`. Carries everything the audit_outbox row
/// builder needs to populate the new columns added by migration 0024.
#[derive(Debug, Clone)]
pub struct Signature {
    /// Raw signature bytes (Ed25519: 64 bytes).
    pub bytes: Vec<u8>,
    /// Stable identifier for the signing key. For local mode this is
    /// derived from the SHA-256 of the public key bytes; for KMS this
    /// would be the KMS arn / key uri.
    pub key_id: String,
    /// Algorithm name; today always "ed25519". Stored explicitly so
    /// future mixed-algo deployments don't silently roll over.
    pub algorithm: String,
    /// Server-side wallclock at signing time. Independent of the
    /// CloudEvent's `time` field — auditors use this to detect cases
    /// where producer fabricated a CloudEvent timestamp.
    pub signed_at: DateTime<Utc>,
    /// Producer identity string (e.g. "sidecar:wl-abc-123",
    /// "webhook-receiver:region-us-west2"). Echoed into audit logs.
    pub producer_identity: String,
}

#[derive(Debug, Error)]
pub enum SignError {
    #[error("signing mode unavailable: {0}")]
    ModeUnavailable(String),
    #[error("disabled mode is only allowed in demo profile (SPENDGUARD_PROFILE=demo); refusing to construct DisabledSigner in production")]
    DisabledOutsideDemo,
    #[error("invalid signing key file: {0}")]
    InvalidKeyFile(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("ed25519: {0}")]
    Ed25519(String),
}

/// Signer trait. Async because the production KMS implementation
/// will RPC to AWS / GCP / Azure. Local + Disabled impls are sync
/// internally; the async surface is uniform.
#[async_trait]
pub trait Signer: Send + Sync {
    /// Sign the SHA-256 hash of `canonical_bytes`. Returns the full
    /// `Signature` envelope including key_id, algorithm, signed_at, and
    /// producer_identity. Callers must not mutate `canonical_bytes`
    /// between hashing and signing — the canonical encoding is the
    /// contract; any drift breaks the verifier (S8).
    async fn sign(&self, canonical_bytes: &[u8]) -> Result<Signature, SignError>;

    /// Stable key identifier. Useful for logs and metrics.
    fn key_id(&self) -> &str;

    /// Algorithm string. Today "ed25519" or "disabled".
    fn algorithm(&self) -> &str;

    /// Producer identity string (service:instance).
    fn producer_identity(&self) -> &str;
}

// ============================================================================
// Local Ed25519 implementation
// ============================================================================

pub struct LocalEd25519Signer {
    signing_key: SigningKey,
    key_id: String,
    producer_identity: String,
}

impl LocalEd25519Signer {
    /// Construct from an in-memory SigningKey. Computes the key_id as
    /// `ed25519:<hex(sha256(public_key_bytes))[..16]>`. This stable id
    /// is what gets written to `audit_outbox.signing_key_id` and
    /// echoed in CloudEvent extension attributes.
    pub fn from_key(signing_key: SigningKey, producer_identity: String) -> Self {
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let mut hasher = Sha256::new();
        hasher.update(pubkey_bytes);
        let digest = hasher.finalize();
        // First 16 hex chars = 8 bytes of digest. Plenty of entropy for
        // a non-cryptographic identifier; full digest available via the
        // future key registry (S7).
        let key_id = format!("ed25519:{}", &hex::encode(digest)[..16]);
        Self {
            signing_key,
            key_id,
            producer_identity,
        }
    }

    /// Load a PKCS8 PEM file from disk. The file must contain a
    /// PKCS8-encoded Ed25519 private key. We read the key once at
    /// startup; rotating the file requires a process restart (S7
    /// adds in-place rotation via the key registry).
    pub fn from_pkcs8_pem_file(
        path: &Path,
        producer_identity: String,
    ) -> Result<Self, SignError> {
        use ed25519_dalek::pkcs8::DecodePrivateKey;
        let pem = std::fs::read_to_string(path)?;
        let signing_key = SigningKey::from_pkcs8_pem(&pem)
            .map_err(|e| SignError::InvalidKeyFile(format!("{path:?}: {e}")))?;
        let signer = Self::from_key(signing_key, producer_identity);
        info!(
            key_id = %signer.key_id,
            producer = %signer.producer_identity,
            path = %path.display(),
            "loaded Ed25519 signing key from disk"
        );
        Ok(signer)
    }
}

#[async_trait]
impl Signer for LocalEd25519Signer {
    async fn sign(&self, canonical_bytes: &[u8]) -> Result<Signature, SignError> {
        let mut hasher = Sha256::new();
        hasher.update(canonical_bytes);
        let digest = hasher.finalize();
        let sig = self.signing_key.sign(&digest);
        Ok(Signature {
            bytes: sig.to_bytes().to_vec(),
            key_id: self.key_id.clone(),
            algorithm: "ed25519".into(),
            signed_at: Utc::now(),
            producer_identity: self.producer_identity.clone(),
        })
    }

    fn key_id(&self) -> &str {
        &self.key_id
    }

    fn algorithm(&self) -> &str {
        "ed25519"
    }

    fn producer_identity(&self) -> &str {
        &self.producer_identity
    }
}

// ============================================================================
// KMS stub (S7 will fill in)
// ============================================================================

pub struct KmsSigner {
    pub key_arn: String,
    pub producer_identity: String,
}

impl KmsSigner {
    pub fn new(key_arn: String, producer_identity: String) -> Self {
        Self {
            key_arn,
            producer_identity,
        }
    }
}

#[async_trait]
impl Signer for KmsSigner {
    async fn sign(&self, _canonical_bytes: &[u8]) -> Result<Signature, SignError> {
        Err(SignError::ModeUnavailable(format!(
            "KMS signing mode is not yet implemented (S7); arn={}",
            self.key_arn
        )))
    }

    fn key_id(&self) -> &str {
        &self.key_arn
    }

    fn algorithm(&self) -> &str {
        "kms-ed25519"
    }

    fn producer_identity(&self) -> &str {
        &self.producer_identity
    }
}

// ============================================================================
// Disabled signer (demo / test only)
// ============================================================================

pub struct DisabledSigner {
    producer_identity: String,
}

impl DisabledSigner {
    /// Construct only when the supplied profile is `"demo"`. Refuses
    /// outside demo so a misconfigured Helm chart fails at startup,
    /// not at audit time. The env-reading wrapper is `from_env`.
    /// Tests that want a disabled signer without env coupling should
    /// use `for_test()`.
    pub fn for_profile(profile: &str, producer_identity: String) -> Result<Self, SignError> {
        if profile != "demo" {
            return Err(SignError::DisabledOutsideDemo);
        }
        warn!(
            producer = %producer_identity,
            "DisabledSigner constructed — audit signatures will be empty (demo profile)"
        );
        Ok(Self { producer_identity })
    }

    /// Convenience wrapper that reads `SPENDGUARD_PROFILE` from the
    /// environment. Used from `signer_from_env`.
    pub fn from_env(producer_identity: String) -> Result<Self, SignError> {
        let profile = std::env::var("SPENDGUARD_PROFILE").unwrap_or_default();
        Self::for_profile(&profile, producer_identity)
    }

    /// Test-only constructor that bypasses the demo profile check.
    /// Marked `#[doc(hidden)]` so consumer crates don't reach for it
    /// in production paths.
    #[doc(hidden)]
    pub fn for_test(producer_identity: String) -> Self {
        Self { producer_identity }
    }
}

#[async_trait]
impl Signer for DisabledSigner {
    async fn sign(&self, _canonical_bytes: &[u8]) -> Result<Signature, SignError> {
        // Return a syntactically valid Signature with empty bytes.
        // Verifiers (S8) treat empty bytes as "unsigned" and refuse
        // to validate; in demo profile that's the desired behavior.
        Ok(Signature {
            bytes: Vec::new(),
            key_id: format!("disabled:{}", self.producer_identity),
            algorithm: "disabled".into(),
            signed_at: Utc::now(),
            producer_identity: self.producer_identity.clone(),
        })
    }

    fn key_id(&self) -> &str {
        "disabled"
    }

    fn algorithm(&self) -> &str {
        "disabled"
    }

    fn producer_identity(&self) -> &str {
        &self.producer_identity
    }
}

// ============================================================================
// Mode selection (read from env at service startup)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningMode {
    Local,
    Kms,
    Disabled,
}

impl SigningMode {
    pub fn parse(s: &str) -> Result<Self, SignError> {
        match s {
            "local" => Ok(Self::Local),
            "kms" => Ok(Self::Kms),
            "disabled" => Ok(Self::Disabled),
            other => Err(SignError::InvalidKeyFile(format!(
                "unknown SIGNING_MODE: {other:?} (expected local|kms|disabled)"
            ))),
        }
    }
}

/// Build a `Box<dyn Signer>` from environment variables. Used by every
/// audit-producing service at startup so the wiring is uniform.
///
/// Environment contract (suffixed with the service name, e.g.
/// `SPENDGUARD_SIDECAR_SIGNING_MODE`):
///
/// * `<PREFIX>_SIGNING_MODE`: `local` (default) | `kms` | `disabled`
/// * `<PREFIX>_SIGNING_KEY_PATH`: path to PKCS8 PEM (local mode only)
/// * `<PREFIX>_SIGNING_KMS_ARN`: KMS arn (kms mode only)
/// * `<PREFIX>_SIGNING_PRODUCER_IDENTITY`: required string used in
///   CloudEvent.producer_id and audit logs.
///
/// `SPENDGUARD_PROFILE=demo` is required if mode=disabled.
pub fn signer_from_env(prefix: &str) -> Result<Box<dyn Signer>, SignError> {
    let mode_var = format!("{prefix}_SIGNING_MODE");
    let mode = std::env::var(&mode_var).unwrap_or_else(|_| "local".into());
    let mode = SigningMode::parse(&mode)?;

    let identity_var = format!("{prefix}_SIGNING_PRODUCER_IDENTITY");
    let producer_identity = std::env::var(&identity_var).map_err(|_| {
        SignError::InvalidKeyFile(format!("{identity_var} env var required"))
    })?;

    match mode {
        SigningMode::Local => {
            let path_var = format!("{prefix}_SIGNING_KEY_PATH");
            let key_path = std::env::var(&path_var).map_err(|_| {
                SignError::InvalidKeyFile(format!(
                    "{path_var} env var required when SIGNING_MODE=local"
                ))
            })?;
            let signer = LocalEd25519Signer::from_pkcs8_pem_file(
                Path::new(&key_path),
                producer_identity,
            )?;
            Ok(Box::new(signer))
        }
        SigningMode::Kms => {
            let arn_var = format!("{prefix}_SIGNING_KMS_ARN");
            let arn = std::env::var(&arn_var).map_err(|_| {
                SignError::InvalidKeyFile(format!(
                    "{arn_var} env var required when SIGNING_MODE=kms"
                ))
            })?;
            // KmsSigner constructs successfully but its sign() returns
            // ModeUnavailable until S7. That's intentional — operators
            // who set SIGNING_MODE=kms today get a clear runtime error
            // pointing at the missing implementation, not silent empty
            // signatures.
            Ok(Box::new(KmsSigner::new(arn, producer_identity)))
        }
        SigningMode::Disabled => {
            let signer = DisabledSigner::from_env(producer_identity)?;
            Ok(Box::new(signer))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;

    fn make_local_signer() -> LocalEd25519Signer {
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        LocalEd25519Signer::from_key(signing_key, "test-producer:1".into())
    }

    #[tokio::test]
    async fn local_signer_produces_stable_signature_for_same_input() {
        let signer = make_local_signer();
        let payload = b"canonical-cloudevent-bytes";
        let sig1 = signer.sign(payload).await.unwrap();
        let sig2 = signer.sign(payload).await.unwrap();
        // Ed25519 is deterministic per RFC 8032, so the same input ->
        // the same signature bytes (so we can use signatures as
        // dedup keys; this also catches non-determinism regressions).
        assert_eq!(sig1.bytes, sig2.bytes);
        assert_eq!(sig1.bytes.len(), 64);
        assert_eq!(sig1.algorithm, "ed25519");
        assert!(sig1.key_id.starts_with("ed25519:"));
        assert_eq!(sig1.producer_identity, "test-producer:1");
    }

    #[tokio::test]
    async fn local_signer_produces_different_signatures_for_different_inputs() {
        let signer = make_local_signer();
        let sig1 = signer.sign(b"alpha").await.unwrap();
        let sig2 = signer.sign(b"beta").await.unwrap();
        assert_ne!(sig1.bytes, sig2.bytes);
    }

    #[tokio::test]
    async fn key_id_stable_across_signs() {
        let signer = make_local_signer();
        let original = signer.key_id().to_string();
        for _ in 0..5 {
            let sig = signer.sign(b"x").await.unwrap();
            assert_eq!(sig.key_id, original);
        }
    }

    #[tokio::test]
    async fn key_id_differs_per_keypair() {
        let s1 = make_local_signer();
        let s2 = make_local_signer();
        // Distinct keypairs MUST yield distinct key_ids; otherwise an
        // operator can't tell rows apart in audit_outbox.
        assert_ne!(s1.key_id(), s2.key_id());
    }

    #[tokio::test]
    async fn local_signer_round_trips_pkcs8_pem() {
        use ed25519_dalek::pkcs8::EncodePrivateKey;
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let pem = signing_key
            .to_pkcs8_pem(ed25519_dalek::pkcs8::spki::der::pem::LineEnding::LF)
            .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ed25519.pem");
        std::fs::write(&path, pem.as_bytes()).unwrap();

        let signer = LocalEd25519Signer::from_pkcs8_pem_file(
            &path,
            "test-producer:pem".into(),
        )
        .unwrap();
        let sig = signer.sign(b"hello").await.unwrap();
        assert_eq!(sig.bytes.len(), 64);
        assert_eq!(sig.producer_identity, "test-producer:pem");
    }

    #[tokio::test]
    async fn kms_signer_returns_mode_unavailable_until_s7() {
        let signer = KmsSigner::new("arn:aws:kms:test".into(), "test-producer:kms".into());
        let result = signer.sign(b"hello").await;
        match result {
            Err(SignError::ModeUnavailable(msg)) => {
                assert!(msg.contains("S7"));
                assert!(msg.contains("arn:aws:kms:test"));
            }
            _ => panic!("expected ModeUnavailable"),
        }
        // Even though sign() fails, the signer's metadata is queryable
        // for diagnostic logs.
        assert_eq!(signer.algorithm(), "kms-ed25519");
        assert_eq!(signer.key_id(), "arn:aws:kms:test");
    }

    #[tokio::test]
    async fn disabled_signer_refuses_outside_demo_profile() {
        // Pure-function variant — no env mutation, safe under
        // parallel cargo test execution.
        let r1 = DisabledSigner::for_profile("", "test-producer:disabled".into());
        assert!(matches!(r1, Err(SignError::DisabledOutsideDemo)));

        let r2 = DisabledSigner::for_profile("production", "test-producer:disabled".into());
        assert!(matches!(r2, Err(SignError::DisabledOutsideDemo)));

        let r3 = DisabledSigner::for_profile("staging", "test-producer:disabled".into());
        assert!(matches!(r3, Err(SignError::DisabledOutsideDemo)));
    }

    #[tokio::test]
    async fn disabled_signer_constructs_in_demo_profile() {
        let signer = DisabledSigner::for_profile("demo", "test-producer:demo".into())
            .expect("demo allows disabled");
        let sig = signer.sign(b"hello").await.unwrap();
        // Empty bytes by contract; key_id and algorithm clearly say
        // "disabled" so an audit row reader can't mistake it for real.
        assert!(sig.bytes.is_empty());
        assert_eq!(sig.algorithm, "disabled");
        assert!(sig.key_id.starts_with("disabled:"));
        assert_eq!(sig.producer_identity, "test-producer:demo");
    }

    #[test]
    fn signing_mode_parse_accepts_known_values_rejects_others() {
        assert_eq!(SigningMode::parse("local").unwrap(), SigningMode::Local);
        assert_eq!(SigningMode::parse("kms").unwrap(), SigningMode::Kms);
        assert_eq!(
            SigningMode::parse("disabled").unwrap(),
            SigningMode::Disabled
        );
        assert!(SigningMode::parse("nonsense").is_err());
        assert!(SigningMode::parse("").is_err());
    }

    #[tokio::test]
    async fn signature_metadata_is_complete() {
        // Every Signature must carry all four fields so audit_outbox
        // row builder doesn't have to guess.
        let signer = make_local_signer();
        let sig = signer.sign(b"hello").await.unwrap();
        assert!(!sig.key_id.is_empty());
        assert!(!sig.algorithm.is_empty());
        assert!(!sig.producer_identity.is_empty());
        // signed_at is recent; allow 5s skew for slow CI.
        let elapsed = (Utc::now() - sig.signed_at).num_seconds();
        assert!((-1..=5).contains(&elapsed), "signed_at must be ~now: {elapsed}s");
    }
}
