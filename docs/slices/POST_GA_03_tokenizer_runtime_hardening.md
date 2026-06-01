# POST_GA 03 - Tokenizer Runtime Hardening

> **Branch**: `post-ga/POST_GA_03_tokenizer_runtime_hardening`
> **Status**: draft
> **Spec ancestor(s)**: `post-ga-backlog-spec-v1alpha1.md`, `tokenizer-service-spec-v1alpha1.md`
> **Issues**: #92, #94, #96, #98, #100, #103, #105, #110, #111, #112, #114, #115, #117, #118, #119, #126, #127, #129, #133, #135, #148, #149, #151, #152, #156
> **Estimated change size**: large; tokenizer runtime, security, tests, Helm

---

## §0. TL;DR

Harden the tokenizer service around readiness, request identity,
resource limits, production UDS setup, provider parity, drift evidence,
and integration tests.

## §1. Architectural Context

Tokenizer is the hot-path replacement for the old heuristic. It already
meets GA readiness, but post-GA issues identify runtime polish: readyz
before bind, request ID validation, per-tenant rate limiting, content
size scaling, timeout enforcement, NetworkPolicy metrics exposure,
drift sample semantics, and test coverage.

## §2. Scope

- readiness/bind correctness: #96
- schema bundle and CloudEvent helper parity: #92, #94, #152
- naming/code quality: #98, #115
- SLO/spec amendments backed by runtime guards: #100, #114, #127
- production UDS and NetworkPolicy docs/templates: #103, #105
- per-tenant rate limiting and UUIDv7 request IDs: #110, #111
- envelope and drift guardrails: #112, #135, #148, #149
- canonical ingest client serialization concern: #156
- tests and fixtures: #117, #118, #119, #126, #129, #133, #151

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| Asset size reduction | POST_GA_04 |
| Cohere/Llama Tier 1 provider clients | POST_GA_05 |
| Generic cross-check fixture diversity | POST_GA_10 |

## §4. File-Level Changes

- Modify `services/tokenizer/src/**`
- Modify tokenizer tests under `services/tokenizer/tests/**`
- Modify tokenizer Helm chart templates and production values if needed
- Update `docs/tokenizer-service-spec-v1alpha1.md`
- Update tokenizer runbooks under `docs/operations/runbooks/**`
- Add evidence under `docs/reviews/post-ga/POST_GA_03_tokenizer_runtime_hardening/`

## §5. Schema / Proto

No breaking proto changes are expected. Request ID validation may reject
bad `request_id` values at runtime but does not require a field change.
If partition/default changes are required for sampled event time, add
forward-only migrations.

## §6. Audit-Chain Impact

Tokenizer tier and version evidence must remain populated on audit rows.
Drift alerts must keep immutable audit routing. Event-time sampling fixes
must not mutate historical rows.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| `/readyz` called before gRPC bind | Not ready |
| Invalid request_id | Reject or mint safe UUIDv7 per documented rule |
| Tenant exceeds tokenizer rate limit | Fail closed or degrade per spec without cross-tenant effect |
| Oversized multi-turn request | Enforced cap and clear error |
| BPE path stalls | Per-request timeout |
| Drift sample event time absent | Persist-time fallback is explicit and tested |
| Canonical ingest client serializes all events through one mutex | Throughput impact is measured and either fixed or accepted by Staff+ with evidence |

## §8. Acceptance Gates

- `cargo build && cargo test` for `services/tokenizer`
- Targeted tests for readiness, request_id, rate limit, BPE timeout, max message size, and drift sample semantics
- Throughput or concurrency evidence for canonical ingest client serialization behavior
- Helm demo and production templates render cleanly
- NetworkPolicy render/grep proves metrics exposure is intended
- `make demo-up DEMO_MODE=default`
- Evidence under `docs/reviews/post-ga/POST_GA_03_tokenizer_runtime_hardening/`

## §9. Review Checklist

1. Can readiness report ready before gRPC is bound?
2. Is rate limiting per tenant, not global only?
3. Does UUID validation prevent log correlation poisoning?
4. Are timeouts around expensive tokenizer paths real?
5. Are NetworkPolicy and UDS requirements deployable?
6. Are drift-alert tests non-tautological?
7. Is any CanonicalIngestClient serialization bottleneck measured or removed?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Asset duplication cleanup | POST_GA_04 owns it |
| New provider clients | POST_GA_05 owns it |

## §11. Risk / Rollback

This slice touches hot-path tokenization. Prefer feature flags or
configuration defaults that preserve current behavior until tests prove
the change. Roll back by reverting runtime commits and keeping docs
aligned with shipped behavior.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer should run security, readiness, and hot-path latency oriented
grep/tests.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Software Architect | Keep runtime tokenizer hardening separate from asset optimization | POST_GA_03 vs POST_GA_04 |
| Backend Architect | Readiness must be tied to actual bind state | #96 acceptance |
| Security Engineer | Request IDs and metrics exposure are security-relevant | #103 and #111 in scope |
| Database Optimizer | Event-time sampling fixes require migration review | #148 and #149 |
| Tokenizer Domain Expert | Provider parity tests must avoid tautological fixtures | #117, #119, #133 |

## §14. Merge Checklist

- [ ] Runtime tokenizer tests pass
- [ ] Helm/demo gates pass
- [ ] All mapped issues have closure evidence
- [ ] AIT review clean or Staff+ arbitration recorded
- [ ] Memory updated
