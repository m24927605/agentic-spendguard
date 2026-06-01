# SpendGuard Release Artifacts

This directory defines the local GA release bundle format. The bundle is an operator-facing artifact set that maps a release candidate back to an exact git commit, Helm chart package, migration inventory, release notes pointer, and checksum manifest.

The canonical builder is:

```bash
scripts/release/build-release-bundle.sh --output /tmp/spendguard-ga-release
```

The canonical verifier is:

```bash
scripts/release/check-release-bundle.sh /tmp/spendguard-ga-release
```

The scripts are intentionally local and fail closed. They do not publish a GitHub Release, create a tag, push images, or sign artifacts. Image signing, SBOM, vulnerability scanning, and provenance are owned by GA_09 through `.github/workflows/publish-images.yml` and `scripts/security/ga-security-scan.sh`.

## Bundle Layout

```text
spendguard-ga-release/
  commit.txt
  manifest.txt
  release-notes.pointer
  SHA256SUMS
  charts/
    spendguard-<chart-version>.tgz
  migrations/
    inventory.txt
    inventory.sha256
  sbom/
    README.md
```

`commit.txt` is the exact git SHA. `manifest.txt` records branch, commit, build timestamp, chart version, and tool versions. `SHA256SUMS` covers every regular file in the bundle except itself.

## Secret Policy

Release bundles must not contain credentials, private keys, database URLs, API keys, or rendered Kubernetes Secrets. The checker scans for common secret patterns as a guardrail, and GA_09 security signoff is required before a bundle is promoted.
