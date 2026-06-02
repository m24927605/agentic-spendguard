# POST_GA_09 Direct Codex Adversarial Review - Round 1

AIT attempt: `a338` (`ait run --adapter codex --review adversarial --review-adapter codex`) returned `attempt is not reviewable`, so direct codex CLI fallback was used per local review workflow.

## Findings

### Blocker - Endpoint cache singleflight did not share failed reload results

`services/output_predictor/src/endpoint_cache.rs` serialized same-tenant reloads, but only a successful reload wrote a fresh cache entry. True misses removed/kept no entry, so queued callers re-entered `load_one` and hit the DB one by one. DB-error stale serving returned stale without updating any shared state, so queued callers also retried the DB sequentially. This made the #174 singleflight claim untrue for misses and DB outage stale paths.

Resolution: add a short reload-result backoff. True misses record a 1s not-configured backoff, and DB-error stale serves record a 1s stale reload backoff without changing `loaded_at`. Queued callers reuse the same result while unrelated tenants remain independent.

### Major - Force-reset audit effect overclaimed predictor breaker reset

`force_reset` audit payload said `output_predictor observes on next endpoint cache reload`, but the output predictor endpoint cache only reads endpoint identity plus `enabled`; the in-memory circuit breaker is driven by health checks and runtime observations. The audit event was still ambiguous about whether the operation reset a control-plane status row or actual predictor pod breaker state.

Resolution: rename the audit operation to `force_reset_plugin_health_status`, change the response note/log message, and record an explicit status-only effect stating that output_predictor in-memory breakers are not directly mutated.

## Outcome

Round 1 findings require code and documentation changes before Round 2.
