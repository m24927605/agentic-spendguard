# POST_GA 09 - Strategy C Resilience

> **Branch**: `post-ga/POST_GA_09_strategy_c_resilience`
> **Status**: implementation-complete; pending adversarial review
> **Spec ancestor(s)**: `post-ga-backlog-spec-v1alpha1.md`, `output-predictor-plugin-contract-v1alpha1.md`, `output-predictor-service-spec-v1alpha1.md`
> **Issues**: #172, #173, #174, #175, #176
> **Estimated change size**: medium-large; Strategy C resilience and audit clarity

---

## §0. TL;DR

Harden customer Strategy C plugin behavior around force-reset audit
clarity, reason-string caps, cache miss herd control, stale-cache
serving, and input length caps.

## §1. Architectural Context

Strategy C is optional and must never block enforcement. GA shipped mTLS,
per-tenant SVID, circuit breaker, and onboarding. Remaining issues are
resilience and abuse-hardening improvements that make plugin integration
safer under outages and malicious or oversized inputs.

## §2. Scope

- #172: force_reset audit ambiguity
- #173: reason-string length cap
- #174: thundering herd singleflight on endpoint cache miss
- #175: serve-stale-on-DB-error for endpoint cache
- #176: `decision_id` and fingerprint length caps

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| API response shape/rate limit | POST_GA_07 |
| Customer onboarding docs | Already GA_10 |
| New plugin protocol version | Future v1beta1 if additive changes are insufficient |

## §4. File-Level Changes

- Modify `services/output_predictor/src/strategy_c.rs`
- Modify `services/output_predictor/src/endpoint_cache.rs`
- Modify control-plane force-reset audit code if needed
- Add tests under `services/output_predictor/tests/**`
- Update plugin contract/service docs
- Add evidence under `docs/reviews/post-ga/POST_GA_09_strategy_c_resilience/`

## §5. Schema / Proto

No proto changes expected. Audit payload enrichment for force reset must
use existing signed audit/event payload patterns. Cache stale serving
should not require schema unless stale metadata is not currently
available.

## §6. Audit-Chain Impact

Force-reset audit events must distinguish operator intent and target
tenant/plugin state. Reason strings and input fields must be bounded
before they enter metrics, logs, or audit payloads.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Force reset happens | Audit payload clearly records target and reason |
| Reason string too long | Truncate/reject per documented cap |
| Many requests miss endpoint cache | Singleflight collapses DB load |
| DB temporarily unavailable | Use bounded stale cache or fall to B |
| Oversized decision_id/fingerprint | Reject or cap before hot-path work |

## §8. Acceptance Gates

- `cargo build && cargo test` for `services/output_predictor`
- Tests for cache singleflight, stale fallback, force-reset audit payload, reason cap, input caps
- `make demo-up DEMO_MODE=plugin_c_synthetic`
- Helm demo/production render if config added
- Evidence under `docs/reviews/post-ga/POST_GA_09_strategy_c_resilience/`

## §9. Review Checklist

1. Does stale cache preserve tenant isolation?
2. Can singleflight deadlock or serialize unrelated tenants?
3. Is force-reset audit unambiguous and signed?
4. Are length caps enforced before logging/audit?
5. Does every failure still fall back to Strategy B when appropriate?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Plugin protocol v1beta1 | Not required for issues #172-#176 |
| Customer UI reset flow | Product UX |

## §11. Risk / Rollback

Stale cache can route to an old endpoint if bounded incorrectly.
Singleflight can create contention. Keep tenant-scoped locks and TTLs
short and test DB-error behavior. Roll back by disabling stale serving
and singleflight if correctness concerns appear.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer should focus on tenant isolation, audit payload clarity, and
hot-path failure isolation.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Software Architect | Keep Strategy C resilience separate from API evolution | POST_GA_09 vs POST_GA_07 |
| Backend Architect | Singleflight must be tenant-scoped | §9 |
| Security Engineer | Reason/input caps are security controls | #173 and #176 |
| Database Optimizer | Stale cache must reduce DB outage blast radius | #175 |
| Customer Plugin Domain Expert | Plugin failure still falls to Strategy B | §7 |
| Implementer | Added tenant-scoped endpoint-cache singleflight, bounded stale-on-DB-error serving, force-reset audit target/transition payload, reason cap, and plugin-bound identifier caps | `76da6d2`, `e633ce0`, `62096cf`, `b61797e` |
| Reviewer R1 | Found endpoint-cache singleflight did not share true-miss or DB-error stale results; found force-reset audit effect overclaimed predictor breaker reset semantics | Codex direct adversarial fallback after AIT attempt `a338` was not reviewable |
| Implementer R1 | Added 1s reload-result backoff for true misses and DB-error stale serves; clarified force-reset audit/response as control-plane health-status-only | Pending commit |

## §14. Merge Checklist

- [x] #172-#176 fixed and tested
- [x] plugin_c_synthetic demo passes
- [x] Audit evidence recorded
- [ ] AIT review clean or Staff+ arbitration recorded
- [ ] Memory updated
