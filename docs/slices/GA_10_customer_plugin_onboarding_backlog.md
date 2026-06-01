# GA 10 - Customer Plugin Onboarding and Backlog Triage

> **Branch**: `ga/GA_10_customer_plugin_onboarding_backlog`
> **Status**: implementation
> **Spec ancestor(s)**: `ga-readiness-spec-v1alpha1.md`
> **Estimated change size**: medium; docs, conformance checks, issue triage

---

## §0. TL;DR

Make customer plugin onboarding concrete and triage remaining non-P1 issues into GA-before, GA-after, and roadmap buckets with evidence and labels.

## §1. Architectural Context

The customer plugin template and SVID enforcement now work, but GA customers need a certification checklist, error taxonomy, onboarding flow, and clean backlog posture.

## §2. Scope

- Plugin certification checklist
- Tenant SVID onboarding guide
- Error taxonomy for plugin/operator failures
- Conformance command docs
- Non-P1 issue triage report
- GitHub issue labeling/commenting as needed
- GA-before closure plan for customer-critical non-P1s

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| Customer UI for onboarding | Future product UX |
| Closing every P2/P3 issue | Only GA-before issues block this phase |

## §4. File-Level Changes

- Add `docs/customer/plugin-onboarding.md`
- Add `docs/customer/plugin-certification-checklist.md`
- Add `docs/customer/plugin-error-taxonomy.md`
- Update `contrib/output_predictor_template/README.md`
- Add `docs/reviews/ga-readiness/GA_10_customer_plugin_onboarding_backlog/backlog-triage.md`
- Add evidence under `docs/reviews/ga-readiness/GA_10_customer_plugin_onboarding_backlog/`

## §5. Schema / Config / API Impact

No API changes expected. If the error taxonomy exposes missing machine-readable errors, fix them in-slice or record Staff+ arbitration.

## §6. Audit / Security / Operational Impact

Customer plugin onboarding must not weaken SVID validation, mTLS, tenant isolation, or audit-chain requirements.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Plugin lacks SVID validation | Certification fails |
| Conformance test not run | Slice cannot merge |
| Open issue is misclassified | Reviewer blocks until triage is corrected |
| Error taxonomy hides fail-closed behavior | Review blocks |

## §8. Acceptance Gates

- `python3 -m pytest contrib/output_predictor_template/conformance_test.py -q`
- Plugin onboarding docs include SVID, mTLS, timeout, retry, circuit-breaker, and audit expectations
- `gh issue list --repo m24927605/agentic-spendguard --limit 120 --state open` triaged into GA-before/GA-after/roadmap
- GA-before issues are closed or moved to a named implementation slice before phase completion
- Duplicate candidates #155 and #170 are closed only with commit/test evidence

## §9. Review Checklist

1. Can a customer run certification without private knowledge?
2. Does onboarding enforce tenant SVID exactly?
3. Are plugin failures mapped to operator/customer actions?
4. Are remaining issues honestly classified?
5. Are GA-before issues not hidden as roadmap?
6. Are issues #85-#177 all represented in the triage report unless already closed?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Full customer portal | Product UX work |
| Closing all P3 enhancements | Roadmap, not GA blocker |

## §11. Risk / Rollback

Docs and triage comments can be reverted or corrected. Any runtime fix follows normal slice rollback.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must inspect triage honesty and reject undocumented customer-critical failure modes.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Customer Plugin/Backend Architect | Plugin certification and backlog triage share one customer-readiness slice | GA_10 owns both |
| Security Engineer | SVID/mTLS requirements remain non-negotiable | Certification checklist enforces them |
| SRE/Operations Architect | Plugin failure modes must map to customer and operator actions | Error taxonomy maps Strategy C metric labels to actions |
| Database Optimizer | DB/index polish issues can be non-GA only when named and visible | Backlog triage assigns them to post-GA implementation slices |
| Software Architect | Duplicate and historical issues must close with evidence | #155 and #170 closure evidence recorded in backlog triage |
| Review R1 | codex CLI adversarial reviewer | Fixed live API path and `client_cert_id` registration sample, client SVID evidence path, SVID `PERMISSION_DENIED` to `tls_error` routing, and backlog slice/table consistency |
| Review R2 | codex CLI adversarial reviewer | Fixed reference-image `--insecure` override guidance and replaced invalid multi-issue `gh issue close` evidence with a reproducible loop |

## §14. Merge Checklist

- [x] Plugin onboarding docs exist
- [x] Conformance tests pass
- [x] Backlog triage report exists
- [ ] AIT review clean or arbitration recorded
- [ ] Memory updated
