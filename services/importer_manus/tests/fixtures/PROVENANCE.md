# Manus Billing Importer — Fixture Provenance

This file pins the provenance of
`services/importer_manus/tests/fixtures/manus_usage.json`, the
canonical sanitized snapshot the SpendGuard Manus billing importer
replays against during default-feature `cargo test` and the
`make demo-import-manus-fixture` regression.

Review-standards (D15 review-standards.md §1 + §4):

| ID | Pin |
|----|-----|
| `T9(a)` Capture / generator-run date | **2026-06-08 UTC** |
| `T9(b)` Operator | **Backend Architect (D15 agent run)** |
| `T9(c)` Generator script path | `services/importer_manus/scripts/generate_fixture.py` |
| `T9(c)` Generator script SHA-256 | `sha256:2f02de08c8ab7b94f11773ec0e6ed397454024babcef8c91dca296ddb35bf03b` |
| `T8`    PII / real-data assertion | **No PII. No real Manus customer data. All workspace + session IDs synthetic per T8 sentinel pattern (`ws_FAKE_*` / `ses_FAKE_*`).** |

## Fixture body checksum

`manus_usage.json` is the deterministic output of running:

```bash
python3 services/importer_manus/scripts/generate_fixture.py \
  > services/importer_manus/tests/fixtures/manus_usage.json
```

Current body SHA-256:

```
sha256:535d5cf688f486cac501436eb24e6357fa6c3677464001a0402e86686a50b0de
```

The body hash is recomputed at runtime by `FixtureLoader::sha256_hex()`
and stamped onto every emitted `ImportRecord` via
`fixture_provenance_sha256`. If the fixture is edited by hand without
re-running the generator, the body hash here drifts from the live hash
— `cargo test fixture_provenance_pin_holds` flags the drift.

## Synthetic-ID invariant (T8)

Every record in the fixture uses synthetic identifiers. The loader
hard-rejects non-synthetic IDs at parse time
(`FixtureLoadError::NonSyntheticId`) so a future PR that drops in
real customer data would fail CI before merge.

Sentinel patterns enforced in
`services/importer_manus/src/fixture.rs`:

```
^ws_FAKE_.+$    workspace IDs
^ses_FAKE_.+$   session IDs
```

## No bearer tokens

The fixture contains no `Authorization` header, no `Bearer ...` literal,
and no `FAKE_MANUS_TOKEN_*` sentinel — the importer's live HTTP path
(`live` feature) is the only code path that sees the real
`MANUS_API_TOKEN`, and fixture-mode is a strict no-network code path.
Review-standards T1 / T2 / T8 apply.

## 8 sessions × 3 tiers + edge cases (acceptance A1.5)

| # | session_id                          | workspace_id        | tier             | credits | status      |
|---|-------------------------------------|---------------------|------------------|---------|-------------|
| 1 | ses_FAKE_team_completed_001         | ws_FAKE_team_001    | team_plan        | 47      | completed   |
| 2 | ses_FAKE_team_failed_002            | ws_FAKE_team_001    | team_plan        | 12      | failed      |
| 3 | ses_FAKE_team_cancelled_003         | ws_FAKE_team_002    | team_plan        | 0       | cancelled   |
| 4 | ses_FAKE_team_inprogress_004        | ws_FAKE_team_002    | team_plan        | 8       | in_progress |
| 5 | ses_FAKE_enterprise_005             | ws_FAKE_ent_001     | enterprise       | 350     | completed   |
| 6 | ses_FAKE_byok_006                   | ws_FAKE_byok_001    | enterprise_byok  | 1024    | completed   |
| 7 | ses_FAKE_team_large_007             | ws_FAKE_team_001    | team_plan        | 950     | completed   |
| 8 | ses_FAKE_team_minimal_008           | ws_FAKE_team_003    | team_plan        | 1       | completed   |

The demo path emits **7** envelopes — row #4 (`in_progress`) is
filtered out by `FixtureLoader::terminal_records` per
review-standards E3.

team_plan terminal-row math (demo verifier asserts; A5.4):

```
47 + 12 + 0 + 950 + 1   = 1010 credits
1010 × 20_526 micro-USD = 20_731_260 micro-USD
```

## Regeneration

When the generator script changes intentionally:

1. Bump the script body — keep `synthetic_only = True` invariant.
2. Re-run `python3 .../generate_fixture.py > .../manus_usage.json`.
3. Update both SHA-256 pins above (`sha256 ...` of script + fixture).
4. Update the PROVENANCE date.
5. Re-run `cargo test -p spendguard-importer-manus` — golden
   CloudEvent test will catch any envelope drift.
