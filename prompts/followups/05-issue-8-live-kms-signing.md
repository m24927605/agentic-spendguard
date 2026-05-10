# Followup #8 — Live KMS signing backend (S6)

GitHub issue: https://github.com/m24927605/agentic-flow-cost-evaluation/issues/8

## Goal

Replace `KmsSigner` stub in `services/signing/src/signer.rs` with a real AWS
KMS `Sign` integration (default cloud). Today it returns the typed
"unsupported" error; production sites that prefer cloud-managed keys can't
use signing. Helm fail-gates already block `signing.mode=disabled` outside
demo (PR #2 commit `b6c1aa3`) and require strict verification under
`signing.profile=production` (commit `ebebbd8`).

The Codex round 7+8 SP relaxations (migrations 0031, 0032) already accept
any signer's key_id and signature, so the SP-side is ready; only the
producer side needs implementing.

## Files to read first

- `services/signing/src/signer.rs` — full file:
  - `Signer` trait
  - `LocalEd25519Signer` (reference for trait contract)
  - `KmsSigner` stub
  - `DisabledSigner`
- `services/signing/src/key_validity.rs` — S7 KeyValidity registry; KMS
  signer must respect the same windowing
- `services/canonical_ingest/src/verifier.rs` — `Verifier` trait; the
  canonical_ingest side must verify whatever KmsSigner produces (today
  Ed25519; KMS asymmetric keys are typically RSA or ECDSA-P256)
- `charts/spendguard/values.yaml` — `signing.kms.{kmsKeyArn, awsRegion}`
  knobs already exist; wire them through
- PR #2 commits `409c220` (round 7) + `d019e94` (round 8) — SP relaxations
  that accept arbitrary key_id and signature

## Acceptance criteria

- AWS KMS first; GCP KMS as a separate followup. New deps:
  `aws-sdk-kms = "1"` + `aws-config = "1"` to `services/signing/Cargo.toml`
- `KmsSigner` struct holds: `key_arn: String`, `key_id_label: String`
  (operator-supplied stable identifier shown in audit, e.g.
  `kms:arn:aws:kms:us-east-1:.../alias/spendguard-prod`), `algorithm:
  KmsAlgorithm` (start with `ECDSA_SHA_256` since AWS Ed25519 KMS keys are
  not yet GA in all regions; revisit if Ed25519 KMS lands)
- `Signer::sign(bytes) -> SignedBytes`:
  - Calls `kms.sign().key_id(arn).message(bytes).message_type(Raw).signing_algorithm(ECDSA_SHA_256).send()`
  - Returns `SignedBytes { key_id: self.key_id_label, bytes: <raw signature> }`
- Key validity: at construct time, fetch the key's metadata
  (`describe_key`) and feed expiry / rotation-time into `KeyValidity`
  per S7
- canonical_ingest verifier extension: add `KmsEcdsaVerifier` variant that
  verifies ECDSA-P256 signatures against the operator-supplied public key
  (downloaded once at startup via `kms.get_public_key`, cached with the
  key_id in the trust store)
- Helm `chart.profile=production` + `signing.mode=kms` produces a
  spendguard-sidecar pod that successfully writes a real KMS-signed
  audit_outbox row, the canonical_ingest pod accepts it on the JSON
  canonical-form path (round-11 producer_id "ledger:*" gating doesn't
  apply to sidecar's "sidecar:*" path; verify both work)
- LocalStack-based integration test:
  - Spin up `localstack-pro/localstack` with KMS enabled
  - Create an asymmetric ECDSA-P256 key
  - Sign + verify round-trip via the new KmsSigner + KmsEcdsaVerifier
- `cargo check` clean

## Pattern references

- `LocalEd25519Signer` is the gold-standard trait impl for in-process
  signing
- `services/canonical_ingest/src/verifier.rs:Verifier` shows how new
  signing modes plug into the verification side

## Verification

```bash
cargo test -p spendguard-signing
# and the LocalStack KMS integration test
docker run --rm -d --name sg-localstack-kms -e SERVICES=kms -p 4566:4566 localstack/localstack:3.5
SPENDGUARD_SIGNING_MODE=kms \
SPENDGUARD_SIGNING_KMS_ARN=arn:aws:kms:us-east-1:000000000000:key/... \
AWS_ENDPOINT_URL=http://localhost:4566 \
cargo run --bin spendguard-signing-cli -- sign-and-verify "hello"
# expect: ok
docker rm -f sg-localstack-kms
```

GCP KMS path deferred to a separate followup.

## Commit + close

```
feat(s6): live AWS KMS signing backend (followup #8)

Replaces the KmsSigner stub with a real aws-sdk-kms integration
using ECDSA_SHA_256. Key validity windowing fetched from
describe_key; canonical_ingest verifier gains a KmsEcdsaVerifier
variant that downloads the public key once via get_public_key.

Helm signing.mode=kms now produces real signed audit rows under
production profile.

Tests: LocalStack KMS round-trip; SP relaxations from PR #2 round
7+8 already accept the new key_id format.
```

After merge: `gh issue close 8 --comment "Shipped (AWS path) in <commit-sha>; GCP deferred."`
