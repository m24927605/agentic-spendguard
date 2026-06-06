# D06 — Acceptance Gates

These are the gates a reviewer (`superpowers:code-reviewer`) re-runs to confirm D06 is shipped. Every gate is runnable in the current repo state per build plan §3.

## 1. Build + lint + typecheck

| Gate | Command (from `sdk/typescript-vercel-ai/`) | Pass condition |
|---|---|---|
| A1.1 | `pnpm install --frozen-lockfile` | exit 0; lockfile drift = fail |
| A1.2 | `pnpm run lint` | biome zero diagnostics |
| A1.3 | `pnpm run typecheck` | `tsc --noEmit` exit 0 |
| A1.4 | `pnpm run build` | tsup produces `dist/index.js`, `dist/index.d.ts`, plus `dist/mastra.{js,d.ts}` and `dist/streaming.{js,d.ts}`, exit 0 |
| A1.5 | `pnpm run size` | `dist/index.js` minified ≤ 30 KB (excluding peer-dep externals); gzipped ≤ 12 KB |

## 2. Test gates

| Gate | Command | Pass condition |
|---|---|---|
| A2.1 | `pnpm run test` | vitest exit 0; coverage ≥ 85% statements / 80% branches / 85% functions / 85% lines |
| A2.2 | `pnpm run test tests/identity.test.ts` | All 7 identity cases pass; cross-language parity case (2.7) reads `sdk/fixtures/cross-language/v1.json` and produces byte-equal output |
| A2.3 | `pnpm run test tests/streaming.test.ts` | All 6 streaming cases pass — finish/error mutex respected, cancellation rolls back |
| A2.4 | `pnpm run test tests/providers/` | All 12 provider cases pass (6 × openai + 6 × anthropic), no live API key needed |
| A2.5 | `pnpm run test tests/mastra/` | All 6 Mastra cases pass — including alias function-reference equality (5.6) |
| A2.6 | `pnpm run test tests/retry.test.ts` | All 3 retry/idempotency cases pass; exactly one commit + one release across retry scenarios |
| A2.7 | `pnpm run test tests/treeShaking.test.ts` | All 3 tree-shaking cases pass; bundle stays externalised |
| A2.8 | `pnpm run test:types` | `tsc --noEmit` over `tests/types.test-d.ts` passes; type-level expectations green |

## 3. Public-surface contract gates

The middleware satisfies the Vercel AI SDK v5 `LanguageModelV2Middleware` interface verbatim. The factory signature and option type are the customer-facing contract.

| Gate | Command | Pass condition |
|---|---|---|
| A3.1 | `pnpm pack && tar -tzf spendguard-vercel-ai-0.1.0.tgz \| grep -E "(index\|middleware\|streaming\|mastra)\.(js\|d\.ts)" \| wc -l` | ≥ 8 (each subpath: `.js` + `.d.ts`) |
| A3.2 | `node -e 'import("@spendguard/vercel-ai").then(m => console.log(Object.keys(m).sort()))'` after `pnpm pack && pnpm add ./spendguard-vercel-ai-0.1.0.tgz` in a tmp dir | Output includes `createSpendGuardMiddleware`, `wrapWithSpendGuard` |
| A3.3 | `node -e 'import("@spendguard/vercel-ai/mastra").then(m => console.log(Object.keys(m).sort()))'` | Output includes `createSpendGuardLanguageMiddleware` |
| A3.4 | Sample-adapter shim at `tests/_support/sampleConsumer.ts` imports and uses `wrapLanguageModel({ model, middleware: createSpendGuardMiddleware(...) })` against the v0.1.0 types | `pnpm run typecheck` passes |
| A3.5 | Confirm Mastra alias is a function-reference alias (not a copy): `createSpendGuardMiddleware === createSpendGuardLanguageMiddleware` strict equality | vitest assertion holds |

## 4. Vercel AI SDK demo gate (PRIMARY ACCEPTANCE)

| Gate | Command (from repo root) | Pass condition |
|---|---|---|
| A4.1 | `make demo MODE=agent_real_vercel_ai_ts` | Exit 0. Audit log emits LLM_CALL_PRE (reserve) then LLM_CALL_POST (commit) sequence at least once. |
| A4.2 | `grep -E "(LLM_CALL_PRE\|LLM_CALL_POST)" deploy/demo/artifacts/agent_real_vercel_ai_ts/audit.log \| wc -l` | ≥ 2 (one pre, one post). |
| A4.3 | Streaming sub-mode: `make demo MODE=agent_real_vercel_ai_ts_stream` exits 0 | Commit event timestamp > last stream chunk timestamp. Verified by `deploy/demo/scripts/verify_commit_ordering.sh`. |

## 5. Mastra demo gate (PRIMARY ACCEPTANCE)

| Gate | Command | Pass condition |
|---|---|---|
| A5.1 | `make demo MODE=agent_real_mastra` | Exit 0. Mastra Agent.generate() succeeds against the wrapped model. |
| A5.2 | `grep -E "(LLM_CALL_PRE\|LLM_CALL_POST)" deploy/demo/artifacts/agent_real_mastra/audit.log \| wc -l` | ≥ 2. |
| A5.3 | Demo uses `import { createSpendGuardLanguageMiddleware } from "@spendguard/vercel-ai/mastra"` — the documented Mastra entry — not the base entry | grep confirms the import |

## 6. Streaming-completion gate (PRIMARY ACCEPTANCE)

Streaming commit MUST fire when the stream completes, never before. Mid-stream cancellation MUST roll back.

| Gate | Command | Pass condition |
|---|---|---|
| A6.1 | `pnpm run test tests/streaming.test.ts -t "3.1"` | Commit fires exactly once after `finish` part lands |
| A6.2 | `pnpm run test tests/streaming.test.ts -t "3.3"` | `stream.cancel()` mid-stream triggers `onError` exactly once; commit never fires |
| A6.3 | Demo run: `make demo MODE=agent_real_vercel_ai_ts_stream` — inspect `audit.log` event ordering | `LLM_CALL_POST` timestamp > final `text-delta` chunk timestamp |
| A6.4 | Demo run with abort signal mid-stream (`deploy/demo/scripts/agent_real_vercel_ai_ts/stream_cancel.sh`) | Release event present; no commit event for that decisionId |

## 7. Provider matrix gate

| Gate | Command | Pass condition |
|---|---|---|
| A7.1 | `pnpm run test tests/providers/openai.test.ts` | 6/6 pass against `@ai-sdk/openai@^1` recorded fixtures |
| A7.2 | `pnpm run test tests/providers/anthropic.test.ts` | 6/6 pass against `@ai-sdk/anthropic@^1` recorded fixtures |
| A7.3 | No live provider key required at any point in the suite | grep CI logs for `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` env-checks: zero matches in test code |

## 8. Publish-pipeline dry-run

| Gate | Command | Pass condition |
|---|---|---|
| A8.1 | `pnpm pack` | Produces `spendguard-vercel-ai-0.1.0.tgz` including `dist/`, `README.md`, `CHANGELOG.md`, `LICENSE_NOTICES.md`, excluding `src/`, `tests/`, `node_modules/`, `scripts/`, `.github/` |
| A8.2 | `tar -tzf spendguard-vercel-ai-0.1.0.tgz \| grep -E "(src/\|tests/\|node_modules)"` | empty output |
| A8.3 | `du -k spendguard-vercel-ai-0.1.0.tgz` | ≤ 300 KB |
| A8.4 | `cat .github/workflows/sdk-ts-vercel-ai-publish.yml` | OIDC `id-token: write`; `npm publish --provenance`; gated on `ts-vercel-ai-v*` tag prefix |
| A8.5 | `gh workflow run sdk-ts-vercel-ai-publish.yml --ref <branch>` (workflow_dispatch) | Reaches the `Publish (provenance)` step; CI green for everything before it |

## 9. Documentation gates

| Gate | Path | Pass condition |
|---|---|---|
| A9.1 | `sdk/typescript-vercel-ai/README.md` | Has install + 30-line Vercel AI SDK quickstart + 30-line Mastra quickstart; both quickstarts show reserve → generate → commit |
| A9.2 | `sdk/typescript-vercel-ai/CHANGELOG.md` | 0.1.0 entry: "first public release; LanguageModelV2 middleware for Vercel AI SDK v5+ and Mastra Agents" |
| A9.3 | `sdk/typescript-vercel-ai/LICENSE_NOTICES.md` | Lists `ai`, `@ai-sdk/provider`, `@spendguard/sdk` notices |
| A9.4 | `docs/site/docs/integrations/vercel-ai-and-mastra.md` | Page exists. Sections: (a) Why one package covers both, (b) Vercel AI SDK quickstart, (c) Mastra quickstart, (d) Streaming behaviour + cancellation semantics, (e) RunPlan integration, (f) Troubleshooting |
| A9.5 | `README.md` (repo root) `## Adapter integrations` table | Row added: `@spendguard/vercel-ai` (TS) — "Vercel AI SDK + Mastra" — link to npm |
| A9.6 | Makefile | `demo MODE=agent_real_vercel_ai_ts` target exists; `demo MODE=agent_real_vercel_ai_ts_stream` and `demo MODE=agent_real_mastra` likewise |

## 10. Cross-language determinism gate (P0 — inherited from D05)

| Gate | Command | Pass condition |
|---|---|---|
| A10.1 | Cross-language fixture parity: `pnpm run test tests/identity.test.ts -t "2.7"` | All 64 fixture entries produce byte-identical `idempotencyKey` vs Python `pydantic_ai.py` derivation |
| A10.2 | Hand-spot-check: pick a random fixture entry, recompute `idempotencyKey` in Python via `from spendguard.ids import derive_idempotency_key`, compare TS output | exact match |

## 11. Runtime matrix

| Gate | Command | Pass condition |
|---|---|---|
| A11.1 | Node 22 LTS CI shard `pnpm run test` | exit 0 |
| A11.2 | Node 20.10 CI shard `pnpm run test` | exit 0 |
| A11.3 | Bun 1.1+ CI shard `bun test tests/{middleware,identity,streaming,claim}.test.ts` | exit 0 (advisory in v0.1.0) |

## 12. Definition-of-done checklist (per build plan §7)

- [ ] All 8 slices in `design.md` §7 merged into main
- [ ] All gates A1.x through A11.x run green
- [ ] README.md repo-root `## Adapter integrations` row added
- [ ] `docs/site/docs/integrations/vercel-ai-and-mastra.md` exists
- [ ] Makefile `demo MODE=agent_real_vercel_ai_ts` + `…_stream` + `…_mastra` entries exist
- [ ] Memory write-back: `project_coverage_D06_shipped.md` entry created per §8
