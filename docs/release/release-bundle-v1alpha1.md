# Release Bundle v1alpha1

> **Status**: GA_01 candidate
> **Producer**: `scripts/release/build-release-bundle.sh`
> **Verifier**: `scripts/release/check-release-bundle.sh`

## Purpose

The release bundle is the minimal reproducible artifact set for a SpendGuard GA release candidate. It answers:

- which git commit is being released
- which Helm chart package was produced
- which SQL migrations belong to the release
- where release notes are expected
- whether local files match the recorded checksums

## Required Fields

| File | Requirement |
|---|---|
| `commit.txt` | Full 40-character git SHA |
| `manifest.txt` | Branch, commit, UTC timestamp, Helm version, chart version |
| `release-notes.pointer` | Path to the required release notes template or generated notes |
| `charts/*.tgz` | Helm package from `charts/spendguard` |
| `migrations/inventory.txt` | Sorted list of service migration SQL files with checksums |
| `migrations/inventory.sha256` | SHA-256 of `inventory.txt` |
| `sbom/README.md` | Pointer to GA_09 local security scan evidence plus image SBOM/provenance workflow |
| `SHA256SUMS` | Checksums for all bundle files except `SHA256SUMS` |

## Non-Goals

The v1alpha1 bundle does not push images or sign artifacts itself. GA_09 wires image signing, SBOM, vulnerability scanning, and provenance into `.github/workflows/publish-images.yml`, and local release signoff is produced by `scripts/security/ga-security-scan.sh --require-external-tools`.

## Validation

Run:

```bash
scripts/release/check-release-bundle.sh /tmp/spendguard-ga-release
```

The checker validates file presence, checksum integrity, basic commit format, chart package presence, migration inventory checksum, and common secret pattern absence.
