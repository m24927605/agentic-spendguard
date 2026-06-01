# POST_GA 06 - Stats Drift Hygiene

> **Branch**: `post-ga/POST_GA_06_stats_drift_hygiene`
> **Status**: draft
> **Spec ancestor(s)**: `post-ga-backlog-spec-v1alpha1.md`, `stats-aggregator-spec-v1alpha1.md`, `audit-chain-prediction-extension-v1alpha1.md`
> **Issues**: #157, #162
> **Estimated change size**: medium; stats aggregation, alert dedup, numeric guards

---

## §0. TL;DR

Add prediction drift alert dedup/cooldown and harden drift math against
NaN/Infinity so the stats aggregator emits stable, bounded alerts.

## §1. Architectural Context

Stats aggregation converts audit prediction evidence into drift alerts.
The current GA path works, but post-GA issues call out alert dedup over
24 hours and numeric safety around invalid floating-point values.

## §2. Scope

- #157: 24h cooldown/dedup per `(tenant, model, agent_id, prompt_class)`
- #162: NaN/Infinity guard in drift alert math
- Tests for dedup, cooldown expiry, tenant isolation, and numeric guards
- Spec sync for alert source/event behavior only when runtime changes require it

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| Drift event type doc cleanup | POST_GA_02 unless runtime changed |
| Dashboard/alert rule changes | Future observability slice unless required |
| Backfill of historical alerts | Not needed for forward hygiene |

## §4. File-Level Changes

- Modify `services/stats_aggregator/src/**`
- Add tests under stats aggregator test layout
- Add SQL migration only if dedup state needs durable storage
- Update `docs/stats-aggregator-spec-v1alpha1.md` if behavior changes
- Add evidence under `docs/reviews/post-ga/POST_GA_06_stats_drift_hygiene/`

## §5. Schema / Proto

Prefer an existing table or deterministic idempotency key if available.
If durable cooldown state is required, add a forward migration with a
unique key on the dedup dimensions and indexed expiry/last_emitted time.
No proto changes expected.

## §6. Audit-Chain Impact

Dedup suppresses duplicate alert emission; it must not delete or mutate
prior audit rows. Suppressed decisions should be observable through
metrics or logs so operators can distinguish quiet cooldown from no
drift.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Same drift repeats within 24h | Suppress duplicate alert |
| Same drift after cooldown | Emit once |
| Different tenant/model/agent/prompt | Independent cooldown key |
| Drift math produces NaN/Infinity | Skip or clamp according to documented fail-closed rule |
| Dedup store unavailable | Fail safe; do not spam immutable audit |

## §8. Acceptance Gates

- `cargo build && cargo test` for `services/stats_aggregator`
- Dedup/cooldown tests cover same-key and different-key behavior
- Numeric tests cover NaN, Infinity, and division-by-zero cases
- Migration smoke if SQL added
- `make demo-up DEMO_MODE=default` if aggregator runtime path changes
- Evidence under `docs/reviews/post-ga/POST_GA_06_stats_drift_hygiene/`

## §9. Review Checklist

1. Is the dedup key exactly `(tenant, model, agent_id, prompt_class)`?
2. Is the cooldown 24h and testable without sleeping?
3. Are NaN and Infinity impossible to emit into audit payloads?
4. Is tenant isolation preserved?
5. Does failure mode avoid alert spam?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Historical backfill | Not needed for forward alert hygiene |
| UI surfacing of suppressed alerts | Product UX |

## §11. Risk / Rollback

Risk is suppressing real alerts or spamming audit. Keep dedup key and
cooldown explicit, with metrics for suppression. Roll back by disabling
dedup only if alert spam risk is controlled by another gate.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer should inspect dedup persistence and numeric edge tests.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Software Architect | Keep stats hygiene separate from doc-only drift sync | POST_GA_06 vs POST_GA_02 |
| Backend Architect | Cooldown key must match issue text exactly | §7 |
| Security Engineer | Alert suppression must not hide tenant isolation issues | §9 |
| Database Optimizer | Durable dedup needs a bounded indexed key | §5 |
| Stats Domain Expert | NaN/Infinity are invalid alert payload values | §7 |

## §14. Merge Checklist

- [ ] #157 fixed and tested
- [ ] #162 fixed and tested
- [ ] Stats gates pass
- [ ] AIT review clean or Staff+ arbitration recorded
- [ ] Memory updated
