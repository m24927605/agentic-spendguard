# Supply Chain Security

Status: GA_09 release gate

## Image Publication

`.github/workflows/publish-images.yml` publishes every first-party image rendered by the production chart example under `ghcr.io/<owner>/spendguard/<component>`:

- `spendguard/canonical-ingest`
- `spendguard/control-plane`
- `spendguard/egress-proxy`
- `spendguard/ledger`
- `spendguard/outbox-forwarder`
- `spendguard/output-predictor`
- `spendguard/run-cost-projector`
- `spendguard/sidecar`
- `spendguard/stats-aggregator`
- `spendguard/tokenizer`
- `spendguard/ttl-sweeper`
- `spendguard/webhook-receiver`

Main branch pushes publish `sha-<short>` tags only. Version tag pushes publish the version tag only. The workflow intentionally does not publish `latest` or `latest-main`.

## Required Controls

| Control | Workflow requirement |
|---|---|
| Repository vulnerability scan | Single pre-matrix `aquasecurity/trivy-action` filesystem scan with `CRITICAL,HIGH` gate |
| Image vulnerability scan | `aquasecurity/trivy-action` scan against the pushed digest |
| SBOM | Docker Buildx `sbom` attestation enabled for pushed images |
| Provenance | Docker Buildx `provenance` attestation enabled for pushed images |
| Signing | `cosign sign --yes <image>@<digest>` after push |
| OIDC | Workflow has `id-token: write` for keyless signing |
| Mutable tag prevention | No `latest` or `latest-main` tag configuration |

## Local Release Gate

Run the full release signoff gate before promotion:

```bash
brew install syft trivy cosign cargo-audit
scripts/security/ga-security-scan.sh --require-external-tools
```

Release-mode requires a clean git worktree before any evidence directory is created or written, so the recorded `commit_sha` always identifies the scanned contents. The gate emits:

- `tool-versions.txt`
- `helm-demo.txt`
- `helm-production.yaml`
- `production-helm-validator.txt`
- `cargo-metadata.txt`
- `cargo-sbom.json`
- optional `syft-sbom.spdx.json`
- optional `trivy-fs.json`
- optional `cargo-audit.json`
- `scan-summary.json`
- `README.md`

Default local acceptance may run without external scanners:

```bash
scripts/security/ga-security-scan.sh
```

That mode still validates repository invariants, records missing tools, records optional external scanner execution failures, sanitizes developer-local filesystem paths from evidence files, and points release operators to the fail-closed release-mode command. Release-mode fails closed if any required scanner is missing or exits nonzero.

## Release Bundle Relationship

The release bundle carries chart, migration, checksum, and pointer metadata. It does not push or sign images itself. `sbom/README.md` in the bundle points operators to the GA_09 security evidence and the publish workflow attestations.

## Verification

Before promoting a release candidate:

1. Confirm `scripts/security/ga-security-scan.sh --require-external-tools` exits 0.
2. Confirm pushed image refs are digest-resolved in registry metadata.
3. Confirm `cosign verify` succeeds for each pushed digest under the repository's GitHub Actions OIDC identity.
4. Confirm Trivy high/critical gate is clean or documented as accepted by Staff+ security owner.
5. Confirm production Helm values use exact semver or digest tags and no mutable image aliases.
