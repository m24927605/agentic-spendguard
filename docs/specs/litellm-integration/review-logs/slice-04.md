# Slice 4 — Streaming reconciler · Codex adversarial review log

Slice scope: Implement `_async_log_success_streaming` in
`sdk/python/src/spendguard/integrations/litellm.py` — end-of-stream
reconciliation that derives commit amount from `response_obj.usage`
when present, falls back to the pre-call estimator snapshot when
absent (degraded path), narrows `SidecarUnavailable` to the commit
boundary only, and keeps semantic `SpendGuardError` propagating
as-is.

## Round summary

| Round | Verdict | Headline finding |
| --- | --- | --- |
| R1 | FAIL | P1.1 over-broad `SpendGuardError` wrap masks semantic errors; P1.2 stash uses mutable claim ref; P2.1 reconciler test didn't actually use `.usage`. |
| R2 | FAIL | P1 snapshot built AFTER `await self._client.request_decision(...)` — mutation window during await could change committed amount. |
| R3 | **PASS** (stopping rule met) | None — snapshot is now pre-await; no new P0/critical-P1 in diff. |

## R1 fixes (committed `ee61f1d`)

- P1.1 Narrow `except SpendGuardError` at commit boundary →
  `except SidecarUnavailable` only; semantic `SpendGuardError`
  re-raises unwrapped so callers can distinguish transport
  failures from invariant violations.
- P1.2 Stash key renamed `estimator_claims` →
  `estimator_claims_snapshot` (carries primitive `SimpleNamespace`,
  not the operator's mutable claim object).
- P2.1 Streaming happy-path test now derives commit amount from
  `resp.usage.completion_tokens * 2` so the assertion actually
  exercises the reconciler-from-usage pathway.

## R2 fix (committed `56bf2f9`)

- P1 Moved `_estimator_snapshot = SimpleNamespace(...)` from
  AFTER `await self._client.request_decision(...)` to immediately
  after estimator cardinality + binding validation, BEFORE the
  sidecar await. This closes the mutation window where a concurrent
  task touching the operator's shared mutable claim object during
  the await could change what the streaming fallback later commits.
- Regression test `test_streaming_snapshot_captures_pre_await_value`
  added: `request_decision` mock mutates the claim DURING the
  await; assertion confirms the streaming fallback commits the
  pre-await value `"500"`, not the post-mutation `"999999999"`.

## R3 outcome — Stopping rule met (§3.4)

- (A) Prior P0 — none open from Slice 4.
- (A') Critical-P1 — R2 P1 fixed by `56bf2f9`; R3 confirmed
  snapshot now built pre-await.
- (B) Zero new P0/critical-P1 in the R3 diff review.
- (C) N≥2 — this is R3.

Codex R3 quote: *"Snapshot construction is now moved before
`await self._client.request_decision(...)`, immediately after
estimator validation. VERDICT: PASS"*

## Tests

- `tests/test_litellm_streaming_unit.py` — 12 tests pass.
- Full suite: 90 tests pass, ruff clean (litellm-scoped), mypy strict
  clean.

## Slice 4 → CLOSED. Next: Slice 5 — Failure release + retry handling.
