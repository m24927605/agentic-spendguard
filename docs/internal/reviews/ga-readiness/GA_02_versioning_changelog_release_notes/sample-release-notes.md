# SpendGuard Release Notes

> **Release**: `v2026.05.31-ga.1`
> **Commit**: `cce30e12759d345281ceb5287567acf4dd6f535d`
> **Date**: `2026-05-31`

## Summary

GA readiness release-note validation covers the hardened predictor upgrade baseline and confirms operator-facing release sections are filled before publication.

## Breaking Changes

None for this sample validation note. The GA_02 validator still requires this section so a real release has an explicit breaking-change decision.

## Migrations

Apply ledger, canonical ingest, and control-plane migrations in documented order. Treat immutable audit history as forward-fix only.

## Helm Values

Production Helm values must reference Kubernetes Secrets for database URLs and signing material. Strategy C production deployments require per-tenant SVID bindings.

## Operator Actions

Run release bundle verification, Helm production render, migration verification, and the relevant demo gates before publishing a release.

## Security Notes

Container security baseline, RLS, replay protection, and SVID validation remain required. SBOM, image signing, and vulnerability scan evidence are owned by GA_09.

## Rollback

Rollback must follow migration classification and must not delete immutable audit rows. Use forward-fix for audit-chain data corrections.

## Verification

This sample is validated by `scripts/release/prepare-release-notes.sh --check` as GA_02 evidence.
