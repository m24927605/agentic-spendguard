# COV_S05_09 — D05 TS SDK substrate: cross-language fixture harness

> **Deliverable**: D05 TS SDK substrate
> **Slice**: 9 of 10 (M)
> **Spec set**: [`docs/specs/coverage/D05_ts_sdk_substrate/`](../specs/coverage/D05_ts_sdk_substrate/)

## Scope

Land the cross-language fixture corpus + harness that proves Python ↔ TS byte-equivalence for the audit-chain determinism invariants (§13). SLICES 6+7 shipped scattered fixtures inside individual test files (promptHash FX1-FX7, ids FX1-FX7, deriveUuidFromSignature FXU1-FXU5). SLICE 9 lands a single canonical fixture JSON + harness that both languages consume.

Concretely:
- `sdk/fixtures/cross-language/v1.json` — NEW canonical fixture file:
  - Schema version 1
  - ≥20 fixtures covering: `derive_idempotency_key` (8 vectors with named-arg variations), `compute_prompt_hash` (8 vectors with edge cases — empty, long, Unicode, BOM), `derive_uuid_from_signature` (4 vectors with different scopes)
  - Each fixture: { id, fn, inputs: {...}, expected_output: "..." }
  - Inputs use the LOCKED named-arg shape per design.md §11.2 (no positional)
- `sdk/typescript/tests/crossLanguage.test.ts` — NEW:
  - Loads `sdk/fixtures/cross-language/v1.json`
  - Iterates every fixture, dispatches to the matching TS function, asserts byte-equivalence
  - Test naming format: `cross-language fixture ${id}: ${fn}` so failures point at the exact mismatched vector
- `sdk/python/tests/test_cross_language_fixtures.py` — NEW:
  - Same fixture file, Python side
  - Pytest parametrize per fixture so failures are isolated
- `sdk/fixtures/cross-language/README.md` — provenance + invariants doc:
  - "Adding a fixture: generate Python output first, paste hex, add to v1.json, both test suites pin it"
  - "Updating output for a renamed fn: NEW fixture file v2.json, never rewrite v1.json (audit-chain immutability)"

## Files touched

| File | Why |
|------|-----|
| `sdk/fixtures/cross-language/v1.json` | NEW canonical corpus |
| `sdk/fixtures/cross-language/README.md` | provenance |
| `sdk/typescript/tests/crossLanguage.test.ts` | NEW TS harness |
| `sdk/python/tests/test_cross_language_fixtures.py` | NEW Python harness |

## Test/verification plan

1. `cd sdk/typescript && pnpm run test` — 341 SLICE 8 baseline + ~20 new = ~361 passing
2. `cd sdk/python && python -m pytest tests/test_cross_language_fixtures.py` — ≥20 passing
3. `cd sdk/python && python -m pytest` — 965 + 20 = 985+ no regressions
4. Run Python-side first to generate the fixture outputs; commit fixture file + both tests asserting on the same outputs
5. If TS output differs from Python for any fixture → P0 BLOCKER (cross-language byte-equivalence is the §13 invariant)

## Anti-scope

- No release dance / NPM publish — SLICE 10
- No new helper modules (SLICE 6 + 7 own ids/promptHash/pricing/runPlan)
- No proto bump
- No identity-propagation RunContext (deferred per SLICE 7 R2)
- No Cohere/Llama tokenizer fixtures (those live in tokenizer suite, not D05)

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D05_ts_sdk_substrate/design.md) §11.2 (audit-chain determinism invariants), §13 cross-language parity, §8 slice 9 row
- SLICE 6: [`COV_S05_06_d05_ids_prompt_hash_pricing.md`](COV_S05_06_d05_ids_prompt_hash_pricing.md) — scattered FX1-FX7 fixtures consolidated here
- SLICE 7 R2 lesson: spec authority over slice doc — applies to fixture shape too
