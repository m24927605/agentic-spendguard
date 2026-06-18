# COV_D38_00 — D38 Mastra adapter: Phase-0 reconciliation

> **Deliverable**: D38 Mastra dedicated adapter (`@spendguard/mastra`)
> **Slice**: 0 of 7 (S — ≤200 LOC, docs + metadata only)
> **Spec set**: [`docs/specs/coverage/D38_mastra/`](../../specs/coverage/D38_mastra/)
> **Precedence**: `design.md` is LOCKED and trumps this doc (review-standards §1). Any disagreement here is a slice-author bug — follow design.md and flag the drift.

> NOTE-TO-ORCHESTRATOR: review-standards.md §"Reviewer prompt template" tells the reviewer to read slice docs at `docs/specs/coverage/D38_mastra/slices/<SLICE_ID>.md`, but per the build directive these slice docs live at `docs/internal/slices/COV_D38_*.md` (D04/D11 convention). Fill the reviewer template with the `docs/internal/slices/` paths or symlink; do not duplicate the docs.

## Scope

Land the design §9 Phase-0 reconciliation BEFORE any `@spendguard/mastra` claim ships: (1) append the dated amendment to D06's design.md re-scoping its Mastra coverage to explicit AI SDK instances, (2) tighten `@spendguard/vercel-ai`'s `ai` peer-dep to the truthful v4 range and bump to 0.2.0 with a CHANGELOG entry, (3) prove D06 non-regression (test suite + `vercel_ai_mastra` demo, first of two runs). History is not rewritten — the amendment is APPENDED; everything above it stays byte-identical.

This slice touches a *different package* (`sdk/typescript-vercel-ai`) plus D06 docs and must merge — with its own non-regression gates — before any other D38 slice (design §13 rationale). Zero files under `sdk/typescript-mastra/` exist after this slice.

## Files touched

Exact set per implementation.md §5 / §8:

| File | Why |
|------|-----|
| `docs/specs/coverage/D06_vercel_ai_sdk/design.md` | APPEND the §9.1 amendment section (dated 2026-06-10). Zero edits above the appended section |
| `sdk/typescript-vercel-ai/package.json` | `"ai": ">=4.0.0"` → `">=4.0.0 <5"` in `peerDependencies` (devDependencies stays `^4.0.0`, range-compatible); `"version": "0.1.0"` → `"0.2.0"` |
| `sdk/typescript-vercel-ai/CHANGELOG.md` | `0.2.0` entry: peer-dep correction rationale (design §9.2 wording), pointer to `@spendguard/mastra` for Mastra users, D06-follow-on note for v5/v6 middleware |

## LOCKED surface quoted verbatim

### D06 amendment text — design.md §9.1 (the appended section MUST carry this content)

> `## 9. Amendment 2026-06-10 (D38 Phase-0)` — (a) The §1/§3-era rationale "Mastra Agents call `generateText`/`streamText` from `ai` underneath" is stale: Mastra owns its own agent loop since v0.14.0 (Aug 2025). (b) D06's Mastra coverage is re-scoped to **explicit AI SDK `LanguageModel` instances** handed to Mastra (Mastra still consumes `doGenerate`/`doStream` model objects); the model-router string syntax has no `wrapLanguageModel` injection point and is covered by **D38** (`@spendguard/mastra`). (c) The `@spendguard/vercel-ai/mastra` subpath alias remains published and functional for explicit-instance users; its docs gain a pointer to `@spendguard/mastra` as the recommended Mastra integration. (d) Locked decision #5 ("AI SDK v5+ only") is corrected to match shipped reality — see §9.2.

Per design.md §9.1: "The original sections are left byte-intact above the amendment (no history rewrite). The title's \"(covers Mastra)\" stays for historical traceability; the amendment paragraph is the authoritative scope statement."

### Peer-dep decision — design.md §9.2 (LOCKED, justified deviation)

> **Decision (deviating from the default recommendation, with justification):** tighten the peer-dep to **`"ai": ">=4.0.0 <5"`**, released as `@spendguard/vercel-ai` **0.2.0** with a CHANGELOG entry. We explicitly do NOT adopt the `>=5.0.0 <7` tightening [...]

Reviewer guard (review-standards §8.2): "fixing" the range to `>=5.0.0 <7` is a P0 finding — the deviation is LOCKED. D06 design.md locked decision #5 is corrected by the amendment to (design §9.2 verbatim): "shipped 0.x targets the AI SDK v4 line (`LanguageModelV1Middleware`); v5 (`LanguageModelV2Middleware`) and v6 (`LanguageModelV3`) variants are the D06 follow-on."

### D06 demo non-regression — design.md §9.4 (HARD gate)

> `deploy/demo/vercel_ai_mastra/` and `deploy/demo/verify_step_vercel_ai_mastra.sql` are NOT touched by any D38 slice. Acceptance gate A6.4 re-runs `make demo-up DEMO_MODE=vercel_ai_mastra` + `make -C deploy/demo demo-verify-vercel-ai-mastra` green after Phase-0 and again after the final slice.

## VERIFY-AT-IMPL pins owned by this slice

None. No design §12 marker is pinned by COV_D38_00 (V1/V2/V3/V5 → COV_D38_02; V4/V7 → COV_D38_03; V6 → COV_D38_05; V8 → COV_D38_06).

## Test/verification plan (tests.md §4)

| ID | One-liner |
|----|-----------|
| TA-06 (first run) | D06 demo non-regression: `make demo-up DEMO_MODE=vercel_ai_mastra && make -C deploy/demo demo-verify-vercel-ai-mastra` green; `git diff --stat` shows zero changes under `deploy/demo/vercel_ai_mastra/` and `verify_step_vercel_ai_mastra.sql` |
| TA-07 | Phase-0 append-only proof: amendment commit exists; pre-amendment content of D06 design.md is byte-identical |
| TA-08 | `node -p 'require("./sdk/typescript-vercel-ai/package.json").peerDependencies.ai'` → `>=4.0.0 <5`; version `0.2.0`; `pnpm -C sdk/typescript-vercel-ai run test` green |

## Acceptance gates (acceptance.md §1; slice subset per §8: A1.1..A1.6)

```sh
# A1.1 — amendment appended, append-only
grep -n "## 9. Amendment 2026-06-10 (D38 Phase-0)" docs/specs/coverage/D06_vercel_ai_sdk/design.md
git diff <base> -- docs/specs/coverage/D06_vercel_ai_sdk/design.md   # additions only

# A1.2 / A1.3 — peer range + version
node -p 'require("./sdk/typescript-vercel-ai/package.json").peerDependencies.ai'   # >=4.0.0 <5
node -p 'require("./sdk/typescript-vercel-ai/package.json").version'               # 0.2.0

# A1.4 — CHANGELOG entry
grep -n "0.2.0" sdk/typescript-vercel-ai/CHANGELOG.md

# A1.5 — D06 package green, zero src changes
pnpm -C sdk/typescript-vercel-ai run test && pnpm -C sdk/typescript-vercel-ai run typecheck
git diff --stat -- sdk/typescript-vercel-ai/src/   # empty

# A1.6 — D06 demo non-regression (first run)
make demo-up DEMO_MODE=vercel_ai_mastra
make -C deploy/demo demo-verify-vercel-ai-mastra
```

## Anti-scope (review-standards §13 row COV_D38_00)

- NO files under `sdk/typescript-mastra/` — package init is COV_D38_01.
- NO source changes under `sdk/typescript-vercel-ai/src/` (A1.5 / review-standards §8.3) — this is a metadata + docs slice.
- NO AI SDK v5 (`LanguageModelV2Middleware`) / v6 (`LanguageModelV3`) middleware work — D06 follow-on, explicitly OUT of D38 (design §9.3, review-standards §8.5).
- `deploy/demo/vercel_ai_mastra/**` and `deploy/demo/verify_step_vercel_ai_mastra.sql` byte-untouched (design §9.4).
- No `@spendguard/vercel-ai` npm publish in this slice — version bump + CHANGELOG only; publishing follows the existing D06 pipeline outside D38 scope.
- No history rewrite of D06 design.md — append only.

## Backlinks

- [`design.md`](../../specs/coverage/D38_mastra/design.md) — §9 (Phase-0 reconciliation, all LOCKED decisions), §11.11, §13 (slice plan)
- [`implementation.md`](../../specs/coverage/D38_mastra/implementation.md) — §5 (Phase-0 file changes, exact), §8 (slice → file map)
- [`tests.md`](../../specs/coverage/D38_mastra/tests.md) — §3 TA-06/TA-07/TA-08, §4 (slice → test map)
- [`acceptance.md`](../../specs/coverage/D38_mastra/acceptance.md) — §1 (A1.1..A1.6), §8 (slice subsets)
- [`review-standards.md`](../../specs/coverage/D38_mastra/review-standards.md) — §8 (Phase-0, P0), §13 (anti-scope)
