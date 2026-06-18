# COV_S05_10 — D05 TS SDK substrate: publish pipeline (D05 closes)

> **Deliverable**: D05 TS SDK substrate
> **Slice**: 10 of 10 (S)
> **Spec set**: [`docs/specs/coverage/D05_ts_sdk_substrate/`](../../specs/coverage/D05_ts_sdk_substrate/)

## Scope

Closes D05. Lands the npm publish pipeline + release artifacts per design §6 + §8 slice 10 row:
1. GitHub Actions workflow with Trusted Publisher OIDC to npm (mirrors `sdk-publish.yml` PyPI flow). Triggered on `ts-sdk-v*` git tags. `npm publish --provenance` enabled.
2. `npm pack` size budget check (CI gate)
3. README.md + LICENSE_NOTICES.md (Apache 2.0 license + @noble/hashes BSD-3 attribution)
4. CHANGELOG.md with v0.1.0 entry
5. Pin embedded pricing snapshot to `pricing/seed.yaml` content as of release time (regenerate sdk/typescript/src/pricing/demo.ts)
6. Include `sdk/fixtures/cross-language/v1.json` in npm pack `files:` field per SLICE 9 R1 forward-compat note
7. version 0.1.0 in package.json
8. Tag `ts-sdk-v0.1.0` (release script, NOT auto-pushed)

Concretely:
- `.github/workflows/sdk-ts-publish.yml` — NEW:
  - Triggered on push of tag `ts-sdk-v*`
  - jobs: lint + typecheck + test + build + size-check + publish (with provenance OIDC)
  - id-token: write permission for OIDC
  - Reads npm-package-name from package.json
- `sdk/typescript/README.md` — NEW (or extend existing):
  - 1-page operator-facing README
  - Install + Quick start + Links to docs site
- `sdk/typescript/LICENSE_NOTICES.md` — NEW:
  - Apache 2.0 (SpendGuard SDK itself)
  - BSD-3-Clause attribution for @noble/hashes
- `sdk/typescript/CHANGELOG.md` — NEW:
  - v0.1.0 entry summarizing SLICE 1-9 deliverables
- `sdk/typescript/package.json`:
  - version "0.1.0"
  - `files:` field includes dist/* + ../../sdk/fixtures/cross-language/v1.json (or copies it under sdk/typescript/fixtures/ at build time)
  - Add `prepublishOnly` script that copies the fixture file
- `sdk/typescript/scripts/size-budget.sh` (or similar) — NEW:
  - Run `npm pack` 
  - Inspect tarball size
  - Fail if > 200 KB (somewhat loose bound to allow for the embedded pricing demo + fixture file)
- `sdk/typescript/src/pricing/demo.ts` — regenerate from current `deploy/demo/init/pricing/seed.yaml` content; verify pricing_version matches
- Version bump in tsup banner if any

## Files touched

| File | Why |
|------|-----|
| `.github/workflows/sdk-ts-publish.yml` | NEW publish workflow |
| `sdk/typescript/README.md` | NEW |
| `sdk/typescript/LICENSE_NOTICES.md` | NEW |
| `sdk/typescript/CHANGELOG.md` | NEW v0.1.0 |
| `sdk/typescript/package.json` | version + files + scripts |
| `sdk/typescript/scripts/size-budget.sh` | NEW CI size gate |
| `sdk/typescript/src/pricing/demo.ts` | regenerate from seed.yaml |

## Test/verification plan

1. `pnpm run typecheck` clean
2. `pnpm run test` — 366 SLICE 9 baseline; zero regression
3. `pnpm run build` clean; dist/index.js + subpaths
4. `npm pack --dry-run` shows the expected file list including v1.json
5. `npm pack` tarball size ≤ 200 KB
6. GitHub Actions workflow yaml passes `actionlint` (if available locally) or syntax check
7. README install snippet works (smoke test)
8. CHANGELOG v0.1.0 entry summarizes the 9 prior slices

## Anti-scope

- No actual publish to npm (this is the pipeline; release is a separate human action)
- No git tag push (release script for human operator)
- No proto bump (LLM_CALL_OUTCOME deferred to cross-component slice)
- No identity-propagation RunContext (deferred per SLICE 7 R2)
- No new RPC bodies (all 5 wired SLICE 4-5)

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D05_ts_sdk_substrate/design.md) §6 publish pipeline (line 24), §8 slice 10 row, §11 embedded pricing snapshot reference
- SLICE 9: [`COV_S05_09_d05_cross_language_fixtures.md`](COV_S05_09_d05_cross_language_fixtures.md) — v1.json packaging forward-compat note
- Python `sdk-publish.yml` reference pattern
