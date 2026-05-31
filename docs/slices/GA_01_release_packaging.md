# GA 01 - Release Packaging

> **Branch**: `ga/GA_01_release_packaging`
> **Status**: design
> **Spec ancestor(s)**: `ga-readiness-spec-v1alpha1.md`
> **Estimated change size**: small; release docs and scripts

---

## §0. TL;DR

Create deterministic release bundle packaging so an operator can map a GA artifact back to an exact main commit, chart render, migration inventory, and release notes pointer.

## §1. Architectural Context

HARDEN shipped working code, not a release artifact model. GA needs a repeatable bundle builder that fails closed on dirty state and missing deployment evidence.

## §2. Scope

- Release bundle layout
- Bundle build script
- Bundle validation script
- Artifact manifest with commit SHA and checksums
- Documentation under `docs/release/`

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| GitHub release publication | GA_02 |
| SBOM/signing implementation | GA_09 |
| Production values authoring | GA_03 |

## §4. File-Level Changes

- Add `docs/release/README.md`
- Add `docs/release/release-bundle-v1alpha1.md`
- Add `scripts/release/build-release-bundle.sh`
- Add `scripts/release/check-release-bundle.sh`
- Add evidence under `docs/reviews/ga-readiness/GA_01_release_packaging/`

## §5. Schema / Config / API Impact

No runtime schema or API changes.

## §6. Audit / Security / Operational Impact

The release manifest becomes the operator audit record for what was shipped. It must not contain secrets.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Dirty worktree | Build script exits non-zero |
| Missing Helm binary | Build script exits non-zero with clear message |
| Missing release notes pointer | Check script exits non-zero |
| Bundle checksum mismatch | Check script exits non-zero |

## §8. Acceptance Gates

- `scripts/release/build-release-bundle.sh --output /tmp/spendguard-ga-release`
- `scripts/release/check-release-bundle.sh /tmp/spendguard-ga-release`
- `helm template spendguard charts/spendguard --set chart.profile=demo`
- Production Helm render using the current validation values

## §9. Review Checklist

1. Does the bundle include exact commit SHA?
2. Are checksums deterministic?
3. Does the script fail on dirty state?
4. Is no secret material bundled?
5. Can a reviewer validate the bundle without local hidden state?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Container image signing | Owned by GA_09 |
| Release tag creation | Owned by GA_02 |

## §11. Risk / Rollback

Rollback is documentation-only: revert this slice if the artifact model is wrong. No runtime behavior changes.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must inspect script fail-closed behavior, checksum coverage, and absence of secrets.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Release Engineering Architect | Release packaging must be separate from versioning and release notes | Keeps GA_01 small |
| Security Engineer | Bundle manifest may include checksums, not credentials | Secret-free scope |
| Review R1 | codex CLI adversarial reviewer | Fix checksum coverage, chart authenticity, chart secret scanning, release notes target, manifest consistency, evidence freshness, and output path guard | Fixed in-slice |
| Review R2 | codex CLI adversarial reviewer | Bind commit, chart, and migration inventory to the checkout | Fixed in-slice |
| Review R3 | codex CLI adversarial reviewer | Verify from committed tree, deploy-only migration inventory, required manifest fields, safer output parent behavior | Fixed in-slice |
| Review R4 | codex CLI adversarial reviewer | Enforce v1alpha1 schema and fixed release notes template pointer in the committed tree | Fixed in-slice |
| Review R5 | codex CLI adversarial reviewer | Found symlink, live-checkout pointer, and non-portable migration checksum gaps | Staff+ arbitration required |
| Staff+ arbitration | Software Architect + Release Engineering Architect + Security Engineer + SRE/Operations Architect + Performance/Database Architect | Unanimous fix-in-slice decision; no out-of-scope deferral | Final arbitration fixes reject symlinks, validate pointer only in committed tree, and make migration checksum portable |

## §14. Merge Checklist

- [ ] Release bundle docs exist
- [ ] Build and check scripts pass
- [ ] Helm gates pass
- [x] AIT review clean or arbitration recorded
- [ ] Memory updated
