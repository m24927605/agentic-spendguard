# GA Security Signoff

Status: GA_09 release gate
Owner: Staff+ Security Engineer
Applies to: Agentic SpendGuard predictor upgrade GA readiness

## Decision

Agentic SpendGuard can be promoted only when the GA_09 security scan passes and the release operator has run the release-mode external scanner gate:

```bash
scripts/security/ga-security-scan.sh --require-external-tools
```

Local development and slice acceptance may run the deterministic gate without external scanners:

```bash
scripts/security/ga-security-scan.sh
```

Release-mode is intentionally stricter. It fails closed unless `syft`, `trivy`, `cosign`, and `cargo-audit` are installed.

## Coverage

| Area | Required control | Evidence |
|---|---|---|
| SVID/mTLS | Strategy C plugin clients use exact per-tenant SPIFFE URI `spiffe://spendguard.platform/predictor-client/<tenant_id>` | `charts/spendguard/templates/output_predictor_plugin_svid.yaml`; `services/output_predictor/src/plugin_svid.rs`; GA_09 scan |
| Secrets | Production Helm values reference Secrets and never embed DB URLs, PEM material, API keys, or tokens | `scripts/release/validate-production-helm-values.sh`; GA_09 scan |
| RLS | Database writers use established `set_config` tenant context; executable migrations must not grant `BYPASSRLS` | GA_09 scan strips SQL comments and rejects executable `BYPASSRLS` grants |
| Replay protection | Canonical ingest reserves CloudEvent replay keys before immutable append | `services/canonical_ingest/migrations/0020_event_replay_dedup.sql`; `services/canonical_ingest/src/persistence/append.rs` |
| PII boundary | Tokenizer raw-text shadow calls default-deny and require tenant opt-in | `services/tokenizer/src/shadow/security.rs`; `services/tokenizer/src/shadow/worker.rs` |
| Provider quota | `count_tokens` calls are tenant-quota guarded | `services/tokenizer/src/shadow/security.rs` |
| Containers | Runtime images set `USER 65532:65532`; Helm renders `runAsUser=65532`, `readOnlyRootFilesystem=true`, `allowPrivilegeEscalation=false`, and `capabilities.drop=[ALL]` | runtime Dockerfiles; Helm production validator |
| Supply chain | Published images are scanned, SBOM/provenance-attested, and signed by digest; mutable `latest` promotion is forbidden | `.github/workflows/publish-images.yml`; `docs/security/supply-chain.md` |

## Release Blockers

The release is blocked if any of these conditions hold:

- GA_09 scan result is not `pass`.
- Production Helm render contains plaintext DB URLs or missing Secret references.
- Any published runtime Dockerfile lacks `USER 65532:65532`.
- The publish workflow reintroduces `latest` or `latest-main` image promotion.
- The publish workflow lacks Trivy scan, BuildKit SBOM, BuildKit provenance, keyless cosign signing, or OIDC permission.
- Executable SQL grants `BYPASSRLS`.
- Per-tenant SVID URI generation or validation drifts from the exact predictor-client prefix.
- Tokenizer PII shadow default becomes allow-by-default.

## Residual Risk

Third-party penetration testing is still outside this repo-local GA phase. That is an external assurance activity, not a waiver for any known high-severity implementation finding.

## Staff+ Signoff

| Role | Decision |
|---|---|
| Security Engineer | GA promotion requires release-mode scanner gate and no unhandled high/critical findings |
| Release Engineering Architect | Image signing/SBOM/provenance live in the publish workflow; the release bundle points to that evidence |
| Backend Architect | SVID, RLS, replay, and PII invariants remain runtime gates, not documentation-only promises |
| SRE/Operations Architect | Helm render validation and local security scan are operator-reproducible |
