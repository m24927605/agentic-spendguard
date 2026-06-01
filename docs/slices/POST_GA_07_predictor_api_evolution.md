# POST_GA 07 - Predictor API Evolution

> **Branch**: `post-ga/POST_GA_07_predictor_api_evolution`
> **Status**: draft
> **Spec ancestor(s)**: `post-ga-backlog-spec-v1alpha1.md`, `output-predictor-service-spec-v1alpha1.md`, `predictor-architecture-spec-v1alpha1.md`
> **Issues**: #161, #165
> **Estimated change size**: medium; output predictor API and rate limiting

---

## §0. TL;DR

Evolve output predictor API observability by exposing
`prediction_policy_used` and add per-tenant Predict RPC rate limits.

## §1. Architectural Context

Output predictor selects among Strategy A/B/C according to policy and
runtime availability. Audit columns already capture prediction policy
state, but API consumers need explicit response shape. Separately, the
Predict RPC needs tenant-scoped rate limiting to protect shared service
capacity.

## §2. Scope

- #161: add `prediction_policy_used` to PredictResponse or equivalent
  backward-compatible shape
- #165: per-tenant Predict RPC rate limit
- SDK/proto compatibility review if proto changes
- Tests for legacy clients, unknown policy, and per-tenant isolation

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| Strategy C resilience internals | POST_GA_09 |
| Predictor dashboard changes | Future observability if required |
| Contract DSL behavior changes | Not needed |

## §4. File-Level Changes

- Modify output predictor proto and generated code if field addition is chosen
- Modify `services/output_predictor/src/**`
- Update SDK bindings/tests if generated clients are affected
- Update docs for response semantics and rate-limit config
- Add evidence under `docs/reviews/post-ga/POST_GA_07_predictor_api_evolution/`

## §5. Schema / Proto

Proto changes must be additive only. Field numbers must be reviewed
against existing generated code. Rate-limit config should be additive
and default to current unrestricted behavior unless Staff+ chooses a
safe default cap.

## §6. Audit-Chain Impact

No audit schema change expected. API `prediction_policy_used` must agree
with existing audit columns for the same request.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Old client ignores new response field | Compatible |
| Tenant exceeds rate limit | Resource exhausted or documented fail-closed status |
| One tenant floods Predict | Other tenants unaffected |
| API field disagrees with audit | Test fails |
| Limit config missing | Documented default behavior |

## §8. Acceptance Gates

- `cargo build && cargo test` for `services/output_predictor`
- Proto generation and SDK tests if proto changes
- Rate-limit tests for per-tenant isolation
- Audit/API consistency test for `prediction_policy_used`
- Helm demo/production templates if config added
- Evidence under `docs/reviews/post-ga/POST_GA_07_predictor_api_evolution/`

## §9. Review Checklist

1. Is the API change backward compatible?
2. Does the response field match audit truth?
3. Is rate limiting per tenant?
4. Are status codes documented and tested?
5. Are SDK bindings regenerated when needed?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| New policy modes | Architecture is locked |
| UI display of policy used | Product UX |

## §11. Risk / Rollback

Proto/API changes carry compatibility risk. Prefer additive fields and
compatibility tests. Rate limiting can cause unexpected throttling; keep
config explicit and observable. Roll back by disabling limit config
while preserving additive proto compatibility.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer should inspect proto compatibility and tenant isolation.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Software Architect | Keep API evolution separate from Strategy C resilience | POST_GA_07 vs POST_GA_09 |
| Backend Architect | Proto changes must be additive only | §5 |
| Security Engineer | Rate limits are tenant isolation controls | §7 |
| Database Optimizer | No DB change unless audit/API consistency requires query support | §6 |
| Output Predictor Domain Expert | API truth must match audit truth | §8 |

## §14. Merge Checklist

- [ ] #161 fixed and tested
- [ ] #165 fixed and tested
- [ ] Compatibility evidence recorded
- [ ] AIT review clean or Staff+ arbitration recorded
- [ ] Memory updated
