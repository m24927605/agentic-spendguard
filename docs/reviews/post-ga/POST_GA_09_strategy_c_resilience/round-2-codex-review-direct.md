# POST_GA_09 Direct Codex Adversarial Review - Round 2

AIT attempt: `a339` (`ait run --adapter codex --review adversarial --review-adapter codex`) returned `attempt is not reviewable`, so direct codex CLI fallback was used.

## Findings

### Major - True-miss backoff table was unbounded

`services/output_predictor/src/endpoint_cache.rs` introduced `not_configured_backoffs` for R1 miss sharing, but the map was unbounded. Expired one-off tenant misses were only removed when the same tenant was looked up again or explicitly evicted. Many valid UUID true misses could accumulate indefinitely, turning the miss-sharing fix into a memory DoS risk.

Resolution: cap the true-miss backoff table at 4096 tenants, sweep expired entries on each insert, evict one arbitrary active entry when full, and add unit coverage for the bounded/sweep behavior.

## Outcome

Round 2 findings require code and documentation changes before Round 3.
