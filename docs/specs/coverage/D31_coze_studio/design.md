# D31 — Coze Studio Model Provider — Design

**Status:** Spec — Tier 3, build plan §2.3.
**Parent strategy:** [`framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md) "No-code / visual agent builders".
**Build plan:** [`framework-coverage-build-plan-2026-06.md`](../../../strategy/framework-coverage-build-plan-2026-06.md) §2.3 D31.
**Owner:** Backend Architect.
**Siblings:** [`D09`](../D09_kong_ai_gateway/design.md) (HTTP+mTLS sidecar companion this reuses), [`D10`](../D10_dify_plugin/design.md) (closest no-code platform pattern).

## §1. What we're building

A SpendGuard packaging for **Coze Studio** (ByteDance OSS no-code agent builder, Apache-2.0, Go microservices). v1 ships **Pattern 2 — OpenAI-compatible base-URL redirect**: a Coze workspace config snippet pointing the workspace's "OpenAI" model provider at the SpendGuard sidecar HTTP companion (`/v1/openai/chat/completions`), plus a `DEMO_MODE=coze_studio_real` topology proving a real OpenAI call through Coze flows through SpendGuard reserve+commit, plus a docs page. No new sidecar code: the companion listener was extracted by D09 SLICE 1. v2 (deferred, §6) ships the Coze plugin SDK route.

## §2. Why this slot, why now

- Coze Studio open-sourced 2026-07 under Apache-2.0 — the most credible OSS alternative to Dify for ByteDance-aligned shops. Same coverage value as D10: no native budget primitive, no signed audit chain.
- Coze's custom-endpoint UI exposes a generic OpenAI-compatible base URL, so SpendGuard's existing HTTP companion satisfies the integration *without Coze-specific code*. Highest-ROI no-code slot — coverage cost is "yaml + docs + a docker-compose stanza".
- The plugin SDK route is tighter but materially more work and locks the binding to Coze's plugin ABI; defer until customer demand emerges.

## §3. Key architectural decisions

### 3.1 Pattern 2 (base-URL redirect) is the v1 surface

Artefact is a **Coze workspace YAML snippet** plus the existing SpendGuard sidecar HTTP companion `/v1/openai/chat/completions` endpoint (D09 SLICE 1). Operators paste the snippet, Coze's "OpenAI" provider POSTs to SpendGuard, SpendGuard reserves → forwards to real OpenAI → commits. Same shape as D03 / D33 / D34, wrapped in Coze's config format and demo'd against self-hosted Coze.

### 3.2 Reuse D09 SLICE 1 HTTP companion verbatim

No new sidecar listener. The companion's existing endpoints satisfy Coze's "OpenAI-compatible base URL" contract 1:1. D09 SLICE 1 is a strict prerequisite for D31 SLICES 2-4 (§6).

### 3.3 Tenant mapping via header injection

Coze provides per-provider custom-header config. Snippet instructs operators to inject `X-SpendGuard-Tenant-Id: <workspace_id>` and `X-SpendGuard-Budget-Id: <budget_id>`; the companion's existing tenant-resolver reads these. No Coze-side code.

### 3.4 Fail-closed default

Coze sees a real HTTP error (502 on DENY/DEGRADE) and surfaces it via its own retry/error UI. No `fail_open` flag in v1 — operators wanting fail-open switch to egress-proxy install (D02/D03).

### 3.5 v1 covers OpenAI shape only

Coze's UI exposes Anthropic / Gemini / Bedrock / "OpenAI-compatible" slots. v1 ships OpenAI-compatible only — broadest Coze surface. Other vendor slots track to D31 v1.1, gated on D09 SLICE 1 expanding the companion's forwarder family.

### 3.6 No Coze plugin SDK in v1

The Go plugin tool SDK would let a native plugin handle LLM calls intra-process. v1.1 follow-up. Pattern 2 already covers 100% of Coze model calls, so the SDK route adds tightness, not coverage.

## §4. Slice plan (4 slices)

| # | Name | Size | Scope |
|---|------|------|-------|
| 1 | `COV_D31_01_coze_workspace_config` | S | `examples/coze-studio/coze-workspace-config.yaml` snippet + `examples/coze-studio/README.md` + `examples/coze-studio/headers-cheatsheet.md`. No code. Pinned to a specific Coze Studio image digest. |
| 2 | `COV_D31_02_http_companion_smoke` | S | `examples/coze-studio/smoke.sh` curl-driven smoke against docker-compose Coze + sidecar; replays Coze's "test connection" probe through the companion, asserts a real audit row. Depends on D09 SLICE 1. |
| 3 | `COV_D31_03_demo_mode` | M | `DEMO_MODE=coze_studio_real` Makefile branch + `deploy/demo/coze_studio/compose.override.yaml` + `verify_step_coze_studio_real.sql` + demo driver step. Real OpenAI upstream. |
| 4 | `COV_D31_04_docs_page` | S | `docs/site/docs/integrations/coze-studio.md` page + README adapter row. |

~600 LOC total (~150 yaml/snippet + ~250 demo/compose + ~200 docs).

## §5. Anti-scope

- No native Coze plugin SDK (Pattern 3) in v1; v1.1 GH issue.
- No Anthropic / Gemini / Bedrock provider slots; OpenAI-compatible only.
- No mid-stream cap enforcement; commit at end-of-body matches D09 §3.3.
- No Coze Cloud (SaaS) integration; OSS self-host only.
- No SpendGuard-side new endpoints; companion is reused as-is.
- No Coze upstream PR; recipe lives in our repo.
- No multi-tenant Coze workspace federation; per-workspace snippet only.

## §6. Out-of-band coordination

**Hard dependency:** D09 SLICE 1 (`crates/spendguard-sidecar` HTTP companion with `/v1/openai/chat/completions` + tenant header resolver + mTLS+SVID) must be merged on `main` before D31 SLICES 2-4 can run. If D31 starts ahead of D09 SLICE 1, SLICE 1 (config snippet) can still ship; SLICES 2-4 block.

**Coze image pin:** self-hosted Coze Studio docker-compose stack pinned by image digest, not floating tag. Drift handled by the standard "pin and bump" pattern; SLICE 2 smoke is the canary.

---

*Locked decisions: §3.1, §3.2, §3.3, §3.4, §3.5, §3.6. Slice plan: §4 (4 slices). Anti-scope: §5. Dependencies: §6.*
