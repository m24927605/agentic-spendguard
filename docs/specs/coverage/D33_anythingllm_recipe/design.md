# D33 — AnythingLLM Custom Base URL Recipe — `design.md`

> Status: Doc-first spec, single-tool deep-dive. Scope-lock.
> Siblings: `implementation.md`, `tests.md`, `acceptance.md`, `review-standards.md`.
> Build plan §2.3 (Tier 3 #D33). Strategy Pattern 2 row 10. Parent: [D03](../D03_base_url_landing/design.md) §3.6 stubs the link target; D33 ships the real page.
> Prereq: D02 `spendguard install` self-hosted; `make demo-up DEMO_MODE=proxy` localhost.
> Audience: Project owner, Technical Writer, R1-R5 reviewer.

---

## 1. What we are shipping and why

A single recipe page on `docs.agenticspendguard.dev/docs/drop-in/anythingllm/` that takes an AnythingLLM operator from "I run AnythingLLM" to "every chat is reserved and committed in the SpendGuard ledger" in under 5 minutes. AnythingLLM (Mintplex Labs, MIT, ~54k stars, 30+ providers, desktop + Docker) exposes a **Generic OpenAI** provider in its LLM Preference UI that accepts a custom base URL. Pattern 2: set the base URL to the SpendGuard egress proxy at `http://localhost:9000/v1`; SpendGuard reserves the predicted spend, forwards to the real upstream, and commits actual usage on response. Zero AnythingLLM code change.

D03 already lists AnythingLLM as row 10 of the drop-in matrix with a one-line `Setting` block. D33 is the deep dive: the four exact fields to fill in (Base URL, API Key, Chat Model Name, Token context window), per-deployment notes for Desktop / Docker / Cloud, gotchas, and a verified end-to-end smoke.

### 1.1 In-scope

- Starlight page at `/docs/drop-in/anythingllm/` replacing the D03 stub.
- `DEMO_MODE=anythingllm_real` in `deploy/demo/Makefile` that boots AnythingLLM alongside the proxy stack.
- `deploy/demo/anythingllm_smoke.sh` modelled on `proxy_smoke.sh` (configures the Generic OpenAI provider via `/api/v1/system/update-env`, sends one chat, asserts a 200 + content).
- `deploy/demo/verify_step_anythingllm_real.sql` asserting `reserve` + `commit_estimated` rows exist for the smoke's call.

### 1.2 Anti-scope

- **No AnythingLLM SDK / wrapper / fork.** Pattern 2 is configuration-only.
- **No embedding / voice / multimodal coverage.** Chat path only.
- **No Desktop install automation.** Recipe documents Desktop; smoke runs Docker only (Desktop is GUI-driven, not CI-amenable).
- **No Agent-Skills walkthrough.** Inherits SpendGuard via base URL; link upstream and move on.
- **No Cloud-tier (AnythingLLM Cloud) instructions.** Cloud cannot point at localhost; link to `/docs/deployment/helm/`.

---

## 2. Key design decisions

### 2.1 Two slices, one optional

**Slice 1 — Recipe page + demo mode + smoke.** Replaces the D03 stub, adds `DEMO_MODE=anythingllm_real`, ships smoke + verify SQL. Slice 1's CI green run is the evidence that promotes D03 row 10 `Verified` from `Spec` to `Live` (per D03 `acceptance.md` §A4) — the promotion edit ships in the same slice.

**Slice 2 — Screenshots + UX polish (optional).** Two annotated PNGs of the LLM Preference panel + voice pass. Descoped automatically if Slice 1 ships under 400 LOC and the R1 reviewer flags low marginal value.

### 2.2 Smoke uses AnythingLLM's HTTP API, not the GUI

**Decision:** Configure via `/api/v1/system/update-env` (`LLMProvider`, `GenericOpenAiBasePath`, `GenericOpenAiKey`, `GenericOpenAiModelPref`); chat via `/api/v1/workspace/{slug}/chat`. No Playwright.

**Why:** GUI automation is brittle across AnythingLLM versions; the system-config API is stable and documented. Screenshots cover the GUI path for human readers; the API path covers CI.

### 2.3 Egress proxy, not sidecar HTTP companion

**Decision:** Point AnythingLLM at the egress proxy (`http://localhost:9000/v1`), matching D03 `DEMO_MODE=proxy`. The sidecar HTTP companion (D09 SLICE 1 listener) gets a one-line "production deployments" callout but is not the first-time path.

**Why:** Egress proxy is the shipped Pattern 2 surface today; D09 may or may not land before D33. Keeps D33 unblocked.

### 2.4 Pin the AnythingLLM image

**Decision:** Pin `mintplexlabs/anythingllm:1.8.4` in the demo compose. Bump only when smoke regressions force it.

**Why:** `:latest` breaks reproducibility; an upstream UI rename cascades into doc + screenshot churn. Pin and let `tests.md` catch upstream drift.
