# SpendGuard Release Notes Template

> **Release**: `<version>`
> **Commit**: `<40-character git SHA>`
> **Date**: `<YYYY-MM-DD>`

<!--
Required by scripts/release/prepare-release-notes.sh:
- release version matching vYYYY.MM.DD-ga.N
- 40-character commit SHA
- date
- all sections below
Replace placeholders before publishing generated release notes.
-->

## Summary

Describe the customer-visible release outcome.

## Breaking Changes

List breaking API, chart, migration, or operator behavior changes. Write `None` only after checking.

## Migrations

List migration commands, preflight checks, backup requirements, and forward-fix-only caveats.

## Helm Values

List required production values, Secret references, cert-manager/SVID settings, and security-sensitive defaults.

## Operator Actions

List commands or manual steps operators must perform before, during, and after upgrade.

## Security Notes

List security-relevant changes, supply-chain evidence, known residual risks, and required operator actions.

## Rollback

Describe rollback or forward-fix procedure. Do not claim data rollback is safe for immutable audit tables unless a tested rollback path exists.

## Verification

List release gates run, evidence paths, and command summaries.
