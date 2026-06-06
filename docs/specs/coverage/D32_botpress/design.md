# D32 — Botpress Integration via Integration SDK — design.md

**Status:** Spec — Tier 3, build plan §2.3. **Owner:** Backend Architect.
**Siblings:** [`D10`](../D10_dify_plugin/design.md) (no-code platform pattern), [`D09`](../D09_kong_ai_gateway/design.md) (HTTP+mTLS companion reused), [`D05`](../D05_ts_sdk_substrate/design.md) (TS substrate).

---

## 1. Problem

Botpress (~14.7k stars; MIT v12 OSS; Cloud SaaS) ships an **Integration SDK** (`@botpress/sdk`) plus **hooks** with before/after model-call slots (`beforeAiGeneration` / `afterAiGeneration` in `>= 0.7`) — the strongest pre/post-model-call extension point in Category D. Today a Botpress bot calling OpenAI / Anthropic / Bedrock has no pre-call dollar gate, no reservation, no signed audit.

D32 ships SpendGuard as `@spendguard/botpress-integration` on npm. Install via `botpress integrations push` (self-host v12) or marketplace (Cloud once approved). `beforeAiGeneration` reserves against the D09 sidecar HTTP companion; `afterAiGeneration` commits real usage; try/catch wraps release. Same lifecycle as LiteLLM proxy callback and D10, different transport (HTTP+mTLS) and language (TypeScript on Node 20).

## 2. Goals

1. Publish `@spendguard/botpress-integration` v0.1.0, Apache-2.0, at `integrations/botpress/`, peer-deps `@spendguard/sdk@^0.1.0` + `@botpress/sdk@^0.7`.
2. Distributable via `botpress integrations push`; works on v12 self-host and Cloud.
3. `beforeAiGeneration` reserves **before** Botpress dispatches upstream; DENY/DEGRADE throw `RuntimeError` (fail-closed).
4. `afterAiGeneration` commits with real usage (`event.payload.usage.inputTokens + outputTokens`).
5. Config: `upstreamProvider` ∈ `{openai, anthropic, bedrock}` + sidecar URL + budget/window IDs. Botpress owns upstream HTTP; this is a hook-side guardrail, NOT a forwarder (delta vs D10).
6. Demo `make demo-up DEMO_MODE=botpress_real` boots Botpress v12 + integration + sidecar; verifies reserve → reply → commit, deny short-circuits.
7. Docs page `docs/site/docs/integrations/botpress.md`.
8. R1-R5 loop per build-plan §1.1.

## 3. Non-goals

- Workflow-node gating beyond AI hook (RAG, tool-call): hook surface varies by version.
- Replacing native rate limit (orthogonal).
- Channel plugins (WhatsApp / Slack / Web Chat).
- Botpress Cloud OAuth: passthrough only.
- Token-by-token mid-stream cap.
- Botpress v11 or `@botpress/sdk` < 0.7.
- Strategy C: deferred to v1.1.

## 4. Architecture

`beforeAiGeneration` → `SpendGuardReservation` delegate builds `BudgetBinding` from config + conv ctx, estimator projects claim, `SpendGuardClient` (D05) POSTs `/v1/decision` to the sidecar HTTP companion over loopback mTLS. ALLOW returns; DENY/DEGRADE throw `RuntimeError` with `code: "BUDGET_DENIED" | "BUDGET_DEGRADED"`. Botpress then dispatches upstream HTTP (own client). `afterAiGeneration` fires post-generation → `commitSuccess(real_usage)` via `/v1/trace`. Hook-error path → `releaseFailure(exc)` with `outcome=FAILURE`.

Botpress runs as Node process; SpendGuard sidecar is a sibling pod (Helm) or co-host (compose). Transport is HTTP+mTLS via the companion endpoints D09 SLICE 1 added — D32 **reuses, does not extend** the companion contract. No UDS: Botpress Node and Rust sidecar live in separate containers (mirrors D09 §3.1).

## 5. Key decisions

- **Hook-based guardrail, not forwarder.** Unlike D10 (proxies upstream through a daemon), D32 lets Botpress own upstream HTTP and intercepts before/after only. Cleaner failure surface.
- **`SpendGuardReservation` delegate, not subclass.** Composition; entry only translates the hook signature. Same as D10 `_DifyReservation`, D11 `_delegate`.
- **HTTP+mTLS reuses D09 SLICE 1.** No new sidecar listener. If D32 starts first, SLICE 1 absorbs the dependency behind a feature flag (mirrors D09 §6).
- **TypeScript ESM, Node 20.10+.** Mirrors D05/D04/D06/D08/D29. Bundle < 100 KB (peer-deps externalised).
- **End-of-hook commit only.** Mid-stream cap out of scope.
- **`validateConfiguration` issues a 1-token reserve+release roundtrip** at install time (same as D10 INV-4).
- **Conversation mapping:** `conversation.id` → `session_id`; `botId` → `tenant_id` default (overridable); `userId` → `actor_id`.
- **Fail-closed default.** DEGRADE → `RuntimeError`. Dev escape: `SPENDGUARD_BOTPRESS_FAIL_OPEN=1`.
- **Strategy C deferred to v1.1.**

## 6. Slice plan (5 slices)

| # | Name | Size | Scope |
|---|------|------|-------|
| 1 | `COV_D32_01_integration_scaffold` | S | `integrations/botpress/` via `botpress integrations init`; tsup + biome + vitest; peer-deps pinned. |
| 2 | `COV_D32_02_hooks_registration_skeleton` | M | `SpendGuardReservation` delegate + before/after hooks stubbed; Zod config; `validateConfiguration` lifecycle. |
| 3 | `COV_D32_03_reserve_commit_wiring` | M | Reserve in beforeAi (DENY/DEGRADE throw, ALLOW returns); commit in afterAi with real usage; release on error; D05 `deriveIdempotencyKey`. |
| 4 | `COV_D32_04_tests_against_botpress_v12` | M | Vitest unit + integration suite booting real Botpress v12 via testcontainers; ≥ 37 unit + 4 integration tests; msw mock sidecar. |
| 5 | `COV_D32_05_demo_and_docs` | M | `DEMO_MODE=botpress_real` + `compose.botpress.yaml` + `verify_step_botpress.sql`; docs page + README row + npm OIDC publish workflow. |

~1900 LOC (~900 impl + 700 test + 300 yaml/docs/compose). Slice 4 heaviest (Botpress v12 boot in CI).

## 7. Open questions (locked)

1. **SDK floor:** `@botpress/sdk >= 0.7.0, < 0.8.0`. 0.6 lacked `beforeAiGeneration`; 0.8 is follow-up.
2. **`upstreamProvider`:** Cloud reads via `client.getModel`; self-host declares in integration config.
3. **Streaming usage:** Botpress aggregates SSE and emits totalled `usage` on `afterAiGeneration`. Missing → estimator-snapshot commit + WARN (D10 INV-5).
4. **Bedrock IAM / GCP SA:** passthrough only; Botpress owns SigV4.
5. **Install path:** `botpress integrations push` sideload is v1 invariant; marketplace push is follow-up.
6. **D09 SLICE 1:** if D32 starts first, SLICE 1 absorbs companion extraction.

---

*Locked decisions: §5 (all). Slice plan: §6 (5 slices). Anti-scope: §3.*
