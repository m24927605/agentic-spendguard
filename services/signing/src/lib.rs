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
use ed25519_dalek::{Signer as Ed25519SignerTrait, Signature as Ed25519Signature, SigningKey, Verifier as Ed25519VerifierTrait, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    /// Round-2 #8 PR 8b: AWS KMS-backed signing failures. Wraps
    /// aws-sdk-kms's typed errors into a single SignError variant so
    /// callers don't need to depend on aws-sdk-kms directly.
    #[error("kms: {0}")]
    Kms(String),
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
// KMS-backed signer (Round-2 #8 PR 8b)
// ============================================================================
//
// Phase 5 GA hardening S7: real AWS KMS integration. Uses
// `aws_sdk_kms::Client::sign()` with `MessageType::Digest` +
// `SigningAlgorithmSpec::EcdsaSha256` so the producer service hashes
// canonical_bytes locally and KMS only sees a 32-byte sha256. This
// keeps KMS request/response bandwidth bounded regardless of payload
// size and matches the "we sign the hash, not the bytes" contract
// LocalEd25519Signer already follows.
//
// Test surface: `KmsSigner::with_client(...)` lets unit tests inject a
// pre-configured client without touching real AWS. The `from_env`
// constructor still takes the production path (loads creds + region
// from environment / IRSA / instance profile via `aws_config`).
//
// Algorithm: ECDSA_SHA_256 (P-256). Operators choosing KMS today get
// ECDSA. Future RSA + Ed25519 KMS keys would extend this enum-style;
// keeping a single algorithm tightens the verifier-side surface for
// canonical_ingest's S8 strict mode.

pub struct KmsSigner {
    pub key_arn: String,
    pub producer_identity: String,
    client: aws_sdk_kms::Client,
}

impl KmsSigner {
    /// Construct a KMS-backed signer. Loads AWS config from the
    /// process environment (IRSA / instance profile / static creds).
    /// Async because `aws_config::load_from_env()` is async.
    pub async fn new(
        key_arn: String,
        producer_identity: String,
    ) -> Result<Self, SignError> {
        // `load_defaults` with an explicit BehaviorVersion is the
        // recommended path; `load_from_env` is deprecated in
        // aws-config 1.x. Pinning latest behavior so a future
        // aws-config bump doesn't silently change credential
        // resolution order.
        let config = aws_config::load_defaults(
            aws_config::BehaviorVersion::latest(),
        )
        .await;
        let client = aws_sdk_kms::Client::new(&config);
        Ok(Self {
            key_arn,
            producer_identity,
            client,
        })
    }

    /// Test hook — inject a pre-built client (useful for wiremock
    /// fixtures that don't touch real AWS endpoints). Production paths
    /// should always use `KmsSigner::new`.
    pub fn with_client(
        client: aws_sdk_kms::Client,
        key_arn: String,
        producer_identity: String,
    ) -> Self {
        Self {
            key_arn,
            producer_identity,
            client,
        }
    }
}

#[async_trait]
impl Signer for KmsSigner {
    async fn sign(&self, canonical_bytes: &[u8]) -> Result<Signature, SignError> {
        use aws_sdk_kms::primitives::Blob;
        use aws_sdk_kms::types::{MessageType, SigningAlgorithmSpec};

        // Hash locally (32-byte sha256) and ask KMS to sign the digest.
        // Matches LocalEd25519Signer's "we sign the hash" contract; the
        // verifier reproduces the same hash from the stored CloudEvent.
        let digest = {
            let mut h = Sha256::new();
            h.update(canonical_bytes);
            h.finalize().to_vec()
        };

        let resp = self
            .client
            .sign()
            .key_id(&self.key_arn)
            .message(Blob::new(digest))
            .message_type(MessageType::Digest)
            .signing_algorithm(SigningAlgorithmSpec::EcdsaSha256)
            .send()
            .await
            .map_err(|e| {
                SignError::Kms(format!("kms sign call failed (arn={}): {}", self.key_arn, e))
            })?;

        let sig_bytes = resp
            .signature
            .ok_or_else(|| {
                SignError::Kms(format!(
                    "kms returned empty signature (arn={})",
                    self.key_arn
                ))
            })?
            .into_inner();

        Ok(Signature {
            bytes: sig_bytes,
            key_id: self.key_arn.clone(),
            algorithm: "kms-ecdsa-sha256".into(),
            signed_at: Utc::now(),
            producer_identity: self.producer_identity.clone(),
        })
    }

    fn key_id(&self) -> &str {
        &self.key_arn
    }

    fn algorithm(&self) -> &str {
        "kms-ecdsa-sha256"
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
///
/// Round-2 #8 PR 8b: now async because `KmsSigner::new` loads AWS
/// config from the environment (IRSA / instance profile / static
/// creds), which is itself async. Local + Disabled paths await on
/// already-resolved futures so the change is zero-cost for non-KMS
/// callers.
pub async fn signer_from_env(prefix: &str) -> Result<Box<dyn Signer>, SignError> {
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
            // Round-2 #8 PR 8b: KmsSigner now constructs against
            // real aws-sdk-kms. The `.await` here loads AWS config
            // (region + creds via IRSA / instance profile / static
            // env). The first sign() call hits the real KMS API.
            let signer = KmsSigner::new(arn, producer_identity).await?;
            Ok(Box::new(signer))
        }
        SigningMode::Disabled => {
            let signer = DisabledSigner::from_env(producer_identity)?;
            Ok(Box::new(signer))
        }
    }
}

// ============================================================================
// Phase 5 GA hardening S8: Verifier trait + LocalEd25519Verifier
// ============================================================================

/// Reasons a signature can fail verification. The canonical_ingest
/// quarantine table stores this string verbatim so operators can sort
/// quarantined rows by failure mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyFailure {
    /// `signing_key_id` did not match any key in the trust store.
    UnknownKey,
    /// Key was found but the signature did not validate against the
    /// canonical bytes.
    InvalidSignature,
    /// `signing_algorithm` is "pre-S6" or empty — row predates S6 and
    /// cannot be verified. Strict mode rejects; non-strict admits.
    PreS6,
    /// `signing_algorithm` is "disabled" — demo-profile row with empty
    /// signature. Strict mode rejects; non-strict admits.
    Disabled,
    /// S7: event_time is past the key's `valid_until`. Possible during
    /// rotation when an old key keeps signing past its window.
    KeyExpired,
    /// S7: event_time is before the key's `valid_from`. Possible if a
    /// producer has clock skew or pre-issued tokens.
    KeyNotYetValid,
    /// S7: key was revoked (manifest set `revoked: true`). Distinct
    /// from KeyExpired — KeyRevoked is operator-driven (incident
    /// response), KeyExpired is calendar-driven.
    KeyRevoked,
}

impl VerifyFailure {
    pub fn as_str(&self) -> &'static str {
        match self {
            VerifyFailure::UnknownKey => "unknown_key",
            VerifyFailure::InvalidSignature => "invalid_signature",
            VerifyFailure::PreS6 => "pre_s6",
            VerifyFailure::Disabled => "disabled",
            VerifyFailure::KeyExpired => "key_expired",
            VerifyFailure::KeyNotYetValid => "key_not_yet_valid",
            VerifyFailure::KeyRevoked => "key_revoked",
        }
    }
}

/// S7: per-key validity window + revocation flag. Loaded from the
/// optional `keys.json` manifest in the trust store directory.
/// When the manifest is absent or doesn't list a given key_id, the
/// verifier falls back to "always valid" — preserves S6/S8 behavior
/// for deployments that haven't yet enabled rotation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyValidity {
    /// Earliest event_time for which this key may sign.
    pub valid_from: chrono::DateTime<chrono::Utc>,
    /// Latest event_time for which this key may sign. None = no
    /// expiry (operator-pinned long-lived key; rare in production).
    pub valid_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Operator-driven revocation. When true, every event signed
    /// by this key fails with `KeyRevoked` regardless of times.
    #[serde(default)]
    pub revoked: bool,
    /// Wallclock at which the operator flipped `revoked = true`.
    /// Useful for forensics: events before this time may still
    /// be considered authentic (the key was good at sign time);
    /// events after must be quarantined.
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Round-2 #8 ECDSA follow-up: optional algorithm tag. When set
    /// to "kms-ecdsa-sha256" the verifier parses `public_key_pem`
    /// as a P-256 SubjectPublicKeyInfo and routes verify() through
    /// the ECDSA path. None / "ed25519" defaults to the existing
    /// filesystem-scanned Ed25519 trust store path.
    #[serde(default)]
    pub algorithm: Option<String>,
    /// Round-2 #8 ECDSA follow-up: inline public key PEM. Operator
    /// dumps the KMS key's public material via
    /// `aws kms get-public-key --key-id <arn> --output text
    /// --query PublicKey | base64 -d | openssl pkey -pubin
    /// -inform DER -outform PEM` and pastes the result here.
    /// Required when `algorithm = Some("kms-ecdsa-sha256")`.
    #[serde(default)]
    pub public_key_pem: Option<String>,
}

impl KeyValidity {
    /// Default "always valid" window — used when the manifest
    /// doesn't list a particular key_id. Matches pre-S7 behavior
    /// so existing deployments don't break.
    pub fn always_valid() -> Self {
        Self {
            valid_from: chrono::DateTime::<chrono::Utc>::MIN_UTC,
            valid_until: None,
            revoked: false,
            revoked_at: None,
            algorithm: None,
            public_key_pem: None,
        }
    }

    /// Check the supplied event_time against this validity window.
    /// Caller passes `None` if it doesn't have an event_time and
    /// wants crypto-only validation — the validity check then
    /// reduces to the revoked flag.
    pub fn check(&self, event_time: Option<chrono::DateTime<chrono::Utc>>) -> Result<(), VerifyFailure> {
        if self.revoked {
            return Err(VerifyFailure::KeyRevoked);
        }
        let event_time = match event_time {
            Some(t) => t,
            None => return Ok(()),
        };
        if event_time < self.valid_from {
            return Err(VerifyFailure::KeyNotYetValid);
        }
        if let Some(until) = self.valid_until {
            if event_time > until {
                return Err(VerifyFailure::KeyExpired);
            }
        }
        Ok(())
    }
}

/// `keys.json` manifest file format. Sits alongside the `*.pem`
/// files in the trust store directory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeysManifest {
    pub keys: HashMap<String, KeyValidity>,
}

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("invalid trust store: {0}")]
    InvalidTrustStore(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Verifier looks up the public key by `signing_key_id` and validates
/// the canonical bytes against the supplied signature. Returns Ok(())
/// on match, Err(VerifyFailure) on mismatch / unknown key / out-of-
/// window, Err other on infrastructure failures.
///
/// `event_time` (S7): when present, enforces the key's validity
/// window (valid_from / valid_until) before crypto check. Pass
/// `None` to skip the window check (crypto + revocation only).
pub trait Verifier: Send + Sync {
    fn verify(
        &self,
        signing_key_id: &str,
        canonical_bytes: &[u8],
        signature_bytes: &[u8],
        event_time: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), VerifyFailure>;

    /// Diagnostic: returns the count of registered keys. Useful for the
    /// canonical_ingest startup log so operators can see "trust store
    /// has 4 keys loaded".
    fn key_count(&self) -> usize;
}

/// File-backed Ed25519 verifier. Loaded at startup from a directory
/// where each `<key_id-with-colons-replaced>.pem` file holds the PKCS8
/// **public** key. The key_id-on-disk substitutes `:` with `_` because
/// most filesystems treat colon as a path separator (POSIX is fine but
/// macOS HFS+ historically, and tooling commonly assumes no colon).
///
/// Naming convention:
///   * `ed25519:1a2b3c4d5e6f7890` (in CloudEvent.signing_key_id)
///   * → file `ed25519_1a2b3c4d5e6f7890.pem` (on disk)
///
/// This is intentionally simple — S7 will replace it with a registry
/// table and rotation. S8's filesystem registry is the bootstrap.
pub struct LocalEd25519Verifier {
    keys: HashMap<String, VerifyingKey>,
    /// Round-2 #8 ECDSA follow-up: P-256 verifying keys loaded from
    /// `keys.json` manifest entries that set
    /// `algorithm = "kms-ecdsa-sha256"` + `public_key_pem`.
    /// Verifying a CloudEvent with `signing_key_id` that hits this
    /// map routes through the ECDSA path instead of Ed25519.
    ecdsa_keys: HashMap<String, p256::ecdsa::VerifyingKey>,
    /// S7: per-key validity windows. Loaded from `keys.json` in the
    /// trust store dir. Keys that aren't in the manifest get
    /// `KeyValidity::always_valid()` (preserves pre-S7 behavior).
    validities: HashMap<String, KeyValidity>,
    /// Directory used for loads; preserved for diagnostics.
    pub source_dir: PathBuf,
}

impl LocalEd25519Verifier {
    /// Construct from a directory of `.pem` files. Each file must hold
    /// either an Ed25519 PKCS8 PUBLIC key OR an Ed25519 PKCS8 PRIVATE
    /// key (we extract the public key automatically — convenient for
    /// demo where the same PEM is mounted to producer and verifier).
    ///
    /// Key id is derived from the verifying key bytes (sha256[..16]),
    /// matching the producer's `LocalEd25519Signer::from_key`. The file
    /// name is irrelevant to identification — operators can name files
    /// after the service (`sidecar.pem`) and the verifier still
    /// resolves `signing_key_id = ed25519:<hex16>` correctly.
    pub fn from_dir(dir: &Path) -> Result<Self, VerifyError> {
        use ed25519_dalek::pkcs8::{DecodePrivateKey, DecodePublicKey};
        let mut keys = HashMap::new();
        if !dir.exists() {
            return Err(VerifyError::InvalidTrustStore(format!(
                "trust store directory does not exist: {}",
                dir.display()
            )));
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("pem") {
                continue;
            }
            let pem = std::fs::read_to_string(&path)?;
            let verifying = if let Ok(sk) = SigningKey::from_pkcs8_pem(&pem) {
                sk.verifying_key()
            } else if let Ok(vk) = VerifyingKey::from_public_key_pem(&pem) {
                vk
            } else {
                warn!(
                    path = %path.display(),
                    "trust store: skipping file (not a parseable PKCS8 ed25519 PEM)"
                );
                continue;
            };
            // Derive key_id matching LocalEd25519Signer::from_key:
            // `ed25519:<sha256(pubkey_bytes)[..16]>`.
            let mut hasher = Sha256::new();
            hasher.update(verifying.to_bytes());
            let digest = hasher.finalize();
            let key_id = format!("ed25519:{}", &hex::encode(digest)[..16]);
            keys.insert(key_id, verifying);
        }

        // S7: optional `keys.json` manifest with per-key validity
        // windows + revocation. Absent manifest means every key is
        // always-valid (preserves S6/S8 behavior).
        let validities = load_keys_manifest(dir)?;

        // Round-2 #8 ECDSA follow-up: walk the manifest for entries
        // that supply inline ECDSA public keys + parse them.
        let mut ecdsa_keys: HashMap<String, p256::ecdsa::VerifyingKey> = HashMap::new();
        for (key_id, validity) in &validities {
            if validity.algorithm.as_deref() == Some("kms-ecdsa-sha256") {
                let pem = validity.public_key_pem.as_deref().ok_or_else(|| {
                    VerifyError::InvalidTrustStore(format!(
                        "keys.json entry {key_id:?}: algorithm=kms-ecdsa-sha256 \
                         requires non-empty public_key_pem"
                    ))
                })?;
                use p256::pkcs8::DecodePublicKey;
                let vk = p256::ecdsa::VerifyingKey::from_public_key_pem(pem.trim())
                    .map_err(|e| {
                        VerifyError::InvalidTrustStore(format!(
                            "keys.json entry {key_id:?}: P-256 SPKI parse: {e}"
                        ))
                    })?;
                ecdsa_keys.insert(key_id.clone(), vk);
            }
        }

        info!(
            dir = %dir.display(),
            ed25519_keys = keys.len(),
            ecdsa_keys = ecdsa_keys.len(),
            validities = validities.len(),
            "trust store loaded"
        );
        Ok(Self {
            keys,
            ecdsa_keys,
            validities,
            source_dir: dir.to_path_buf(),
        })
    }

    /// Test-only constructor that takes an in-memory map. Avoids the
    /// filesystem in unit tests.
    #[doc(hidden)]
    pub fn from_keys(keys: HashMap<String, VerifyingKey>) -> Self {
        Self {
            keys,
            ecdsa_keys: HashMap::new(),
            validities: HashMap::new(),
            source_dir: PathBuf::new(),
        }
    }

    /// Test-only constructor that also takes a validity map.
    #[doc(hidden)]
    pub fn from_keys_with_validity(
        keys: HashMap<String, VerifyingKey>,
        validities: HashMap<String, KeyValidity>,
    ) -> Self {
        Self {
            keys,
            ecdsa_keys: HashMap::new(),
            validities,
            source_dir: PathBuf::new(),
        }
    }

    /// Test-only constructor that takes ECDSA keys + validities.
    /// Used by the round-2 #8 ECDSA verifier unit tests.
    #[doc(hidden)]
    pub fn from_ecdsa_keys_with_validity(
        ecdsa_keys: HashMap<String, p256::ecdsa::VerifyingKey>,
        validities: HashMap<String, KeyValidity>,
    ) -> Self {
        Self {
            keys: HashMap::new(),
            ecdsa_keys,
            validities,
            source_dir: PathBuf::new(),
        }
    }
}

fn load_keys_manifest(dir: &Path) -> Result<HashMap<String, KeyValidity>, VerifyError> {
    let manifest_path = dir.join("keys.json");
    if !manifest_path.exists() {
        return Ok(HashMap::new());
    }
    let raw = std::fs::read_to_string(&manifest_path)?;
    let manifest: KeysManifest = serde_json::from_str(&raw)
        .map_err(|e| VerifyError::InvalidTrustStore(format!("keys.json parse: {e}")))?;
    Ok(manifest.keys)
}

impl Verifier for LocalEd25519Verifier {
    fn verify(
        &self,
        signing_key_id: &str,
        canonical_bytes: &[u8],
        signature_bytes: &[u8],
        event_time: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), VerifyFailure> {
        // Triage by signing_algorithm derivable from the key_id prefix
        // (mirror of migration 0024's CASE expression).
        if signing_key_id.is_empty() || signing_key_id.starts_with("pre-S6") {
            return Err(VerifyFailure::PreS6);
        }
        if signing_key_id.starts_with("disabled:") {
            return Err(VerifyFailure::Disabled);
        }

        // Validity check happens before algorithm dispatch so revoked
        // keys fail closed regardless of which algorithm they use.
        let validity = self
            .validities
            .get(signing_key_id)
            .cloned()
            .unwrap_or_else(KeyValidity::always_valid);
        validity.check(event_time)?;

        // Mirror of LocalEd25519Signer::sign / KmsSigner::sign: the
        // signed bytes are the SHA-256 digest of canonical_bytes,
        // NOT canonical_bytes itself.
        let mut hasher = Sha256::new();
        hasher.update(canonical_bytes);
        let digest = hasher.finalize();

        // Round-2 #8 ECDSA follow-up: dispatch on prefix + lookup hit.
        // - `ed25519:` prefix → Ed25519 path (existing)
        // - anything that hits ecdsa_keys map → P-256 ECDSA path
        // - otherwise UnknownKey
        if let Some(vk) = self.keys.get(signing_key_id) {
            let sig = match Ed25519Signature::from_slice(signature_bytes) {
                Ok(s) => s,
                Err(_) => return Err(VerifyFailure::InvalidSignature),
            };
            return match vk.verify(&digest, &sig) {
                Ok(()) => Ok(()),
                Err(_) => Err(VerifyFailure::InvalidSignature),
            };
        }

        if let Some(vk) = self.ecdsa_keys.get(signing_key_id) {
            use p256::ecdsa::signature::Verifier as EcdsaVerifierTrait;
            // KMS returns ASN.1 DER-encoded ECDSA signatures. The
            // p256 crate's DerSignature parses them natively.
            let sig = match p256::ecdsa::DerSignature::from_bytes(signature_bytes) {
                Ok(s) => s,
                Err(_) => return Err(VerifyFailure::InvalidSignature),
            };
            return match vk.verify(&digest, &sig) {
                Ok(()) => Ok(()),
                Err(_) => Err(VerifyFailure::InvalidSignature),
            };
        }

        Err(VerifyFailure::UnknownKey)
    }

    fn key_count(&self) -> usize {
        self.keys.len() + self.ecdsa_keys.len()
    }
}

/// Convenience: build a Verifier from an env-pinned directory.
/// Used by canonical_ingest at startup when strict mode is enabled.
pub fn verifier_from_env(prefix: &str) -> Result<Box<dyn Verifier>, VerifyError> {
    let var = format!("{prefix}_TRUST_STORE_DIR");
    let dir = std::env::var(&var).map_err(|_| {
        VerifyError::InvalidTrustStore(format!(
            "{var} env var required to construct LocalEd25519Verifier"
        ))
    })?;
    Ok(Box::new(LocalEd25519Verifier::from_dir(Path::new(&dir))?))
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
    async fn kms_signer_metadata_is_queryable_without_calling_aws() {
        // Round-2 #8 PR 8b: real KmsSigner. Unit tests don't actually
        // call AWS — they just pin metadata accessors so a future
        // refactor that swaps the algorithm name (e.g. ECDSA P-384)
        // surfaces here rather than silently breaking the verifier
        // contract. The actual sign() path requires either real KMS
        // creds (integration test) or the `with_client(...)` test
        // hook with a mocked aws_sdk_kms::Client (out-of-scope here).
        let arn = "arn:aws:kms:us-west-2:000000000000:key/0000-test".to_string();
        let signer = KmsSigner::new(arn.clone(), "test-producer:kms".into())
            .await
            .expect("KmsSigner::new must succeed under any AWS config");
        assert_eq!(signer.algorithm(), "kms-ecdsa-sha256");
        assert_eq!(signer.key_id(), arn);
        assert_eq!(signer.producer_identity(), "test-producer:kms");
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

    // ============================================================
    // S8 Verifier tests
    // ============================================================

    fn signer_and_verifier() -> (LocalEd25519Signer, LocalEd25519Verifier) {
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let verifying_key = signing_key.verifying_key();
        let signer = LocalEd25519Signer::from_key(signing_key, "test-producer:v".into());
        let mut keys = HashMap::new();
        keys.insert(signer.key_id().to_string(), verifying_key);
        let verifier = LocalEd25519Verifier::from_keys(keys);
        (signer, verifier)
    }

    #[tokio::test]
    async fn verifier_accepts_real_signature_under_correct_canonical() {
        let (signer, verifier) = signer_and_verifier();
        let canonical = b"canonical-cloudevent-bytes-v1";
        let sig = signer.sign(canonical).await.unwrap();
        verifier
            .verify(&sig.key_id, canonical, &sig.bytes, None)
            .expect("real signature must verify");
    }

    #[tokio::test]
    async fn verifier_rejects_signature_under_mutated_canonical() {
        let (signer, verifier) = signer_and_verifier();
        let canonical = b"canonical-cloudevent-bytes-v1";
        let sig = signer.sign(canonical).await.unwrap();
        let mutated = b"canonical-cloudevent-bytes-XX";
        let err = verifier.verify(&sig.key_id, mutated, &sig.bytes, None).unwrap_err();
        assert_eq!(err, VerifyFailure::InvalidSignature);
    }

    #[tokio::test]
    async fn verifier_rejects_unknown_key_id() {
        let (signer, verifier) = signer_and_verifier();
        let canonical = b"x";
        let sig = signer.sign(canonical).await.unwrap();
        // Fabricate a different key_id; trust store doesn't know it.
        let err = verifier
            .verify("ed25519:deadbeef00000000", canonical, &sig.bytes, None)
            .unwrap_err();
        assert_eq!(err, VerifyFailure::UnknownKey);
    }

    #[tokio::test]
    async fn verifier_classifies_pre_s6_legacy_rows() {
        let verifier = LocalEd25519Verifier::from_keys(HashMap::new());
        for kid in ["pre-S6:legacy", ""] {
            let err = verifier.verify(kid, b"x", b"sig", None).unwrap_err();
            assert_eq!(err, VerifyFailure::PreS6, "key_id {kid:?}");
        }
    }

    // ----- Round-2 #8 ECDSA verifier follow-up -----

    #[tokio::test]
    async fn ecdsa_verifier_accepts_real_p256_signature() {
        use p256::ecdsa::{
            signature::Signer as EcdsaSignerTrait, Signature, SigningKey as EcdsaSigningKey,
        };
        use p256::elliptic_curve::rand_core::OsRng;

        // Generate a P-256 keypair + sign the sha256 digest of a
        // canonical bytes blob (mirror of KmsSigner::sign hash step).
        let sk = EcdsaSigningKey::random(&mut OsRng);
        let vk = *sk.verifying_key();
        let canonical = b"resume-after-approval payload bytes";
        let digest = {
            let mut h = Sha256::new();
            h.update(canonical);
            h.finalize()
        };
        let sig: Signature = sk.sign(&digest);
        let sig_der_bytes = sig.to_der().to_bytes().to_vec();

        let arn = "arn:aws:kms:us-west-2:000000000000:key/0000-test".to_string();
        let mut ecdsa_keys = HashMap::new();
        ecdsa_keys.insert(arn.clone(), vk);
        let verifier = LocalEd25519Verifier::from_ecdsa_keys_with_validity(
            ecdsa_keys,
            HashMap::new(),
        );

        verifier
            .verify(&arn, canonical, &sig_der_bytes, None)
            .expect("ECDSA verify should succeed");
    }

    #[tokio::test]
    async fn ecdsa_verifier_rejects_mutated_canonical() {
        use p256::ecdsa::{
            signature::Signer as EcdsaSignerTrait, Signature, SigningKey as EcdsaSigningKey,
        };
        use p256::elliptic_curve::rand_core::OsRng;

        let sk = EcdsaSigningKey::random(&mut OsRng);
        let vk = *sk.verifying_key();
        let canonical = b"original payload";
        let digest = {
            let mut h = Sha256::new();
            h.update(canonical);
            h.finalize()
        };
        let sig: Signature = sk.sign(&digest);
        let sig_der_bytes = sig.to_der().to_bytes().to_vec();

        let mut ecdsa_keys = HashMap::new();
        ecdsa_keys.insert("kms-key".to_string(), vk);
        let verifier = LocalEd25519Verifier::from_ecdsa_keys_with_validity(
            ecdsa_keys,
            HashMap::new(),
        );

        // Mutate the bytes; sha256 changes; signature should fail.
        let err = verifier
            .verify("kms-key", b"tampered payload", &sig_der_bytes, None)
            .unwrap_err();
        assert_eq!(err, VerifyFailure::InvalidSignature);
    }

    #[tokio::test]
    async fn ecdsa_verifier_rejects_unknown_kms_arn() {
        let verifier = LocalEd25519Verifier::from_ecdsa_keys_with_validity(
            HashMap::new(),
            HashMap::new(),
        );
        let err = verifier
            .verify("arn:aws:kms:us-west-2:0:key/missing", b"x", b"sig", None)
            .unwrap_err();
        assert_eq!(err, VerifyFailure::UnknownKey);
    }

    #[tokio::test]
    async fn ecdsa_verifier_loads_inline_pem_from_keys_manifest() {
        use p256::ecdsa::SigningKey as EcdsaSigningKey;
        use p256::elliptic_curve::rand_core::OsRng;
        use p256::pkcs8::EncodePublicKey;

        let sk = EcdsaSigningKey::random(&mut OsRng);
        let vk = *sk.verifying_key();
        let pem = vk
            .to_public_key_pem(p256::pkcs8::LineEnding::LF)
            .expect("encode pubkey pem");

        let dir = tempfile::tempdir().unwrap();
        let arn = "arn:aws:kms:us-west-2:000:key/inline".to_string();
        let manifest = KeysManifest {
            keys: HashMap::from([(
                arn.clone(),
                KeyValidity {
                    valid_from: chrono::DateTime::<chrono::Utc>::MIN_UTC,
                    valid_until: None,
                    revoked: false,
                    revoked_at: None,
                    algorithm: Some("kms-ecdsa-sha256".into()),
                    public_key_pem: Some(pem),
                },
            )]),
        };
        std::fs::write(
            dir.path().join("keys.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let verifier = LocalEd25519Verifier::from_dir(dir.path()).unwrap();
        assert_eq!(verifier.key_count(), 1);

        // Sign + verify round-trip.
        use p256::ecdsa::{signature::Signer as EcdsaSignerTrait, Signature};
        let canonical = b"manifest-loaded round-trip";
        let digest = {
            let mut h = Sha256::new();
            h.update(canonical);
            h.finalize()
        };
        let sig: Signature = sk.sign(&digest);
        let sig_der = sig.to_der().to_bytes().to_vec();
        verifier
            .verify(&arn, canonical, &sig_der, None)
            .expect("manifest-loaded ECDSA verify should succeed");
    }

    #[tokio::test]
    async fn verifier_classifies_disabled_signer_rows() {
        let verifier = LocalEd25519Verifier::from_keys(HashMap::new());
        let err = verifier
            .verify("disabled:test-producer", b"x", b"sig", None)
            .unwrap_err();
        assert_eq!(err, VerifyFailure::Disabled);
    }

    #[tokio::test]
    async fn verifier_rejects_truncated_signature_bytes() {
        let (signer, verifier) = signer_and_verifier();
        let canonical = b"hello";
        let sig = signer.sign(canonical).await.unwrap();
        let truncated = &sig.bytes[..32];
        let err = verifier.verify(&sig.key_id, canonical, truncated, None).unwrap_err();
        assert_eq!(err, VerifyFailure::InvalidSignature);
    }

    #[tokio::test]
    async fn verifier_loads_keys_from_filesystem_directory_regardless_of_filename() {
        use ed25519_dalek::pkcs8::EncodePrivateKey;
        use std::fs;

        let dir = tempfile::tempdir().unwrap();
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let pem = signing_key
            .to_pkcs8_pem(ed25519_dalek::pkcs8::spki::der::pem::LineEnding::LF)
            .unwrap();
        let signer = LocalEd25519Signer::from_key(signing_key, "fs-producer".into());
        // File name irrelevant — verifier derives key_id from key bytes.
        // Use a service-style name like the demo's pki-init writes.
        fs::write(dir.path().join("sidecar.pem"), pem.as_bytes()).unwrap();

        let verifier = LocalEd25519Verifier::from_dir(dir.path()).unwrap();
        assert_eq!(verifier.key_count(), 1);

        let sig = signer.sign(b"payload").await.unwrap();
        verifier
            .verify(&sig.key_id, b"payload", &sig.bytes, None)
            .expect("disk-loaded key must verify regardless of filename");
    }

    // -----------------------------------------------------------------
    // S7 validity window tests
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn verifier_rejects_signature_when_event_time_before_valid_from() {
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let verifying_key = signing_key.verifying_key();
        let signer = LocalEd25519Signer::from_key(signing_key, "p".into());
        let mut keys = HashMap::new();
        keys.insert(signer.key_id().to_string(), verifying_key);
        let mut validities = HashMap::new();
        validities.insert(
            signer.key_id().to_string(),
            KeyValidity {
                valid_from: chrono::Utc::now(),
                valid_until: None,
                revoked: false,
                revoked_at: None,
                algorithm: None,
                public_key_pem: None,
            },
        );
        let verifier = LocalEd25519Verifier::from_keys_with_validity(keys, validities);

        let sig = signer.sign(b"x").await.unwrap();
        let stale = chrono::Utc::now() - chrono::Duration::seconds(3600);
        let err = verifier
            .verify(&sig.key_id, b"x", &sig.bytes, Some(stale))
            .unwrap_err();
        assert_eq!(err, VerifyFailure::KeyNotYetValid);
    }

    #[tokio::test]
    async fn verifier_rejects_signature_when_event_time_after_valid_until() {
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let verifying_key = signing_key.verifying_key();
        let signer = LocalEd25519Signer::from_key(signing_key, "p".into());
        let mut keys = HashMap::new();
        keys.insert(signer.key_id().to_string(), verifying_key);
        let mut validities = HashMap::new();
        validities.insert(
            signer.key_id().to_string(),
            KeyValidity {
                valid_from: chrono::Utc::now() - chrono::Duration::days(7),
                valid_until: Some(chrono::Utc::now() - chrono::Duration::days(1)),
                revoked: false,
                revoked_at: None,
                algorithm: None,
                public_key_pem: None,
            },
        );
        let verifier = LocalEd25519Verifier::from_keys_with_validity(keys, validities);

        let sig = signer.sign(b"x").await.unwrap();
        let now = chrono::Utc::now();
        let err = verifier
            .verify(&sig.key_id, b"x", &sig.bytes, Some(now))
            .unwrap_err();
        assert_eq!(err, VerifyFailure::KeyExpired);
    }

    #[tokio::test]
    async fn verifier_rejects_signature_when_key_revoked() {
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let verifying_key = signing_key.verifying_key();
        let signer = LocalEd25519Signer::from_key(signing_key, "p".into());
        let mut keys = HashMap::new();
        keys.insert(signer.key_id().to_string(), verifying_key);
        let mut validities = HashMap::new();
        validities.insert(
            signer.key_id().to_string(),
            KeyValidity {
                valid_from: chrono::DateTime::<chrono::Utc>::MIN_UTC,
                valid_until: None,
                revoked: true,
                revoked_at: Some(chrono::Utc::now()),
                algorithm: None,
                public_key_pem: None,
            },
        );
        let verifier = LocalEd25519Verifier::from_keys_with_validity(keys, validities);

        let sig = signer.sign(b"x").await.unwrap();
        let err = verifier
            .verify(&sig.key_id, b"x", &sig.bytes, Some(chrono::Utc::now()))
            .unwrap_err();
        assert_eq!(err, VerifyFailure::KeyRevoked);
    }

    #[tokio::test]
    async fn verifier_accepts_signature_when_event_time_inside_window() {
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let verifying_key = signing_key.verifying_key();
        let signer = LocalEd25519Signer::from_key(signing_key, "p".into());
        let mut keys = HashMap::new();
        keys.insert(signer.key_id().to_string(), verifying_key);
        let mut validities = HashMap::new();
        validities.insert(
            signer.key_id().to_string(),
            KeyValidity {
                valid_from: chrono::Utc::now() - chrono::Duration::days(1),
                valid_until: Some(chrono::Utc::now() + chrono::Duration::days(1)),
                revoked: false,
                revoked_at: None,
                algorithm: None,
                public_key_pem: None,
            },
        );
        let verifier = LocalEd25519Verifier::from_keys_with_validity(keys, validities);

        let sig = signer.sign(b"x").await.unwrap();
        verifier
            .verify(&sig.key_id, b"x", &sig.bytes, Some(chrono::Utc::now()))
            .expect("event_time inside window should pass");
    }

    #[tokio::test]
    async fn verifier_skips_window_check_when_event_time_is_none() {
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let verifying_key = signing_key.verifying_key();
        let signer = LocalEd25519Signer::from_key(signing_key, "p".into());
        let mut keys = HashMap::new();
        keys.insert(signer.key_id().to_string(), verifying_key);
        let mut validities = HashMap::new();
        validities.insert(
            signer.key_id().to_string(),
            KeyValidity {
                valid_from: chrono::Utc::now() - chrono::Duration::days(7),
                valid_until: Some(chrono::Utc::now() - chrono::Duration::days(1)),
                revoked: false,
                revoked_at: None,
                algorithm: None,
                public_key_pem: None,
            },
        );
        let verifier = LocalEd25519Verifier::from_keys_with_validity(keys, validities);

        let sig = signer.sign(b"x").await.unwrap();
        // event_time = None bypasses the window check (caller may not
        // know the time, e.g. background re-verification of old rows).
        verifier
            .verify(&sig.key_id, b"x", &sig.bytes, None)
            .expect("None event_time should skip window");
    }

    #[tokio::test]
    async fn verifier_revoked_check_runs_even_when_event_time_is_none() {
        // Revocation is operator-driven (incident response). Even
        // without an event_time, the revoked flag must still apply —
        // otherwise an attacker could omit time and bypass revocation.
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let verifying_key = signing_key.verifying_key();
        let signer = LocalEd25519Signer::from_key(signing_key, "p".into());
        let mut keys = HashMap::new();
        keys.insert(signer.key_id().to_string(), verifying_key);
        let mut validities = HashMap::new();
        validities.insert(
            signer.key_id().to_string(),
            KeyValidity {
                valid_from: chrono::DateTime::<chrono::Utc>::MIN_UTC,
                valid_until: None,
                revoked: true,
                revoked_at: Some(chrono::Utc::now()),
                algorithm: None,
                public_key_pem: None,
            },
        );
        let verifier = LocalEd25519Verifier::from_keys_with_validity(keys, validities);

        let sig = signer.sign(b"x").await.unwrap();
        let err = verifier
            .verify(&sig.key_id, b"x", &sig.bytes, None)
            .unwrap_err();
        assert_eq!(err, VerifyFailure::KeyRevoked);
    }

    #[test]
    fn keys_manifest_round_trips_through_json() {
        let mut keys = HashMap::new();
        keys.insert(
            "ed25519:abc".to_string(),
            KeyValidity {
                valid_from: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
                valid_until: Some(
                    chrono::DateTime::parse_from_rfc3339("2027-01-01T00:00:00Z")
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                ),
                revoked: false,
                revoked_at: None,
                algorithm: None,
                public_key_pem: None,
            },
        );
        let manifest = KeysManifest { keys };
        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: KeysManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.keys.len(), 1);
    }

    #[tokio::test]
    async fn verifier_loads_keys_json_manifest_from_dir() {
        use ed25519_dalek::pkcs8::EncodePrivateKey;
        use std::fs;

        let dir = tempfile::tempdir().unwrap();
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let pem = signing_key
            .to_pkcs8_pem(ed25519_dalek::pkcs8::spki::der::pem::LineEnding::LF)
            .unwrap();
        let signer = LocalEd25519Signer::from_key(signing_key, "p".into());
        fs::write(dir.path().join("k.pem"), pem.as_bytes()).unwrap();

        let mut validities = HashMap::new();
        validities.insert(
            signer.key_id().to_string(),
            KeyValidity {
                valid_from: chrono::DateTime::<chrono::Utc>::MIN_UTC,
                valid_until: None,
                revoked: true,
                revoked_at: Some(chrono::Utc::now()),
                algorithm: None,
                public_key_pem: None,
            },
        );
        let manifest = KeysManifest { keys: validities };
        fs::write(
            dir.path().join("keys.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();

        let verifier = LocalEd25519Verifier::from_dir(dir.path()).unwrap();
        assert_eq!(verifier.key_count(), 1);

        let sig = signer.sign(b"x").await.unwrap();
        let err = verifier
            .verify(&sig.key_id, b"x", &sig.bytes, Some(chrono::Utc::now()))
            .unwrap_err();
        assert_eq!(err, VerifyFailure::KeyRevoked);
    }

    #[test]
    fn key_validity_failure_strings_are_stable() {
        assert_eq!(VerifyFailure::KeyExpired.as_str(), "key_expired");
        assert_eq!(VerifyFailure::KeyNotYetValid.as_str(), "key_not_yet_valid");
        assert_eq!(VerifyFailure::KeyRevoked.as_str(), "key_revoked");
    }

    #[test]
    fn verifier_skips_non_pem_files() {
        use std::fs;
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("not-a-key.txt"), "garbage").unwrap();
        fs::write(dir.path().join("another.json"), "{}").unwrap();
        let verifier = LocalEd25519Verifier::from_dir(dir.path()).unwrap();
        assert_eq!(verifier.key_count(), 0);
    }

    #[test]
    fn verify_failure_stringification_is_stable() {
        // Quarantine table stores as_str(); stability is part of the
        // forensics contract.
        assert_eq!(VerifyFailure::UnknownKey.as_str(), "unknown_key");
        assert_eq!(VerifyFailure::InvalidSignature.as_str(), "invalid_signature");
        assert_eq!(VerifyFailure::PreS6.as_str(), "pre_s6");
        assert_eq!(VerifyFailure::Disabled.as_str(), "disabled");
    }
}
