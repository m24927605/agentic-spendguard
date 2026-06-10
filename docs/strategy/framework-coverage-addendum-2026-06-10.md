# Framework Coverage — Addendum (2026-06-10 trend research)

**Date:** 2026-06-11
**Status:** D38/D39 decided + SHIPPED; D40/D41 + AG-UI outreach are **enumerated candidates awaiting user decision** — not committed scope.
**Parent docs:** [`framework-coverage-2026-06.md`](framework-coverage-2026-06.md) (strategy memo) · [`framework-coverage-build-plan-2026-06.md`](framework-coverage-build-plan-2026-06.md) (D01-D37)

This addendum records the 2026-06-10 trend-research pass (5 parallel researchers, hard data + citations) that extended the coverage matrix beyond the original 37 deliverables, and enumerates the remaining candidate queue.

---

## §1. Decided + shipped (recorded for traceability)

| # | Deliverable | Verdict basis | Outcome |
|---|---|---|---|
| D38 | **Mastra dedicated adapter** | HIGH ROI — Mastra ≥0.14 owns its agent loop; default model-router strings (`"openai/gpt-4o"`) bypass `wrapLanguageModel`, so D06's transitive claim was stale. First-party `CostGuardProcessor` is documented best-effort/**fail-open** → "hard fail-closed pre-dispatch" slot was empty. `@mastra/core` ~966k npm/week, $22M Series A, Apache 2.0, no CLA/DCO. | SHIPPED 2026-06-11 (7 slices, closes `154b9a9`). `@spendguard/mastra` SpendGuardProcessor. |
| D39 | **AG-UI spend-event family** | MEDIUM ROI, display-only thesis — AG-UI never touches the LLM call path (cannot gate), but its `CUSTOM` event slot had **zero** cost/budget prior art. `@ag-ui/core` 3.6M npm/month (90-day rate ~2.4×); first-party integrations: LangGraph, CrewAI, MAF, ADK, Strands, Mastra, Pydantic AI, Agno, LlamaIndex, AG2; AWS Bedrock AgentCore managed support; Oracle adoption. MIT, single-vendor (CopilotKit) governance, all 0.x. | SHIPPED 2026-06-10 (3 slices, closes `bfae07d`). `spendguard.*` 5-event vocabulary, TS+Python byte parity. |
| — | AutoGen v0.5+ upgrade | **LOW ROI — no deliverable.** `ChatCompletionClient.create/create_stream` survived unchanged into the frozen 0.7.x line; AutoGen is officially maintenance-mode (README banner), users steered to MAF (D07 covers). D24 already forwards `tool_choice` (v0.6.2+ delta) with test `test_T33c`; verified 41/41 green 2026-06-10. | No work needed. |

Rejected in the same pass (substitutes scan): OpenAI AgentKit/Agent Builder (winding down 2026-11-30; Agents SDK already covered), DeepAgents (LangGraph-transitive — docs recipe at most), Cloudflare Agents (0.x + "not accepting external PRs" + Workers can't host sidecar — revisit at 1.0), VoltAgent (38.9k npm/month, below bar; `@ai-sdk/*` underneath → likely D06-transitive), Dapr Agents (690 stars / 12.9k PyPI/month below bar despite CNCF GA + ZEISS; sidecar-native fit — **watch item, recheck ~2026-Q4**), Mistral Agents API (server-side execution; Vibe CLI falls under existing base-URL/egress patterns), Temporal/Restate agent layers (LLM calls run through already-covered SDKs).

## §2. D40 candidate — OpenClaw (RANK 1, HIGH ROI)

**TAM (2026-06-10):** 378k GitHub stars / 79k forks (≈ +128k since 2026-03 — pre-March figures from secondary blog snapshots, treat delta as approximate); npm `openclaw` 5,860,844 downloads/30d; mainstream press cycle including the creator's "$1.3M OpenAI tokens in 30 days / 603B tokens" story (May 2026) — built-in market education for spend guardrails.

**Integration surface:** it is a gateway + agent runtime — the loop executes locally (exactly SpendGuard's interception plane). Three hook points:
1. **Provider plugins** — register a custom LLM provider that wraps reserve→dispatch→commit.
2. **`before_model_resolve` hook** — redirect traffic to a gated provider.
3. **Zero-code base-URL path** — `models.providers.*` custom base URLs with request adapters (`openai-completions`, `anthropic-messages`, `google-generative-ai`) — same pattern as the existing D03 drop-in lane.

**Risks:** rolling `vYYYY.M.D` versioning (no semver); plugins run in-process unsandboxed; hook-style plugins already marked legacy in favor of capability registration → adapter churn risk is real; **base-URL recipe is the durable floor**. Local-first single-operator topology (not k8s) — needs a lightweight local SpendGuard mode or hosted-proxy recipe rather than the sidecar Helm path.

**Competitive gap:** documented verbatim upstream — "no built-in hard spend cap"; built-ins are observability only (`/usage tokens|full`); community workaround is OpenRouter org budgets (gateway layer currently conceded to a competitor's hosted guardrail).

**License/gate:** MIT; no DCO/CLA documented (verify before first upstream PR).

**Recommended shape:** two-stage — (a) base-URL recipe doc + demo smoke (cheap, D03-pattern, durable); (b) provider-plugin adapter (4-6 slices, accept churn risk, pin versions).

## §3. D41 candidate — Voice vertical: LiveKit Agents + Pipecat (combined design)

**Thesis:** voice is the highest-token-burn agent vertical (continuous STT→LLM→TTS loops per live session). Both frameworks have zero OSS budget primitives; both need the SAME new substrate capability: **session-scoped reservation with streaming commit** (realtime WS sessions don't fit per-request pre-dispatch reservation). One design, two adapters.

**LiveKit Agents (RANK 2):** 10.9k stars; PyPI `livekit-agents` 4,208,179/30d (top-tier); 1.x stable (1.5.17, 2026-06-03), Python + Node; OpenAI/Character.ai/Retell/Speak built on it. LLM step is a swappable plugin (`AgentSession(llm=...)`) — clean framework-blessed seam. Vendor answer is **LiveKit Inference** (hosted gateway, cloud-only billing limits) — self-hosters get nothing: exactly SpendGuard's wedge. Apache 2.0, no visible DCO/CLA.

**Pipecat (RANK 3):** 12.7k stars; PyPI `pipecat-ai` 949,561/30d; v1.3.0 stable; NVIDIA ships `nvidia-pipecat` and builds official voice examples on it. Pipeline-first `FrameProcessor`/per-provider `LLMService` — arguably the cleanest interception seam of all candidates. No cost features, no hosted billing of its own — gap fully open. BSD-2-Clause.

**Cost:** the session-reservation design is new substrate surface (reservation lifecycle spec work before any adapter slice) — bigger lift than D38/D40. If only one adapter slot exists, LiveKit's 4.4× download volume wins.

## §4. AG-UI vocabulary upstream registration (outreach play, not engineering)

D39 shipped the `spendguard.*` event family as a SpendGuard-defined vocabulary riding AG-UI `CUSTOM` events. Upstream registration with `ag-ui-protocol/ag-ui` remains **deliberately not done** (D39 design records it as a follow-on outreach item).

- **Window:** zero competing cost/budget prior art in the AG-UI spec/repo as of 2026-06-10 (issue/PR sweep: 17 keyword hits, all false positives). First-mover on the vocabulary is still free.
- **Gate:** MIT, no CLA/DCO; contribution rule is "reach out before significant work" + code-owner review; protocol extensions accepted by maintainer fiat (CopilotKit single-vendor governance — no neutral RFC process).
- **Shape:** docs-first PR or discussion proposing the `spendguard.*` (or genericized `spend.*`) custom-event family, pointing at the shipped TS+Python builders + ASP Draft-01 field mapping. Analogous to the aeoess crosswalk play.
- **Risk:** 0.x churn (15+ releases/month); single-vendor governance means acceptance is a relationship outcome, not a process outcome.

## §5. Decision status

| Item | Status |
|---|---|
| D40 OpenClaw | ⏳ awaiting user decision (recommended next: base-URL recipe first) |
| D41 LiveKit+Pipecat voice | ⏳ awaiting user decision (requires session-reservation substrate spec first) |
| AG-UI upstream registration | ⏳ awaiting user decision (outreach, ~days of effort, window open) |
| NVIDIA OpenShell | 🔄 separate workstream — RFC #1734 + PR #1738 comments posted 2026-06-10 (first voice on the cost slot); awaiting maintainer response |

Full per-candidate source citations live in the 2026-06-10 research transcripts (session-internal, same convention as the parent memo's "Background research" section); the numbers above are the load-bearing subset.
