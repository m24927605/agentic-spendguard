# POST_GA 06 - Stats Drift Hygiene

> **Branch**: `post-ga/POST_GA_06_stats_drift_hygiene`
> **Status**: implemented; adversarial review pending
> **Spec ancestor(s)**: `post-ga-backlog-spec-v1alpha1.md`, `stats-aggregator-spec-v1alpha1.md`, `audit-chain-prediction-extension-v1alpha1.md`
> **Issues**: #157, #162
> **Estimated change size**: medium; stats aggregation, alert dedup, numeric guards

---

## §0. TL;DR

Add durable prediction drift alert dedup/cooldown and harden drift math
against NaN/Infinity so the stats aggregator emits stable, bounded
alerts.

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
- Add evidence under `docs/internal/reviews/post-ga/POST_GA_06_stats_drift_hygiene/`

## §5. Schema / Proto

Prefer an existing table or deterministic idempotency key if available.
Durable cooldown state lands in
`services/canonical_ingest/migrations/0022_prediction_drift_alert_cooldowns.sql`.
The primary key is exactly `(tenant_id, model, agent_id, prompt_class)`;
`suppress_until` is indexed for expiry inspection. The same table also
stores a nullable pending signed CloudEvent proto reservation so
commit-then-timeout retries reuse the exact same event id and bytes until
canonical_ingest returns `APPENDED` or `DEDUPED`. Key constraints mirror the
`canonical_events` aggregator mirror columns, including character-count
limits for multibyte-safe `agent_id` values and the same 7-class
`prompt_class` enum. RLS is enabled and forced with a `FOR ALL` policy using
`app.current_tenant_id`; missing or invalid tenant context fails closed.
`last_z_score` and `pending_z_score` reject `NaN` and `+/-Infinity` at both
runtime and SQL CHECK layers. No proto changes.

## §6. Audit-Chain Impact

Dedup suppresses duplicate alert emission; it does not delete or mutate
prior audit rows. Suppressed decisions are observable through structured
logs and `spendguard_stats_aggregator_drift_alerts_suppressed_total`, so
operators can distinguish quiet cooldown from no drift.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Same drift repeats within 24h | Suppress duplicate alert |
| Same drift after cooldown | Emit once |
| Different tenant/model/agent/prompt | Independent cooldown key |
| Drift math produces NaN/Infinity | Fail closed; no audit payload or cooldown row is written |
| Dedup store unavailable before emit | Fail safe; do not spam immutable audit |
| Immutable append times out after possible commit | Keep pending event reservation; retry same CloudEvent id/bytes |
| Immutable append fails before commit | No active cooldown is recorded; retry pending event next cycle |
| Cooldown record fails after append | Alert remains durable; log duplicate-suppression risk |

## §8. Acceptance Gates

- `cargo build && cargo test` for `services/stats_aggregator`
- Dedup/cooldown tests cover same-key and different-key behavior
- Numeric tests cover NaN, Infinity, and division-by-zero cases
- Migration smoke if SQL added
- `make demo-up DEMO_MODE=default` if aggregator runtime path changes
- Evidence under `docs/internal/reviews/post-ga/POST_GA_06_stats_drift_hygiene/`

Executed evidence is recorded in
`docs/internal/reviews/post-ga/POST_GA_06_stats_drift_hygiene/verification.md`.

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

## §12. Review Execution Notes

Reviewer: codex CLI via `codex review --base main`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer should inspect dedup persistence and numeric edge tests.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Software Architect | Keep stats hygiene separate from doc-only drift sync | POST_GA_06 vs POST_GA_02 |
| Backend Architect | Cooldown key must match issue text exactly | §7 |
| Security Engineer | Alert suppression must not hide tenant isolation issues | §9 |
| Database Optimizer | Durable dedup needs a bounded indexed key | §5 |
| Stats Domain Expert | NaN/Infinity are invalid alert payload values | §7 |
| Implementer self-review | PostgreSQL treats `NaN = NaN` as true, so the SQL CHECK must explicitly compare against `'NaN'::REAL` | Migration 0022 + `drift_alert_cooldown_postgres_rejects_non_finite_z_scores` |
| Staff+ panel | Store-unavailable behavior favors suppressing duplicates over immutable audit spam | §6/§7 |
| Staff+ arbitration | Round 5 Major is in-scope; pending signed CloudEvent reservation is required so append timeout retries reuse the same event id and canonical bytes | Migration 0022 + `reserve_emission` |

## §14. Merge Checklist

- [x] #157 fixed and tested
- [x] #162 fixed and tested
- [x] Stats gates pass
- [x] Codex review clean or Staff+ arbitration recorded
- [ ] Memory updated
