# GA 09 - Security Signoff and Supply Chain

> **Branch**: `ga/GA_09_security_signoff_supply_chain`
> **Status**: implemented; Staff+ arbitration accepted final fix
> **Spec ancestor(s)**: `ga-readiness-spec-v1alpha1.md`
> **Estimated change size**: medium; threat model, scan scripts, supply-chain docs

---

## §0. TL;DR

Produce independent GA security signoff covering SVID/mTLS, secrets, RLS, replay protection, PII boundaries, container baseline, SBOM, vulnerability scan, and image signing readiness.

## §1. Architectural Context

HARDEN closed known security blockers. GA still needs a release-level signoff artifact and supply-chain gates that operators can reproduce.

## §2. Scope

- Security signoff document
- Threat model checklist
- SBOM generation or fail-closed missing-tool path
- Vulnerability scan script
- Image signing readiness docs
- Secret rotation checklist

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| External penetration test | Future external audit |
| Managed KMS provisioning | Operator/provider-specific |

## §4. File-Level Changes

- Add `docs/security/ga-security-signoff.md`
- Add `docs/security/threat-model-ga.md`
- Add `docs/security/supply-chain.md`
- Add `scripts/security/ga-security-scan.sh`
- Update `.github/workflows/publish-images.yml` if image signing, SBOM, provenance, or scan gates are not wired
- Add evidence under `docs/internal/reviews/ga-readiness/GA_09_security_signoff_supply_chain/`

## §5. Schema / Config / API Impact

No schema changes expected. If scan findings require code changes, keep them in-slice.

## §6. Audit / Security / Operational Impact

This is the release security gate. It must not waive known high-severity findings without Staff+ arbitration.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| SBOM tool unavailable | Script fails closed or documents install path |
| Critical vulnerability found | Slice blocks or fixes in-slice |
| Helm example contains secret | Slice blocks |
| RLS/audit invariant drift | Slice blocks |

## §8. Acceptance Gates

- `scripts/security/ga-security-scan.sh`
- Helm demo and production renders pass
- Security signoff explicitly covers SVID, secrets, RLS, replay, PII, containers, and supply chain
- Evidence includes scanner versions and results
- Image workflow either verifies signing/SBOM/vulnerability gates or documents a fail-closed local release gate with exact commands

Local evidence on 2026-06-01:

- `scripts/security/ga-security-scan.sh --require-external-tools` passed after R2 security evidence hardening fixes.
- External tools installed and recorded: Syft 1.44.0, Trivy 0.70.0, Cosign 3.0.6, cargo-audit 0.22.1.
- `cargo-audit.json` reports 0 vulnerabilities and no warnings.
- `trivy-fs.json` reports 0 high/critical vulnerabilities for `Cargo.lock`.
- `helm template spendguard charts/spendguard --set chart.profile=demo` passed.
- `helm template spendguard charts/spendguard -f charts/spendguard/values-production.example.yaml --set chart.profile=production` passed.
- `scripts/release/validate-production-helm-values.sh charts/spendguard/values-production.example.yaml --rendered-manifest /tmp/ga09-helm-prod.yaml` passed with negative tests failed closed.
- `scripts/release/build-release-bundle.sh --output /tmp/spendguard-ga09-release` passed.
- `scripts/release/check-release-bundle.sh /tmp/spendguard-ga09-release` passed.
- `make demo-up DEMO_MODE=default` passed after R2 volume handoff fixes; demo completed handshake, decision, provider_report, Step 8 SQL assertions, outbox closure, and canonical_events forwarding checks.
- Upgrade simulation manually root-owned the existing `spendguard-demo_sidecar-uds` named volume, then reran `make demo-up DEMO_MODE=default`; `sidecar-uds-init` handed the volume to `65532:65532`, sidecar bound the UDS, and runtime RPCs succeeded. A non-clean SQL assertion failed only because previous demo rows remained; clean rerun passed.

R1 regression coverage added after adversarial review:

- `pki-init` now hands `/etc/ssl/spendguard` cert/key and signing-key volume contents to UID/GID `65532:65532` while preserving `0640` private-key permissions.
- `bundles-init` now hands `/var/lib/spendguard/bundles` to UID/GID `65532:65532` on both generate and idempotent skip paths, preserving bundle-registry hot-reload writes.
- `Dockerfile.sidecar` pre-creates `/var/run/secrets/spendguard` symlinks and `/var/run/spendguard` before `USER 65532:65532`.
- `sidecar-entrypoint.sh` no longer creates root-owned paths, mutates the OS trust store, or chmods the UDS directory after the USER switch.
- `scripts/security/ga-security-scan.sh` now fails if the above non-root runtime invariants regress.

R2 regression coverage added after adversarial review:

- `pki-init` keeps the demo CA private key `root:root 0600` after handing runtime-readable workload cert/key material to UID/GID `65532:65532`.
- `compose.yaml` adds `sidecar-uds-init`, a one-shot volume handoff service that chowns existing sidecar UDS named volumes before the non-root sidecar starts.
- `scripts/security/ga-security-scan.sh` sanitizes `cargo metadata` and local Cargo SBOM evidence so committed evidence strips developer-local absolute paths.
- `scripts/security/ga-security-scan.sh` now fails if the CA-key isolation, sidecar UDS volume handoff, or cargo evidence path-sanitization invariants regress.

R3 regression coverage added after adversarial review:

- `.github/workflows/publish-images.yml` now publishes/signs the same `ghcr.io/<owner>/spendguard/<component>` image refs that production Helm renders.
- The publish matrix covers every first-party production-chart component in `values-production.example.yaml`: canonical-ingest, control-plane, egress-proxy, ledger, outbox-forwarder, output-predictor, run-cost-projector, sidecar, stats-aggregator, tokenizer, ttl-sweeper, and webhook-receiver.
- `scripts/security/ga-security-scan.sh --require-external-tools` now fails closed on a dirty worktree before writing evidence, preventing a PASS summary from being attributed to an unscanned HEAD.
- `scripts/security/ga-security-scan.sh` now compares the production Helm render image set with the publish workflow matrix.

R4 regression coverage added after adversarial review:

- `scripts/security/ga-security-scan.sh --require-external-tools --output-dir <new repo path>` now checks clean worktree before creating the evidence directory, so the intended output path cannot make the precondition fail itself.
- Default local scan mode now records optional external scanner execution failures instead of aborting before `scan-summary.json`; release mode still fails closed on missing or failed Syft, Trivy, Cosign, or cargo-audit.
- `.github/workflows/publish-images.yml` now runs the repository Trivy scan once in a pre-matrix job and gates the image matrix on that job.
- `scripts/security/ga-security-scan.sh` now fails if the repository scan is moved back into the image matrix.

R5 Staff+ arbitration coverage:

- Round 5 adversarial review found one remaining P2: `production-helm-validator.txt` captured a developer-local absolute path because `ga-security-scan.sh` canonicalized `output_dir` before logging the validator command.
- Staff+ Software Architect, Backend Architect, Security Engineer, Database Optimizer, and SpendGuard domain expert each voted `fix anyway`; no panel member accepted it as out-of-scope.
- `scripts/security/ga-security-scan.sh` now passes a repo-relative `--rendered-manifest` path when possible, sanitizes generated evidence files, and scans the whole GA_09 evidence bundle for developer-local root/home paths before reporting PASS.

Evidence:

- `docs/internal/reviews/ga-readiness/GA_09_security_signoff_supply_chain/README.md`
- `docs/internal/reviews/ga-readiness/GA_09_security_signoff_supply_chain/scan-summary.json`
- `docs/internal/reviews/ga-readiness/GA_09_security_signoff_supply_chain/syft-sbom.spdx.json`
- `docs/internal/reviews/ga-readiness/GA_09_security_signoff_supply_chain/trivy-fs.json`
- `docs/internal/reviews/ga-readiness/GA_09_security_signoff_supply_chain/cargo-audit.json`

## §9. Review Checklist

1. Are critical/high findings handled?
2. Are secrets absent from examples and bundles?
3. Does SVID trust chain match HARDEN_08 implementation?
4. Are RLS and audit immutability not weakened?
5. Are SBOM and scan outputs reproducible?
6. Are image tags digest-pinned or explicitly prevented from mutable `latest` promotion?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Third-party pen test | Requires external vendor |

## §11. Risk / Rollback

Revert docs/scripts unless a security fix changes runtime code. Runtime fixes require normal rollback notes.

## §12. Review Execution Notes

Reviewer: codex CLI via `codex review --base main`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must treat unhandled high-severity findings as blockers.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Security Engineer | Security signoff is a GA gate, not post-GA work | GA_09 dedicated slice |
| Release Engineering Architect | Supply-chain evidence ties back to release bundle | GA_01/GA_09 cross-link |
| Security Engineer | Runtime image `USER 65532:65532` is required even when Helm sets `runAsUser=65532` | Added USER to every `deploy/demo/runtime/Dockerfile.*` |
| Release Engineering Architect | Mutable `latest` and `latest-main` promotion are incompatible with GA signoff | Publish workflow now emits sha/tag refs only and signs pushed digests |
| Release Engineering Architect | Release bundle gate must include migrations added after GA_04 | Refreshed migration inventory for canonical `0021` and ledger `0053` |
| Security Engineer | R1 P1/P2 findings showed image `USER 65532` must be paired with compose volume ownership handoff | Fixed PKI, bundle, and sidecar runtime ownership and added scan guards |
| Release Engineering Architect | Manual publish dispatch must still produce an immutable image tag | Restored workflow_dispatch `sha-<short>` tag without reintroducing mutable `latest` |
| Security Engineer | R2 found the demo CA private key must not become readable by runtime UID 65532 | Re-rooted `ca.key` to `root:root 0600` after PKI runtime handoff and added a scan guard |
| Platform Runtime Architect | R2 found named Docker volumes from pre-GA runs can mask image-level sidecar UDS ownership | Added `sidecar-uds-init` to repair the named volume before sidecar startup |
| Release Engineering Architect | R2 found committed evidence must not leak developer-local absolute paths | Sanitized cargo metadata/SBOM evidence and added a path-leak scan guard |
| Release Engineering Architect | R3 found the publish workflow signed old six-image refs instead of all production Helm refs | Aligned workflow repositories with `spendguard/<component>` and expanded the matrix to 12 production components |
| Security Engineer | R3 found release-mode scan could attribute dirty worktree output to HEAD | Added a clean-worktree precondition before release-mode evidence generation |
| Security Engineer | R4 found release-mode output directory creation could make a clean repo look dirty | Moved the clean-worktree gate before evidence directory creation |
| Security Engineer | R4 found default local scans could abort on optional cargo-audit fetch/lock errors | Default mode now records scanner execution failures while release mode fails closed |
| Release Engineering Architect | R4 found repository Trivy scan repeated across every image matrix entry | Split repository scan into a single pre-matrix job and added a scan invariant |
| Staff+ panel | R5 found GA_09 evidence leaked a developer-local absolute path after five review rounds | Panel unanimously voted `fix anyway`; scan now sanitizes evidence and guards the whole evidence bundle |

## §14. Merge Checklist

- [x] Security signoff exists
- [x] Scan script passes or fails closed with documented missing tool
- [x] Helm security baseline still passes
- [x] Codex review clean or arbitration recorded
- [ ] Memory updated
