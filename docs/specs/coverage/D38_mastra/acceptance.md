# D38 — Acceptance Gates

Gates a reviewer (`superpowers:code-reviewer`) re-runs to confirm D38 is shipped. Every gate is runnable in this repo today (paths exist or are created by the named slice); no gate depends on a third-party action SpendGuard cannot trigger. Commands run from the repo root unless a `-C` directory is given.

## 1. Phase-0 reconciliation gates (slice 0 — must be green before any other slice merges)

| Gate | Command | Pass condition |
|---|---|---|
| A1.1 | `grep -n "## 9. Amendment 2026-06-10 (D38 Phase-0)" docs/specs/coverage/D06_vercel_ai_sdk/design.md` | exactly one match; section is APPENDED (everything above it byte-identical to the pre-slice file — verify with `git diff <base> -- docs/specs/coverage/D06_vercel_ai_sdk/design.md` showing additions only) |
| A1.2 | `node -p 'require("./sdk/typescript-vercel-ai/package.json").peerDependencies.ai'` | `>=4.0.0 <5` |
| A1.3 | `node -p 'require("./sdk/typescript-vercel-ai/package.json").version'` | `0.2.0` |
| A1.4 | `grep -n "0.2.0" sdk/typescript-vercel-ai/CHANGELOG.md` | entry exists; cites the v1-middleware/v4-line rationale and points Mastra users to `@spendguard/mastra` |
| A1.5 | `pnpm -C sdk/typescript-vercel-ai run test && pnpm -C sdk/typescript-vercel-ai run typecheck` | exit 0 — zero source changes under `sdk/typescript-vercel-ai/src/` (`git diff --stat` empty for that path) |
| A1.6 | `make demo-up DEMO_MODE=vercel_ai_mastra && make -C deploy/demo demo-verify-vercel-ai-mastra` | both exit 0 (D06 demo non-regression, first run) |

## 2. Build + lint + typecheck (`sdk/typescript-mastra/`)

| Gate | Command | Pass condition |
|---|---|---|
| A2.1 | `pnpm install --frozen-lockfile` (repo root) | exit 0; `pnpm-workspace.yaml` includes `sdk/typescript-mastra` |
| A2.2 | `pnpm -C sdk/typescript-mastra run lint` | biome zero diagnostics |
| A2.3 | `pnpm -C sdk/typescript-mastra run typecheck` | exit 0 — includes `SpendGuardProcessor implements Processor` conformance against the installed `@mastra/core` devDep (^1.41.0) |
| A2.4 | `pnpm -C sdk/typescript-mastra run build` | tsup produces `dist/index.js` + `dist/index.d.ts`, ESM-only, no CJS artifact |
| A2.5 | `pnpm -C sdk/typescript-mastra run size` | `dist/index.js` ≤ 40 KB minified, ≤ 12 KB gzipped; breach = build failure |
| A2.6 | `node -p 'require("./sdk/typescript-mastra/package.json").engines.node'` | `>=22.13.0` |
| A2.7 | `node -p 'require("./sdk/typescript-mastra/package.json").peerDependencies["@mastra/core"]'` | `>=1.0.0 <2` |

## 3. Test gates

| Gate | Command | Pass condition |
|---|---|---|
| A3.1 | `pnpm -C sdk/typescript-mastra run test` | vitest exit 0; coverage ≥ 90 % stmt / ≥ 85 % branch overall (tests.md §1 per-module floors) |
| A3.2 | `pnpm -C sdk/typescript-mastra run test tests/lockedSurface.test.ts` | TP-01..TP-06 pass — verbatim surface, no fail-open key, error-class identity |
| A3.3 | `pnpm -C sdk/typescript-mastra run test tests/identity.test.ts` | TP-07..TP-09 pass — derivation delegates to substrate; golden vector byte-equal |
| A3.4 | `pnpm -C sdk/typescript-mastra run test tests/failClosed.test.ts` | TP-10, TP-13..TP-16 pass — **DENY-before-inner-call proven with a real `@mastra/core` Agent + stub model recording 0 provider calls** |
| A3.5 | `pnpm -C sdk/typescript-mastra run test tests/processor.test.ts tests/usage.test.ts` | TP-11..TP-12, TP-17..TP-31 pass — reserve shape, unitId threading (TP-19), commit actuals + §6.6 fallback, streaming whole-step bracket, at-most-one-commit |
| A3.6 | `pnpm -C sdk/typescript-mastra run test tests/inflight.test.ts` | TP-32..TP-35 pass |
| A3.7 | `pnpm -C sdk/typescript-mastra run test tests/hashReuse.test.ts` | TP-36..TP-38 pass — zero `@noble/hashes` / `node:crypto` / `createHash` / `createHmac` / `blake2` tokens in `src/` AND `dist/index.js`; no hash dep in package.json |
| A3.8 | `pnpm -C sdk/typescript-mastra run test tests/mastraIntegration.test.ts` | TP-22 passes — processor fires on a model-router-string Agent |

## 4. Public-surface gates

| Gate | Command | Pass condition |
|---|---|---|
| A4.1 | `cd sdk/typescript-mastra && pnpm pack` then in a tmp dir `npm i <tarball>` + `node -e 'import("@spendguard/mastra").then(m => console.log(Object.keys(m).sort()))'` | exactly `["DecisionDenied","SidecarUnavailable","SpendGuardError","SpendGuardProcessor","VERSION"]` |
| A4.2 | `tar -tzf spendguard-mastra-0.1.0.tgz \| grep -E "src/\|tests/\|node_modules"` | empty — only `dist/`, README, LICENSE_NOTICES, CHANGELOG ship |
| A4.3 | `tests/_support/sampleConsumer.ts` constructs `new SpendGuardProcessor({ client, tenantId, unitId })` and mounts it on a typed `Agent` | `pnpm run typecheck` passes |
| A4.4 | `git diff <design-lock-commit> -- docs/specs/coverage/D38_mastra/design.md` | §5 verbatim block unchanged since lock (any change = re-spec, not a slice) |

## 5. Demo gates (slice 5 — `DEMO_MODE=mastra_processor`)

| Gate | Command | Pass condition |
|---|---|---|
| A5.1 | `make demo-up DEMO_MODE=mastra_processor` | exit 0; runner prints LOCKED line `[demo] mastra_processor ALL 3 steps PASS (ALLOW + DENY + STREAM)` |
| A5.2 | (within A5.1) step 2 DENY | runner asserts counting-stub `/_count` UNCHANGED across the DENY step and the rejection carries DENY evidence by direct/cause-chain typed error when Mastra preserves it, or by the §6.7 amendment #5 message-match fallback at the Agent boundary — live fail-closed + zero-provider-call proof |
| A5.3 | `make -C deploy/demo demo-verify-mastra-processor` | `verify_step_mastra_processor.sql` passes against `spendguard_ledger` with `ON_ERROR_STOP=1`: `COV_D38_GATE` assertions — reserve ≥ 2, commit_estimated ≥ 2, denied_decision ≥ 1, INV-2 strict-order (earliest reserve < earliest `spendguard.audit.outcome`), audit decision rows ≥ 2 |
| A5.4 | (within A5.3) canonical + outbox blocks | cross-DB `spendguard_canonical` decision/outcome count check passes; outbox-closure check passes (mirrors `demo-verify-langchain-ts` target structure) |
| A5.5 | `grep -n "mastra_processor" deploy/demo/Makefile` | `demo-up` branch + run/verify dispatch branch + `demo-verify-mastra-processor` target + `.PHONY` entry all present |
| A5.6 | `grep -n "node:22.13" deploy/demo/mastra_processor/docker-compose.yaml` | runner image is the Node ≥ 22.13 base (Mastra floor) |
| A5.7 | `grep -n "SPENDGUARD_UNIT_ID" deploy/demo/mastra_processor/docker-compose.yaml` | set to `66666666-6666-4666-8666-666666666666`; combined with A5.3's reserve ≥ 2 this is the day-1 unitId E2E proof (empty unit_id would have been rejected) |
| A5.8 | `git diff --stat -- deploy/demo/vercel_ai_mastra deploy/demo/verify_step_vercel_ai_mastra.sql` | empty across ALL D38 slices |

## 6. D06 non-regression (final)

| Gate | Command | Pass condition |
|---|---|---|
| A6.1 | `make demo-up DEMO_MODE=vercel_ai_mastra` | exit 0; `[demo] vercel_ai_mastra ALL 3 steps PASS (ALLOW + DENY + STREAM)` printed |
| A6.2 | `make -C deploy/demo demo-verify-vercel-ai-mastra` | exit 0 — run again AFTER the final D38 slice (first run was A1.6) |
| A6.3 | `pnpm -C sdk/typescript-vercel-ai run test` | green at head |

## 7. Documentation gates (slice 6)

| Gate | Path | Pass condition |
|---|---|---|
| A7.1 | `sdk/typescript-mastra/README.md` | install + quickstart mounting `SpendGuardProcessor` on an Agent (V5-pinned key, both router-string and explicit-instance variants); positioning paragraph derived from design §2 (factual contrast, complementary framing, no disparagement); fail-closed posture stated up front |
| A7.2 | `sdk/typescript-mastra/README.md` known-limitations box | auxiliary LLM calls (memory titles, ModerationProcessor classifier, scorers) out of v1 + the D06 explicit-wrap workaround |
| A7.3 | `sdk/typescript-mastra/CHANGELOG.md` | `0.1.0` entry: supersedes D06's transitive-Mastra claim; cites design §2 positioning; notes fail-closed-only |
| A7.4 | `sdk/typescript-mastra/LICENSE_NOTICES.md` | `@mastra/core` (Apache-2.0, `ee/` excluded note), `@spendguard/sdk` (Apache-2.0) |
| A7.5 | docs site integrations page for Mastra (`docs/site-v2/src/content/docs/docs/integrations/mastra.mdx`) | quickstart + positioning + limitation box; D06's vercel-ai page gains a cross-link ("using Mastra? see @spendguard/mastra"); embedded code blocks use `is:raw` (Astro memory rule) |
| A7.6 | `README.md` (repo root) adapter table | `@spendguard/mastra` row added |
| A7.7 | `.github/workflows/sdk-ts-mastra-publish.yml` | exists; `permissions: id-token: write`; `npm publish --provenance --access public`; gated on `mastra-v*` tag; lint/typecheck/test/build/size before publish; Node 22 runner |
| A7.8 | tests.md TA-12 | every `[VERIFY-AT-IMPL]` V1–V8 has a recorded pin (answer + `@mastra/core` version) in the owning slice doc |

## 8. Slice-level acceptance subsets

| Slice | Subset |
|---|---|
| COV_D38_00 | A1.1..A1.6 |
| COV_D38_01 | A2.1..A2.4, A2.6, A2.7; sanity import test |
| COV_D38_02 | A3.2, A3.3, A3.4, A3.6, A3.8; A4.3 |
| COV_D38_03 | A3.5 |
| COV_D38_04 | A3.1, A3.7; coverage floors |
| COV_D38_05 | A5.1..A5.8 |
| COV_D38_06 | A2.5, A4.1, A4.2, A6.1..A6.3, A7.1..A7.8 |

## 9. Ship-readiness checklist

- [ ] Every gate in §1–§7 green.
- [ ] `git status` clean under `sdk/typescript-mastra/`, `examples/mastra-processor/`, `deploy/demo/mastra_processor/`.
- [ ] `git log --oneline -- sdk/typescript-mastra/` shows ≥ 6 atomic commits (one per slice minimum).
- [ ] D06 demo verified green BOTH at slice 0 and at head (A1.6 + A6.1/A6.2).
- [ ] `make demo-up DEMO_MODE=mastra_processor` + `make -C deploy/demo demo-verify-mastra-processor` green at head (demo-as-quality-gate — reviewer sign-off without this run is invalid).
- [ ] All `[VERIFY-AT-IMPL]` markers pinned (TA-12); none silently dropped.
- [ ] Zero new hash code anywhere in the package (A3.7).
- [ ] `project_coverage_D38_shipped.md` memory entry written per build-plan §8 after merge.
