# GA_02 Versioning, Changelog, and Release Notes Evidence

Date: 2026-05-31
Branch: `ga/GA_02_versioning_changelog_release_notes`
Tested head: `7d6d057fc75f01b50dbd025960efeb5e2ef96562`

## Commands

```bash
scripts/release/prepare-release-notes.sh --check-template docs/release/release-notes-template.md
commit=$(git rev-parse HEAD)
tmp=$(mktemp)
scripts/release/prepare-release-notes.sh --version v2026.05.31-ga.1 --commit "$commit" --output "$tmp"
scripts/release/prepare-release-notes.sh --check "$tmp" # after filling required sections
scripts/release/prepare-release-notes.sh --check-tag v2099.12.31-ga.0
scripts/release/prepare-release-notes.sh --version 2026.05.31 --commit "$commit" --output /tmp/bad-notes.md
scripts/release/prepare-release-notes.sh --check <template-without-migrations-section>
scripts/release/build-release-bundle.sh --output /tmp/spendguard-ga02-release
scripts/release/check-release-bundle.sh /tmp/spendguard-ga02-release
helm template spendguard charts/spendguard --set chart.profile=demo
helm template spendguard charts/spendguard -f scripts/helm-validate-test-values.yaml
```

## Result

- Release notes template check: PASS
- Generated release notes placeholder check: PASS; final `--check` requires filled sections
- Tag availability dry-run: PASS
- Invalid version negative gate: PASS, failed closed
- Missing `Migrations` section negative gate: PASS, failed closed
- Changelog includes predictor upgrade and HARDEN summary: PASS
- Versioning policy forbids ambiguous mutable release identifiers: PASS
- GA_01 bundle release-notes pointer remains `docs/release/release-notes-template.md`: PASS
- Bundle build/check: PASS
- Helm demo and production validation renders: PASS
- Bundle checksum manifest checksum: `f674f1add61536df7047162186833b1e2dc3cb6220e1ada4dfbee0b2170508f7`
