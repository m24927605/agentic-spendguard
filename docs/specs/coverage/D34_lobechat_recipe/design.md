# D34 — LobeChat Custom Base URL Recipe — `design.md`

> Status: Doc-first spec, single-tool deep-dive. Scope-lock.
> Siblings: `implementation.md`, `tests.md`, `acceptance.md`, `review-standards.md`.
> Build plan §2.3 (Tier 3 #D34). Strategy Pattern 2 row 11. Parent: [D03](../D03_base_url_landing/design.md) §1.1 row 11. Sibling: [D33](../D33_anythingllm_recipe/design.md) (same shape).
> Prereq: D02 `spendguard install`; `make demo-up DEMO_MODE=proxy`.
> Audience: Project owner, Technical Writer, R1-R5 reviewer.

---

## 1. What we are shipping and why

A recipe page at `docs.agenticspendguard.dev/docs/drop-in/lobechat/` takes a LobeChat operator from "I run LobeChat" to "every chat is in the SpendGuard ledger" in under 5 minutes. LobeChat (LobeHub, Apache-2.0, ~55k stars, Docker first-class) honours an `OPENAI_PROXY_URL` env var that rewrites the OpenAI upstream for every server-side chat. Pattern 2: set `OPENAI_PROXY_URL=http://egress-proxy:9000/v1`; SpendGuard reserves, forwards, commits. Zero LobeChat code change.

D03 lists LobeChat as row 11 of the drop-in matrix. D34 is the deep dive: env var on the Docker container, per-session UI alt path for client mode, notes for Docker / Vercel / Cloud / Client mode, gotchas, end-to-end smoke.

### 1.1 In-scope

- Starlight page at `/docs/drop-in/lobechat/` replacing the D03 SLICE 1 stub.
- `DEMO_MODE=lobechat_real` in `deploy/demo/Makefile` booting LobeChat alongside the proxy stack.
- `deploy/demo/lobechat_smoke.sh` (LobeChat with `OPENAI_PROXY_URL`; one chat via `/api/chat/openai`; assert 200).
- `deploy/demo/verify_step_lobechat_real.sql` asserting `reserve` + `commit_estimated` rows (reuses the D33 pattern).

### 1.2 Anti-scope

- **No plugin / fork.** Env-var config only.
- **No agent-skills / plugin marketplace / TTS / vision.** Chat path only.
- **No client-mode automation.** Browser-side keys are GUI-driven; smoke runs server mode only.
- **No LobeChat Cloud instructions.** Cloud cannot point at localhost; link to Helm.
- **No Vercel walkthrough.** One-line callout, not step-by-step.

---

## 2. Key design decisions

### 2.1 Two slices, one optional

**Slice 1 — Recipe page + demo mode + smoke.** Replaces the D03 stub, adds `DEMO_MODE=lobechat_real`, ships smoke + verify SQL. CI green run closes D03 row 11's `Live` conditional (per D03 `design.md` §3.2).

**Slice 2 — Screenshot + UX polish (optional).** One annotated PNG of the `Settings → Language Model → OpenAI → API Proxy Address` panel for the client-mode callout + voice pass. Descoped if Slice 1 ships under 400 LOC and R1 flags low marginal value.

### 2.2 Smoke uses LobeChat's server route, not the GUI

**Decision:** Configure via container `OPENAI_PROXY_URL` at boot (no API call); chat via `POST /api/chat/openai` with `ACCESS_CODE` header.

**Why:** LobeChat has no admin update-env API — `OPENAI_PROXY_URL` is boot-time, period. Simpler smoke than D33 (one POST). Server-mode `/api/chat/openai` is the only route honouring `OPENAI_PROXY_URL`; client-mode bypasses it.

### 2.3 Egress proxy, not sidecar HTTP companion

**Decision:** Point LobeChat at the egress proxy (`http://egress-proxy:9000/v1`), matching D03 `DEMO_MODE=proxy`. Sidecar HTTP companion (D09) is a one-line "production" callout.

**Why:** Egress proxy is the shipped Pattern 2 surface today; D09 timing is independent. Same rationale as D33 §2.3.

### 2.4 Pin the LobeChat image

**Decision:** Pin `lobehub/lobe-chat:1.40.0` (or the stable tag at slice-impl time). Bump only on smoke regression.

**Why:** `:latest` breaks reproducibility; an upstream env-var rename cascades into doc + smoke churn. Pin; let `tests.md` catch drift.

### 2.5 Use server mode, not client-database mode

**Decision:** Smoke boots LobeChat in `NEXT_PUBLIC_SERVICE_MODE=server` with `ACCESS_CODE` set. The recipe documents both modes; the smoke covers server only.

**Why:** Client mode persists keys in the browser and has no server-side chat route; server mode is the only path where `OPENAI_PROXY_URL` deterministically routes every request through SpendGuard. The client-mode callout points readers at the per-session UI override (Settings → Language Model → OpenAI → API Proxy Address) and warns that the env var is ignored in client mode — the biggest silent-failure mode for this page.
