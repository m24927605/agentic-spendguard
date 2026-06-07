# Devin Billing Importer — Fixture Provenance

This file pins the provenance of
`services/importer_devin/tests/fixtures/devin_usage.json`, the
canonical sanitized snapshot the SpendGuard Devin billing importer
replays against during default-feature `cargo test` and the
`make demo-verify-import-devin-fixture` regression.

Review-standards (D14 review-standards.md §5):

| ID | Pin |
|----|-----|
| `P5(a)` Capture / generator-run date | **2026-06-08 UTC** |
| `P5(b)` Operator | **Backend Architect (D14 agent run)** |
| `P5(c)` Generator script path | `services/importer_devin/scripts/generate_fixture.py` |
| `P5(c)` Generator script SHA-256 | `sha256:dde360a8ba19327ebec4058186ae253b17137aa97f0034b5e68bd47941d5e4c2` |
| `P5(d)` PII / real-data assertion | **No PII. No real Devin team data. All IDs synthetic per T5 sentinel pattern (`TEAM_FIXTURE_NNN` / `SESSION_FIXTURE_NNN`).** |

## Fixture body checksum

`devin_usage.json` is the deterministic output of running:

```bash
python3 services/importer_devin/scripts/generate_fixture.py \
  > services/importer_devin/tests/fixtures/devin_usage.json
```

Current body SHA-256:

```
sha256:aa4c172164a8a6a5d4e97c6bde4ac455e01f5f37932b8a3561ef213049144807
```

The body hash is recomputed at runtime by `FixtureLoader::sha256_hex()`
and stamped onto every emitted `ImportRecord` via
`fixture_provenance_sha256` (review-standards S9). If the fixture is
edited by hand without re-running the generator, the body hash here
drifts from the live hash — `cargo test fixture_provenance_pin_holds`
flags the drift.

## Synthetic-ID invariant (T5)

Every record in the fixture uses synthetic identifiers. The loader
hard-rejects non-synthetic IDs at parse time
(`FixtureLoadError::NonSyntheticId`) so a future PR that drops in
real customer data would fail CI before merge.

Sentinel patterns enforced in
`services/importer_devin/src/fixture_loader.rs`:

```
^TEAM_FIXTURE_\d{3}$
^SESSION_FIXTURE_\d{3}$
```

## No bearer tokens

The fixture contains no `Authorization` header, no `Bearer …` literal,
and no `FAKE_DEVIN_TOKEN_*` sentinel — the importer's live HTTP path
(`live` feature) is the only code path that sees the real
`DEVIN_API_TOKEN`, and fixture-mode is a strict no-network code path.
Review-standards T1 / T7 / P6 apply.

## Regeneration

When the generator script changes intentionally:

1. Bump the script body — keep `synthetic_only = True` invariant.
2. Re-run `python3 .../generate_fixture.py > .../devin_usage.json`.
3. Update both SHA-256 pins above (`sha256 …` of script + fixture).
4. Update the PROVENANCE date.
5. Re-run `cargo test -p spendguard-importer-devin` — golden CloudEvent
   test will catch any envelope drift.
