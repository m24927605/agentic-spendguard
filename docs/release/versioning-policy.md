# SpendGuard Versioning Policy

## Version Format

GA readiness release candidates and GA releases use:

```text
vYYYY.MM.DD-ga.N
```

Examples:

- `v2026.05.31-ga.0`
- `v2026.06.01-ga.1`

`N` is a monotonically increasing integer for releases cut on the same calendar date. Tags are immutable after publication. Never move or overwrite a published tag.

## What A Version Identifies

A product version identifies:

- one git commit
- one Helm chart package
- one migration inventory
- one release notes document
- one release bundle
- one security evidence set once GA_09 lands

## Prohibited Language

Do not use `latest`, `current`, or `stable` as a release identifier in operator instructions. Use exact versions and exact commit SHA values.

## Dry-Run Tag Check

Use the release notes helper for tag availability checks:

```bash
scripts/release/prepare-release-notes.sh --check-tag v2026.05.31-ga.0
```

The helper rejects invalid calendar dates, existing local tags, and existing `origin` remote tags. A local-only `git rev-parse refs/tags/...` check is not sufficient for GA release decisions because a published tag may exist only on the remote. Creating or pushing the tag is a maintainer release action and is not performed by GA_02 automation.

## Changelog Requirements

Every product release entry must include:

- summary
- operator highlights
- migration notes
- Helm/config notes when applicable
- security notes
- rollback or forward-fix caveats
