# Security Policy

SpendGuard is a fail-closed financial-control product: it gates LLM spend and
keeps a signed, tamper-evident audit chain. We take the security of the
decision path, the ledger, and the audit chain seriously.

## Reporting a vulnerability

**Please do not report security issues in public GitHub issues, pull
requests, or discussions.**

Instead, use **GitHub's private vulnerability reporting**: open the
repository's **Security** tab → **Report a vulnerability**. This opens a
private advisory visible only to the maintainers.

Please include:

- A description of the issue and its impact (e.g. a fail-open path, an audit
  bypass, a tenant-isolation break, double-spend/under-count).
- Affected component(s) and version/commit.
- Steps to reproduce or a proof of concept, if available.

We aim to acknowledge a report within a few business days and will keep you
updated on remediation. Please give us a reasonable window to ship a fix
before any public disclosure; we're happy to credit you when the fix lands.

## What we consider high severity

Because the product's job is to refuse spend and prove it, the following are
treated as high or critical:

- A `DENY` that can be turned into an `ALLOW` (fail-open on the decision,
  reservation, commit, or release path).
- Driving a budget over its cap (double-spend, under-count, or bypass of the
  ledger `BUDGET_EXHAUSTED` hard cap).
- Dropping, forging, or mutating an audit record, or otherwise defeating the
  append-only / signature / canonical-ingest verification layers.
- Cross-tenant access, or trusting client-supplied identity (tenant, workload)
  without the configured mTLS/SPIFFE binding.

## Scope and posture

- Every internal service surface is mTLS. Audit rows are append-only (Postgres
  immutability triggers) and signed (Ed25519 locally, AWS KMS ECDSA P-256 in
  production).
- The ledger — not the proxy or sidecar — is the authority for the hard cap.

For the deployed threat model and supply-chain posture, see:

- [`docs/security/threat-model-ga.md`](docs/security/threat-model-ga.md)
- [`docs/security/supply-chain.md`](docs/security/supply-chain.md)

## Supported versions

This project is pre-1.0 and ships from `main`. Security fixes are applied to
`main` and the latest published SDK releases. Pin a released version for
production use and watch the repository for advisories.
