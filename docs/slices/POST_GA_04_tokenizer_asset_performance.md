# POST_GA 04 - Tokenizer Asset Performance

> **Branch**: `post-ga/POST_GA_04_tokenizer_asset_performance`
> **Status**: implementation complete; adversarial review clean in 2 rounds
> **Spec ancestor(s)**: `post-ga-backlog-spec-v1alpha1.md`, `tokenizer-service-spec-v1alpha1.md`
> **Issues**: #95, #102, #104, #108, #116, #120, #122, #125, #130, #134, #140
> **Estimated change size**: medium-large; assets, dispatch, benchmarks

---

## §0. TL;DR

Reduce tokenizer asset and dispatch overhead while improving benchmark
coverage and asset integrity checks.

## §1. Architectural Context

Tokenizer added vendored assets and multiple encoders to replace a
legacy heuristic. The current implementation is correct but carries
asset duplication, dispatch linear scan, repeated test startup work, and
limited large-input benchmark coverage.

## §2. Scope

- #95, #102: reduce sidecar/tokenizer dead-weight asset duplication
- #104: avoid bootstrapping expensive tokenizer state per test
- #108: make cross-check fixtures extensible for Anthropic/Gemini
- #116, #134: reduce duplicated encoder code and unused helpers
- #120: remove dead deps and misleading comments
- #122: boot observability and lazy-load consideration
- #125: asset checksum enumeration for SLICE_04 vendored files
- #130: dispatch performance improvement
- #140: Llama envelope tuning after #135

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| New provider network clients | POST_GA_05 |
| Runtime rate limiting/security | POST_GA_03 |
| License attribution | POST_GA_10 unless tied to asset packaging |

## §4. File-Level Changes

- Modify tokenizer encoder/cache/dispatch code under `services/tokenizer/src/**`
- Modify benchmark fixtures and benches under tokenizer test/bench paths
- Update asset checksum scripts
- Update docs describing asset layout and boot behavior
- Add evidence under `docs/reviews/post-ga/POST_GA_04_tokenizer_asset_performance/`

## §5. Schema / Proto

No schema or proto changes.

## §6. Audit-Chain Impact

No audit-chain schema impact. Tokenizer version IDs must remain stable
for any asset repackaging; version changes require explicit seed/mirror
updates.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Asset de-dup changes hash | Version ID and seed evidence updated deliberately |
| Dispatch optimization misroutes model | Golden routing tests fail |
| Lazy load causes first-request SLO regression | Benchmark captures cold and warm latency |
| Checksum script misses vendored file | Validator fails |

## §8. Acceptance Gates

- `cargo build && cargo test` for `services/tokenizer`
- Tokenizer benchmark before/after evidence with p50/p95/p99 and cold/warm labels
- Asset checksum script covers all vendored tokenizer files
- Dispatch routing tests for all supported providers
- `git diff --check`
- Evidence under `docs/reviews/post-ga/POST_GA_04_tokenizer_asset_performance/`

## §9. Review Checklist

1. Are asset hashes and tokenizer version IDs still coherent?
2. Does dispatch optimization preserve provider routing?
3. Does benchmark evidence include 10K-character and cold-start paths?
4. Did dependency cleanup remove only unused dependencies?
5. Is Llama envelope tuning based on non-tautological evidence?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Runtime security hardening | POST_GA_03 |
| Provider API expansion | POST_GA_05 |

## §11. Risk / Rollback

Risk is silent token count drift. Keep golden fixtures and version/hash
checks in the same commits as performance changes. Roll back any asset
packing change if hashes or fixture counts become ambiguous.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer should compare fixture and benchmark evidence, not only unit
test pass/fail.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Software Architect | Separate performance from runtime security | POST_GA_04 stays asset/dispatch focused |
| Backend Architect | Dispatch improvements need exhaustive routing tests | §8 |
| Security Engineer | Checksum enumeration is part of supply-chain integrity | #125 |
| Performance Engineer | Benchmarks must include cold/warm and p99 | §8 |
| Tokenizer Domain Expert | Llama tuning must wait for real envelope evidence | #140 dependency |
| Software Architect | Keep OpenAI dual-copy integrity design, remove only stale sidecar placeholder dependency | #95/#102 closed without weakening Layer A/B checks |
| Backend Architect | RegexSet dispatch is acceptable only with entry-order alignment test | `regex_set_and_entry_table_stay_aligned` |
| Security Engineer | Checksum evidence must enumerate all 7 vendored assets | `checksums.txt` |
| Performance Engineer | Criterion cold-start was misleading after round 1; use fresh process probe instead | Round 1 P2 fixed by `tokenizer_cold_start_once` + `cold-start-percentiles.tsv` |
| Tokenizer Domain Expert | Do not retune Llama without Tier 1 provider evidence; pin current Bedrock envelope instead | `per_message=5`, `BOS=1`, no version-id change |
| Reviewer | codex CLI fallback review after AIT parser incompatibility | Round 1 P2 fixed; round 2 clean |

## §14. Merge Checklist

- [x] Benchmarks and tests pass
- [x] Asset checksums complete
- [x] All mapped issues have evidence
- [x] AIT review clean or Staff+ arbitration recorded
- [ ] Memory updated
