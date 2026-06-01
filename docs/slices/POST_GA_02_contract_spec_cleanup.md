# POST_GA 02 - Contract and Spec Cleanup

> **Branch**: `post-ga/POST_GA_02_contract_spec_cleanup`
> **Status**: review round 1 findings fixed; awaiting round 2 adversarial review
> **Spec ancestor(s)**: `post-ga-backlog-spec-v1alpha1.md`, `contract-dsl-spec-v1alpha2.md`, `tokenizer-service-spec-v1alpha1.md`, `stats-aggregator-spec-v1alpha1.md`
> **Issues**: #91, #93, #97, #99, #101, #113, #121, #123, #131, #136, #141, #147, #154, #158, #159, #167, #177
> **Estimated change size**: medium; docs/spec cleanup with grep validation

---

## §0. TL;DR

Reconcile documentation drift left by SLICE_02 through SLICE_07 without
changing runtime behavior. This slice makes shipped docs honest,
searchable, and traceable to the implementation.

## §1. Architectural Context

The GA code path is shipped. Several open issues are doc/title/comment
drift: wedge-boundary wording, tokenizer marker semantics, actual hard
caps, fixture counts, source URI formats, event type names, and stale
slice checklist wording. These are not runtime blockers, but stale docs
create future implementation mistakes.

## §2. Scope

- #91, #93: Contract DSL upgrade-path and wedge-boundary wording
- #97, #99, #101, #113, #121, #123, #131, #136: tokenizer and fixture doc drift
- #141: provider API key rotation runbook
- #147, #154: slice doc/index/checklist drift
- #158, #159: drift alert source URI and event type spec sync
- #167: sample_size CHECK constraint spec amendment
- #177: NotServing variant cleanup in docs/specs

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| Runtime tokenizer fixes | POST_GA_03 or POST_GA_04 |
| Stats drift runtime changes | POST_GA_06 |
| Strategy C runtime changes | POST_GA_09 |
| Provider client implementation | POST_GA_05 |

## §4. File-Level Changes

- Update affected sections in `docs/contract-dsl-spec-v1alpha2.md`
- Update affected sections in `docs/tokenizer-service-spec-v1alpha1.md`
- Update affected sections in `docs/stats-aggregator-spec-v1alpha1.md`
- Update slice docs with stale checklist or title drift
- Add `docs/operations/runbooks/provider-key-rotation.md` if no current runbook exists
- Add evidence under `docs/reviews/post-ga/POST_GA_02_contract_spec_cleanup/`

## §5. Schema / Proto

No schema or proto changes. This slice may edit comments in proto files
only when they are demonstrably stale and generated code is unaffected.

## §6. Audit-Chain Impact

No audit-chain data changes. The slice must ensure spec text matches the
actual audit event types and source URI formats emitted by code.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Spec says one event type and code emits another | Spec corrected or implementation issue opened in the owning runtime slice |
| Doc title implies wrong boundary | Title corrected |
| Comment contradicts hard cap | Comment corrected to current runtime behavior |
| Runbook would leak provider keys | Redact and use Secret references only |

## §8. Acceptance Gates

- `scripts/ga/validate-post-ga-docs.sh`
- `git diff --check`
- Grep audit proves documented drift alert event types match emission code
- Link/anchor grep for changed docs
- No runtime build required unless proto comments trigger regeneration

## §9. Review Checklist

1. Are all mapped issues addressed with exact doc locations?
2. Did any wording imply behavior the code does not implement?
3. Are all code references real paths or real commands?
4. Are secrets redacted in provider key rotation docs?
5. Are runtime changes avoided unless explicitly justified?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Runtime drift dedup | POST_GA_06 |
| Tokenizer runtime behavior | POST_GA_03 / POST_GA_04 |
| Provider implementation | POST_GA_05 |

## §11. Risk / Rollback

Risk is documenting the wrong behavior as fact. Rollback is normal doc
revert. Any discovered runtime mismatch belongs to the owning runtime
slice before issue closure.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer should treat fabricated citations and stale event names as
findings.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Software Architect | Use one docs cleanup slice to avoid many tiny doc branches | §2 grouping |
| Backend Architect | Runtime behavior must be sourced from code, not memory | §8 grep gates |
| Security Engineer | Provider key rotation runbook must not include plaintext secrets | §7 |
| Database Optimizer | sample_size constraint docs must match real migrations | #167 in scope |
| Technical Writer | Titles must describe shipped boundaries, not planned behavior | #91 and #136 |
| Implementer | Reconciled all mapped doc/comment drift and added sample-size CHECK smoke coverage | Commits `d972a6e`, `38829a0`, `c63b871` |
| Reviewer | Round 1 found unsafe deployable RUN_* example, unsafe CJK fail-closed wording, and wrong UUIDv7 prefix date | Fixed in `POST_GA_02 fix review round 1 findings`; see `docs/reviews/post-ga/POST_GA_02_contract_spec_cleanup/round-1.md` |

## §14. Merge Checklist

- [x] All 17 issues have doc evidence
- [x] Grep checks pass
- [x] No runtime drift introduced
- [ ] AIT review clean or Staff+ arbitration recorded
- [ ] Memory updated
