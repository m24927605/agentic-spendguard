# GA 09 - Security Signoff and Supply Chain

> **Branch**: `ga/GA_09_security_signoff_supply_chain`
> **Status**: design
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
- Add evidence under `docs/reviews/ga-readiness/GA_09_security_signoff_supply_chain/`

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

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must treat unhandled high-severity findings as blockers.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Security Engineer | Security signoff is a GA gate, not post-GA work | GA_09 dedicated slice |
| Release Engineering Architect | Supply-chain evidence ties back to release bundle | GA_01/GA_09 cross-link |

## §14. Merge Checklist

- [ ] Security signoff exists
- [ ] Scan script passes or fails closed with documented missing tool
- [ ] Helm security baseline still passes
- [ ] AIT review clean or arbitration recorded
- [ ] Memory updated
