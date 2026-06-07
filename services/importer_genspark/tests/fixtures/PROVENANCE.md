# Genspark Billing Importer — Fixture Provenance

This file pins the provenance of
`services/importer_genspark/tests/fixtures/genspark_usage.json`, the
canonical sanitized snapshot the SpendGuard Genspark billing importer
replays against during default-feature `cargo test` and the
`make demo-verify-import-genspark-fixture` regression.

Review-standards (D16 review-standards.md §7):

| ID | Pin |
|----|-----|
| `P2(a)` Capture / generator-run date | **2026-06-08 UTC** |
| `P2(b)` Operator | **Backend Architect (D16 agent run)** |
| `P2(c)` Generator script path | `services/importer_genspark/scripts/generate_fixture.py` |
| `P2(c)` Generator script SHA-256 | `sha256:8fa710f9156e3b78b1baddc686d3f8df2f9ec7833b287beebb1b11de030b385b` |
| `P2(d)` PII / real-data assertion | **No PII. No real Genspark workspace data. All IDs synthetic per T9 sentinel pattern (`FAKE_ws_NNN` / `FAKE_task_NNN`).** |

## Fixture body checksum

`genspark_usage.json` is the deterministic output of running:

```bash
python3 services/importer_genspark/scripts/generate_fixture.py \
  > services/importer_genspark/tests/fixtures/genspark_usage.json
```

Current body SHA-256:

```
sha256:fd2c0bb772bfbf2605ce09204aed0025cd754c3edce7296ee281637e5a52baf6
```

The body hash is recomputed at runtime by `FixtureLoader::sha256_hex()`
and stamped onto every emitted `ImportRecord` via
`fixture_provenance_sha256`. If the fixture is edited by hand without
re-running the generator, the body hash here drifts from the live hash —
`cargo test fixture_provenance_pin_holds` flags the drift.

## Synthetic-ID invariant (T9)

Every record in the fixture uses synthetic identifiers. The loader
hard-rejects non-synthetic IDs at parse time
(`FixtureLoadError::NonSyntheticId`) so a future PR that drops in
real customer data would fail CI before merge.

Sentinel patterns enforced in
`services/importer_genspark/src/fixture_loader.rs`:

```
^FAKE_ws_\d{3}$
^FAKE_task_\d{3}$
```

## Plan coverage

The fixture exercises three code paths so the demo + CI both verify
the contract end-to-end:

1. **`plus`** — full-price conversion (records #1, #2):
   `credits × usd_per_credit × 1e6 = amount_micro_usd` with
   `reason_code = NULL`.
2. **`premium`** — higher-tier conversion (record #3): same shape as
   `plus` but with the premium per-credit rate.
3. **`enterprise`** — unknown-plan fallback (record #4): this slug is
   **NOT** in the embedded price table, forcing the
   `amount_micro_usd = 0` + `reason_code = genspark_plan_unknown`
   path (review-standards T7, F4).

The fixture record #4 is intentional — it asserts the demo can verify
the unknown-plan path without having to hand-edit the committed price
table. Removing record #4 would drop fixture coverage for the
fallback path.

## No bearer tokens

The fixture contains no `Authorization` header, no `Bearer …` literal,
and no `FAKE_GENSPARK_TOKEN_*` sentinel — the importer's live HTTP path
(`live` feature) is the only code path that sees the real
`GENSPARK_API_TOKEN`, and fixture-mode is a strict no-network code
path. Review-standards T1 / T10 apply.

## Regeneration

When the generator script changes intentionally:

1. Bump the script body — keep `synthetic_only = True` invariant.
2. Re-run `python3 .../generate_fixture.py > .../genspark_usage.json`.
3. Update both SHA-256 pins above (`sha256 …` of script + fixture).
4. Update the PROVENANCE date.
5. Re-run `cargo test -p spendguard-importer-genspark` — golden
   CloudEvent test will catch any envelope drift.
