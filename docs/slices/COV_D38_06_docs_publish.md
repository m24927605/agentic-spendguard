# COV_D38_06 — D38 Mastra adapter: docs + publish pipeline

> **Deliverable**: D38 Mastra dedicated adapter (`@spendguard/mastra`)
> **Slice**: 6 of 7 (S — docs, packaging gates, publish workflow; no src changes)
> **Spec set**: [`docs/specs/coverage/D38_mastra/`](../specs/coverage/D38_mastra/)
> **Precedence**: `design.md` is LOCKED and trumps this doc (review-standards §1). Any disagreement here is a slice-author bug — follow design.md and flag the drift.

## Scope

Finish the deliverable surface: package README (quickstart with the V5-pinned mount key, both router-string and explicit-instance variants; fail-closed posture up front), CHANGELOG 0.1.0, LICENSE_NOTICES, docs-site integrations page (positioning derived from design §2 + the aux-LLM limitation box; `is:raw` on embedded code blocks per the Astro memory rule), repo-root README adapter-table row, the tag-gated npm publish workflow with provenance, and the size/pack gates (A2.5, A4.1, A4.2). Pins V8 (`withMastra()` plain-AI-SDK mounting — documented as supported variant B or as unsupported in v1). Closes the deliverable with the FINAL D06 non-regression run (TA-06 second run) and the VERIFY-register closure check (TA-12).

All positioning text derives from design §2: factual contrast only, sourced from upstream's own documentation, complementary framing, zero disparagement.

## Files touched

Exact set per implementation.md §8 (row COV_D38_06):

| File | Why |
|------|-----|
| `sdk/typescript-mastra/README.md` | full content: install + quickstart (V5-pinned key; router-string + explicit-instance), positioning per design §2, fail-closed up front, known-limitations box (A7.1/A7.2) |
| `sdk/typescript-mastra/CHANGELOG.md` | `0.1.0` entry: supersedes D06's transitive-Mastra claim; cites design §2 positioning; notes fail-closed-only (A7.3) |
| `sdk/typescript-mastra/LICENSE_NOTICES.md` | `@mastra/core` (Apache-2.0, `ee/` excluded note), `@spendguard/sdk` (Apache-2.0) (A7.4) |
| `docs/site-v2/src/content/docs/docs/integrations/mastra.mdx` | NEW docs page: quickstart + positioning + limitation box; code blocks `is:raw` (A7.5) |
| docs site vercel-ai integrations page | cross-link: "using Mastra? see `@spendguard/mastra`" (A7.5) |
| `README.md` (repo root) | `@spendguard/mastra` adapter table row (A7.6) |
| `.github/workflows/sdk-ts-mastra-publish.yml` | NEW — `permissions: id-token: write`; `npm publish --provenance --access public`; gated on `mastra-v*` tag; lint/typecheck/test/build/size before publish; Node 22 runner (A7.7) |

## LOCKED surface quoted verbatim

### Positioning — design.md §2 (canonical text; README / docs page / CHANGELOG derive from it)

> This section is the canonical positioning text. README / docs page / CHANGELOG derive from it. Rule: factual contrast only, sourced from upstream's own documentation; no disparagement of Mastra or `CostGuardProcessor`.

| Dimension | Mastra `CostGuardProcessor` (per its own docs) | `@spendguard/mastra` `SpendGuardProcessor` |
|---|---|---|
| Enforcement point | After cost data is observed; cost persisted **asynchronously** | **Pre-dispatch**: budget reserved BEFORE the provider call leaves the process |
| Ceiling semantics | "treat `maxCost` as a best-effort threshold, not a hard ceiling" | Hard ceiling: reservation against a durable ledger; DENY halts the step |
| Failure posture | **Fail-open** on missing context / query failure | **Fail-closed**: sidecar unreachable or DENY ⇒ step aborts with a typed error |
| Backing store | Requires OLAP observability store (DuckDB/ClickHouse; Postgres unsupported for metrics) | SpendGuard sidecar + Postgres ledger + signed audit chain (already deployed for every other SpendGuard adapter) |
| Scope | run / resource / thread, block or warn | tenant / budget / window via SpendGuard contract DSL; shared budgets across Python, LangChain, proxy, and gateway adapters |
| Cross-runtime budget | Mastra-only | Same `budget_id` enforced across every SpendGuard integration |

> The two are complementary: `CostGuardProcessor` remains a good soft-warn UX layer; `SpendGuardProcessor` is the hard enforcement layer. The docs page MUST say exactly that.

> **Vs. D06 (`@spendguard/vercel-ai`)**: D06 gates a *model instance*; D38 gates an *agent step*. Post Phase-0 (§9), D06's coverage claim is scoped to "explicit AI SDK model instances"; D38 owns Mastra Agents — both model-router strings and explicit instances — at the processor boundary.

### Aux-LLM limitation — design.md §4 (the docs page MUST carry this limitation box)

> - **Auxiliary LLM calls** — Mastra memory title generation, `ModerationProcessor`'s classifier call, scorers. OUT of v1 scope. Documented known limitation; workaround: wrap those models explicitly via D06 `wrapLanguageModel`. (Docs page MUST carry this limitation box.)

### Tarball + barrel surface (verifies design §5 at the published artifact level)

The packed module's runtime export keys must be exactly (acceptance.md A4.1):

```
["DecisionDenied","SidecarUnavailable","SpendGuardError","SpendGuardProcessor","VERSION"]
```

Bundle budget (implementation.md §2): **40 KB minified, 12 KB gzipped** for `dist/index.js`.

## VERIFY-AT-IMPL pins owned by this slice (design.md §12)

| ID | Question (design §12 verbatim) | Pre-declared alternatives (design §12 verbatim) | PIN (record at impl) |
|---|---|---|---|
| V8 | Does `withMastra()` (plain-AI-SDK mounting) run the same Processor hooks? | document as supported usage variant B / document as unsupported in v1 | _unpinned_ |

V8 is a documentation pin: the answer selects what the README/docs page SAY; it does not change any code path.

Also closes the register: TA-12 requires every V1–V8 marker to have a recorded answer + `@mastra/core` version in its owning slice doc (V1/V2/V3/V5 → COV_D38_02; V4/V7 → COV_D38_03; V6 → COV_D38_05; V8 → here).

## Test/verification plan (tests.md §4: TA-06 final run, TA-10, TA-12)

| ID | One-liner |
|----|-----------|
| TA-06 (final run) | D06 demo non-regression re-run AFTER the final slice: `vercel_ai_mastra` demo + verify green; zero diffs under the D06 demo files |
| TA-10 | `pnpm -C sdk/typescript-mastra run size` — dist/index.js ≤ 40 KB min / ≤ 12 KB gz |
| TA-12 | `[VERIFY-AT-IMPL]` register closure: every V1–V8 has a recorded pin (answer + `@mastra/core` version) in the owning slice doc |

## Acceptance gates (acceptance.md §8 subset: A2.5, A4.1, A4.2, A6.1..A6.3, A7.1..A7.8)

```sh
# A2.5 — size budget
pnpm -C sdk/typescript-mastra run size

# A4.1 — packed surface is exactly the design §5 barrel
cd sdk/typescript-mastra && pnpm pack
# in a tmp dir:
npm i <tarball> && node -e 'import("@spendguard/mastra").then(m => console.log(Object.keys(m).sort()))'
# expect: ["DecisionDenied","SidecarUnavailable","SpendGuardError","SpendGuardProcessor","VERSION"]

# A4.2 — tarball hygiene (only dist/, README, LICENSE_NOTICES, CHANGELOG)
tar -tzf spendguard-mastra-0.1.0.tgz | grep -E "src/|tests/|node_modules"   # empty

# A6.1..A6.3 — D06 non-regression, FINAL run
make demo-up DEMO_MODE=vercel_ai_mastra
make -C deploy/demo demo-verify-vercel-ai-mastra
pnpm -C sdk/typescript-vercel-ai run test

# A7.1..A7.8 — documentation gates (manual/grep verification per acceptance.md §7):
#  A7.1 README quickstart (V5-pinned key, both model-source variants) + §2-derived positioning + fail-closed up front
#  A7.2 README known-limitations box (aux LLM calls + D06 explicit-wrap workaround)
#  A7.3 CHANGELOG 0.1.0 entry (supersedes D06 transitive claim; fail-closed-only)
#  A7.4 LICENSE_NOTICES (@mastra/core Apache-2.0 + ee/ exclusion; @spendguard/sdk Apache-2.0)
#  A7.5 docs/site-v2/src/content/docs/docs/integrations/mastra.mdx + vercel-ai cross-link; is:raw code blocks
#  A7.6 repo-root README adapter row
#  A7.7 .github/workflows/sdk-ts-mastra-publish.yml (id-token: write; --provenance; mastra-v* tag; gates before publish; Node 22)
#  A7.8 TA-12 VERIFY register closure
```

Plus the acceptance.md §9 ship-readiness checklist (all gates green, working tree clean, ≥ 6 atomic commits, demo run at head, `project_coverage_D38_shipped.md` memory entry after merge).

## Anti-scope (review-standards §13 row COV_D38_06)

- Docs + publish workflow ONLY — NO `sdk/typescript-mastra/src/` changes. A doc-driven code "improvement" here is drift.
- NO npm publish execution in this slice — the workflow is tag-gated (`mastra-v*`); tagging/publishing is a post-merge release action.
- NO positioning language beyond design §2 derivation — no disparagement, no claims not sourced from upstream docs (review-standards §12.1).
- NO AI SDK v6 `LanguageModelV3` middleware work and no doc claims that D06 covers v5/v6 — the §9.1 amendment wording governs (design §9.2/§9.3).
- NO auxiliary-LLM coverage claims — the limitation box is mandatory, not optional (design §4).
- NO per-chunk stream gating claims — docs state the whole-step bracket posture (design §8).
- `deploy/demo/vercel_ai_mastra/**` + `verify_step_vercel_ai_mastra.sql` byte-untouched (A5.8 holds across ALL slices).

## Backlinks

- [`design.md`](../specs/coverage/D38_mastra/design.md) — §2 (positioning, LOCKED wording discipline), §4 (limitation box), §11.13–§11.14, §12 (V8), §13
- [`implementation.md`](../specs/coverage/D38_mastra/implementation.md) — §2 (bundle budget, files whitelist), §7 (tree-shaking), §8
- [`tests.md`](../specs/coverage/D38_mastra/tests.md) — §3 (TA-06/TA-10/TA-12), §4
- [`acceptance.md`](../specs/coverage/D38_mastra/acceptance.md) — §2 (A2.5), §4 (A4.1/A4.2), §6 (A6.1..A6.3), §7 (A7.1..A7.8), §8, §9 (ship-readiness)
- [`review-standards.md`](../specs/coverage/D38_mastra/review-standards.md) — §12 (documentation + positioning), §10 (package hygiene), §13
