# Cross-language fixture corpus

Single source of truth for **Python ↔ TypeScript byte-equivalence** of the
audit-chain determinism functions:

- `derive_idempotency_key({tenant_id, session_id, run_id, step_id, llm_call_id, trigger})`
- `compute_prompt_hash(prompt_text, tenant_id)`
- `derive_uuid_from_signature(signature, scope)`

These three functions are P0 invariants under
[`docs/specs/coverage/D05_ts_sdk_substrate/design.md`](../../../docs/specs/coverage/D05_ts_sdk_substrate/design.md)
§11 (cross-language determinism gates) and
[`review-standards.md`](../../../docs/specs/coverage/D05_ts_sdk_substrate/review-standards.md)
§2 (cross-language byte-equivalence). Drift in any direction silently breaks:

- audit-chain rule dedup (Cost Advisor `(run_id, prompt_hash)` dedup window
  collapses to per-language buckets);
- idempotency replay collapse (retries lose their cache+ledger short-circuit
  because the TS adapter and Python adapter compute different keys);
- content-derived UUID slots (`decision_id` / `llm_call_id` / `audit_chain`
  rows diverge across runtimes for the same logical call).

## Files

| File | Purpose |
|---|---|
| `v1.json` | The fixture corpus. **NEVER edit in place once committed.** |
| `generate.py` | Reference generator. Run when minting a new corpus version. |
| `README.md` | This file. |

## Consumers

| Suite | Path | Asserts |
|---|---|---|
| Python | `sdk/python/tests/test_cross_language_fixtures.py` | Every fixture's Python output equals its `expected_output`. |
| TypeScript | `sdk/typescript/tests/crossLanguage.test.ts` | Every fixture's TS output equals its `expected_output`. |

Both suites read **the same v1.json file**. There is no per-language fixture
file — the whole point of this slice is that one corpus governs both
runtimes. Future Rust sidecar (Rust suite is forward-reserved per
[`tests.md`](../../../docs/specs/coverage/D05_ts_sdk_substrate/tests.md)
§3.9) consumes the same file.

## Schema (v1)

```json
{
  "version": 1,
  "generated_at": "YYYY-MM-DD",
  "generated_with": { "...": "..." },
  "fixtures": [
    {
      "id": "FX1",
      "fn": "derive_idempotency_key",
      "description": "...",
      "inputs": { "tenant_id": "...", "session_id": "...", "...": "..." },
      "expected_output": "sg-..."
    }
  ]
}
```

- `id`: stable, unique handle (FX*, FXP*, FXU* by function). Tests reference
  these directly for triage.
- `fn`: one of the three locked functions above. The harness dispatches by
  this field. An unknown `fn` is a loud failure on BOTH suites.
- `inputs`: keyword-arg shape, using the **Python field names** (snake_case).
  The TS harness adapter maps to camelCase before calling
  `deriveIdempotencyKey` / `deriveUuidFromSignature`.
- `expected_output`: the byte string produced by the live Python
  implementation at fixture generation time. Both suites assert against
  this value.

## Coverage targets (v1.json, ≥20 fixtures)

| Function | Count | Coverage |
|---|---|---|
| `derive_idempotency_key` | 8 (FX1–FX8) | ASCII / UUID tenant / empty trigger / all-empty / alternate trigger / multi-byte UTF-8 / Unit-Separator collision-safety probe. |
| `compute_prompt_hash` | 8 (FXP1–FXP8) | Empty prompt / ASCII / whitespace-strip / multi-byte UTF-8 / BOM-prefixed / control chars / 10KB long / mixed-case UUID canonicalisation. |
| `derive_uuid_from_signature` | 4 (FXU1–FXU4) | decision_id / llm_call_id (same sig, different scope) / audit_chain / custom scope. |

## Immutability + lifecycle

### NEVER edit `v1.json` in place once committed

`v1.json` is a **pinned** corpus. Every audit row produced by any version
of the SpendGuard SDK is implicitly anchored to the v1 hash semantics. An
in-place edit retroactively invalidates every audit row that referenced a
hash from before the edit — a soft data-loss event that the audit-chain
invariants forbid.

If the semantics of a function change and a new pinned output is required,
**mint `v2.json`** at this directory and migrate consumers explicitly:

1. Update the source implementation in `ids.py` / `prompt_hash.py` (or the
   TS mirror).
2. Run `generate.py` with the version arg bumped to write `v2.json`.
3. Update the Python + TS harnesses to load `v2.json` for the new vectors.
4. Keep `v1.json` checked in for archival regression coverage.
5. Document the compat window + cutoff in `CHANGELOG.md` of both SDKs.

### Adding a fixture to v1.json

A fixture can be **appended** as long as it does not change the semantics
of an existing function:

1. Add the fixture body to `_idempotency_vectors()` /
   `_prompt_hash_vectors()` / `_uuid_from_signature_vectors()` in
   `generate.py` with a new unique `id`.
2. Run `generate.py` (see below). The newly-appended fixture inherits its
   `expected_output` from the live Python implementation; existing
   fixtures' `expected_output` values MUST NOT change.
3. `git diff v1.json` — confirm only the new fixture entries are added and
   no existing `expected_output` line moved.
4. Run both test suites to confirm parity.

### Regenerate (or extend) v1.json

```bash
cd sdk/python
PYTHONPATH=src python ../fixtures/cross-language/generate.py \
    --out ../fixtures/cross-language/v1.json
```

The script reads the live Python implementation. If a regenerate produces a
diff in any `expected_output` for an existing fixture, **DO NOT commit**.
That diff is either:

- a legitimate v2 semantic break — mint `v2.json` instead; OR
- an accidental Python regression — revert the source change.

### When a fixture fails on either suite

- **Python suite fails on a fixture** → either Python regressed (revert the
  source change) OR `v1.json` was edited (revert the edit). Never "fix" by
  rewriting `expected_output`.
- **TS suite fails on a fixture** → TS implementation diverged from Python.
  This is the P0 cross-language drift case. Fix the TS implementation;
  never edit `v1.json`.

## Provenance

The corpus was bootstrapped at SLICE 9 (`COV_S05_09`) by consolidating the
scattered FX1–FX7 (`ids.test.ts`) and FXP1–FXP7-equivalent
(`promptHash.test.ts`) and FXU1–FXU5 fixtures that SLICES 6 + 7 had pinned
inline. SLICE 9 added FX8 + FXP5/FXP6/FXP7 + FXU3/FXU4 to hit the ≥20
volume floor and to widen the canonicalisation invariant coverage. The
original scattered fixtures remain in place; SLICE 10 may collapse them.

## References

- Slice doc: [`docs/internal/slices/COV_S05_09_d05_cross_language_fixtures.md`](../../../docs/internal/slices/COV_S05_09_d05_cross_language_fixtures.md).
- Design: [`docs/specs/coverage/D05_ts_sdk_substrate/design.md`](../../../docs/specs/coverage/D05_ts_sdk_substrate/design.md) §11.
- Review standards (P0 gate): [`docs/specs/coverage/D05_ts_sdk_substrate/review-standards.md`](../../../docs/specs/coverage/D05_ts_sdk_substrate/review-standards.md) §2.
- Test plan: [`docs/specs/coverage/D05_ts_sdk_substrate/tests.md`](../../../docs/specs/coverage/D05_ts_sdk_substrate/tests.md) §3.9.
