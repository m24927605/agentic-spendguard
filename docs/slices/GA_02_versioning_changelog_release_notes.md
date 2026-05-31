# GA 02 - Versioning, Changelog, and Release Notes

> **Branch**: `ga/GA_02_versioning_changelog_release_notes`
> **Status**: design
> **Spec ancestor(s)**: `ga-readiness-spec-v1alpha1.md`
> **Estimated change size**: small; docs and release note validator

---

## §0. TL;DR

Define the GA versioning policy, changelog format, and release notes template so releases are operator-readable and taggable without ambiguity.

## §1. Architectural Context

The repo has SDK changelog material but not a product-level GA release standard. Operators need migration, Helm, security, and rollback notes in one consistent format.

## §2. Scope

- Product changelog
- Versioning policy
- Release notes template
- Release notes validation script
- Dry-run tag instructions

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| Publishing a GitHub Release | Future human release step |
| Signing release artifacts | GA_09 |

## §4. File-Level Changes

- Add or update `CHANGELOG.md`
- Add `docs/release/versioning-policy.md`
- Add `docs/release/release-notes-template.md`
- Add `scripts/release/prepare-release-notes.sh`
- Add evidence under `docs/reviews/ga-readiness/GA_02_versioning_changelog_release_notes/`

## §5. Schema / Config / API Impact

No runtime schema or API changes.

## §6. Audit / Security / Operational Impact

Release notes must surface security-relevant changes, migration risk, and operator actions.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Release notes omit migrations | Validator exits non-zero |
| Release notes omit rollback | Validator exits non-zero |
| Version format invalid | Validator exits non-zero |
| Tag already exists | Dry-run command detects it |

## §8. Acceptance Gates

- `scripts/release/prepare-release-notes.sh --check-template docs/release/release-notes-template.md`
- `scripts/release/prepare-release-notes.sh --check docs/reviews/ga-readiness/GA_02_versioning_changelog_release_notes/sample-release-notes.md`
- Product changelog includes predictor upgrade and HARDEN summary
- Version policy forbids ambiguous "latest" wording
- GA_01 bundle points to release notes format

## §9. Review Checklist

1. Are migration and rollback notes mandatory?
2. Does versioning avoid mutable tags?
3. Are security notes mandatory?
4. Are customer-visible behavior changes listed?
5. Is the dry-run tag command non-destructive?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Actual external release publication | Requires maintainer timing |

## §11. Risk / Rollback

Revert docs/scripts if the policy needs revision. No runtime impact.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must check that release notes cannot pass while omitting migrations, Helm changes, security notes, or rollback.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Release Engineering Architect | Release notes are their own slice | Prevents release bundle scope creep |
| Software Architect | Tags are documented but not pushed automatically | Avoids accidental public release |

## §14. Merge Checklist

- [ ] Changelog updated
- [ ] Versioning policy exists
- [ ] Release notes template validates
- [ ] AIT review clean or arbitration recorded
- [ ] Memory updated
