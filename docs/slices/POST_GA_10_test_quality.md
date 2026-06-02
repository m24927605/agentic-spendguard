# POST_GA 10 - Test Quality

> **Branch**: `post-ga/POST_GA_10_test_quality`
> **Status**: clean adversarial review; merge pending
> **Spec ancestor(s)**: `post-ga-backlog-spec-v1alpha1.md`, `tokenizer-service-spec-v1alpha1.md`
> **Issues**: #109, #124
> **Estimated change size**: small-medium; test fixtures and compliance docs

---

## §0. TL;DR

Improve tokenizer cross-check fixture diversity and close the remaining
license/attribution documentation gap.

## §1. Architectural Context

Earlier tokenizer slices focused on correctness and production
readiness. Remaining test-quality work adds coverage for hard Unicode
cases and documents Llama 3.1 Community License attribution/MAU/AUP
constraints so future provider work is legally reviewable.

## §2. Scope

- #109: cross-check fixtures for 4-byte UTF-8, ZWJ, and RTL diversity
- #124: Llama 3.1 Community License attribution, 700M MAU clause, and acceptable-use clauses
- Fixture docs explaining why each new sample exists
- Tests proving tokenizer behavior across added fixture classes

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| Runtime tokenizer performance | POST_GA_04 |
| Llama provider client | POST_GA_05 |
| Broad legal review program | Future compliance workflow |

## §4. File-Level Changes

- Update tokenizer cross-check fixture files/tests
- Update license/attribution docs near tokenizer assets or provider docs
- Add evidence under `docs/reviews/post-ga/POST_GA_10_test_quality/`

## §5. Schema / Proto

No schema or proto changes.

## §6. Audit-Chain Impact

No audit-chain impact. Fixture additions must not change runtime audit
fields.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Unicode fixture exposes tokenizer drift | Test documents and asserts intended provider behavior |
| Fixture is tautological | Reviewer blocks |
| License wording omits MAU/AUP clause | Reviewer blocks |
| Docs imply legal advice | Reword as adoption/compliance checklist |

## §8. Acceptance Gates

- `cargo build && cargo test` for tokenizer tests touched
- Fixture tests cover 4-byte UTF-8, ZWJ, and RTL examples
- License docs mention attribution, 700M MAU threshold, and acceptable-use obligations
- `git diff --check`
- Evidence under `docs/reviews/post-ga/POST_GA_10_test_quality/`

## §9. Review Checklist

1. Are fixtures non-tautological and provider-relevant?
2. Do Unicode samples cover the issue text?
3. Does license text avoid pretending to be legal advice?
4. Are tests deterministic?
5. Is runtime behavior unchanged except for fixture coverage?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Full legal policy engine | Outside engineering slice |
| Provider implementation | POST_GA_05 |

## §11. Risk / Rollback

Risk is brittle fixtures or inaccurate license text. Keep fixture
comments precise and cite file paths or official license links in docs.
Rollback is normal docs/test revert.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer should reject tautological fixtures and unsupported compliance
claims.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Software Architect | Keep test-quality cleanup as final post-GA slice | Lower blast radius |
| Backend Architect | Fixture tests must exercise actual tokenizer code | §8 |
| Security Engineer | License/AUP docs are supply-chain risk controls | #124 |
| Tokenizer Domain Expert | Unicode fixture diversity closes known blind spots | #109 |
| Technical Writer | License docs must be operational guidance, not legal advice | §9 |

## §14. Merge Checklist

- [x] #109 fixed and tested
- [x] #124 fixed and reviewed
- [x] Test evidence recorded
- [x] AIT review clean or Staff+ arbitration recorded
- [ ] Memory updated
