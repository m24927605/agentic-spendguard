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

This command is non-destructive and checks whether a candidate tag already exists:

```bash
git rev-parse -q --verify refs/tags/v2026.05.31-ga.0 >/dev/null && echo "tag exists" || echo "tag available"
```

Creating or pushing the tag is a maintainer release action and is not performed by GA_02 automation.

## Changelog Requirements

Every product release entry must include:

- summary
- operator highlights
- migration notes
- Helm/config notes when applicable
- security notes
- rollback or forward-fix caveats
