# Framework Coverage Build Plan

**Date:** 2026-06-06
**Scope source:** [`framework-coverage-2026-06.md`](framework-coverage-2026-06.md) Tier 1 + Tier 2 + Tier 3 (excluding the "Don't do" group)
**Workflow:** agency-agents Staff+ implementers + `superpowers:code-reviewer` per slice + R1-R5 review loop + 5-person Staff+ panel arbitration on R5 failure
**Cadence:** Don't-stop-in-design + don't-stop-per-slice per existing project feedback memories.

## §1. Workflow contract

### §1.1 Per-slice review loop

```
Slice spec written
   ↓
Staff+ implementer dispatched (Agent tool, parallel where independent)
   ↓
Implementer commits to slice branch (no GH PR; AI-only workflow per project memory)
   ↓
─── R1 ─────────────────────────────────────────
superpowers:code-reviewer skill invoked with slice spec + diff
   ↓
findings == 0 ? → SLICE PASSED → commit / push / next slice
   ↓ findings > 0
Same Staff+ implementer dispatched via SendMessage with findings
   ↓
─── R2 ─────────────────────────────────────────
superpowers:code-reviewer re-runs
   ↓
findings == 0 ? → SLICE PASSED
   ↓ findings > 0
... up to R5 ...
   ↓
R5 findings > 0 → Staff+ panel arbitration
   ↓
5 parallel panelists (Software Architect + Backend Architect + AI Engineer +
  Security Engineer + Senior Developer) each produce ≤ 1-page memo
   ↓
Summarizer (Software Architect by default) reconciles → final ruling
   ↓
Ruling: merge-with-residuals | block | rework
```

### §1.2 Reviewer

`superpowers:code-reviewer` skill is the canonical reviewer for every slice. Replaces the codex CLI adversarial review used in earlier hardening phases. The skill reviews against the slice's own `acceptance.md` + `review-standards.md` + repo coding standards.

### §1.3 R5 panel composition

| Role | `subagent_type` |
|------|----------------|
| Architecture | Software Architect |
| Backend systems | Backend Architect |
| AI / framework ecosystem | AI Engineer |
| Security / threat model | Security Engineer |
| Pragmatic impl judgment | Senior Developer |

Summarizer defaults to Software Architect. Materials follow [`docs/review-standards/staff-panel-arbitration-process.md`](../review-standards/staff-panel-arbitration-process.md) §2.

### §1.4 Spec set per deliverable

Each deliverable produces 5 documents (under `docs/specs/coverage/<deliverable>/`):

1. `design.md` — what we're building, why, key decisions, interfaces
2. `implementation.md` — code structure, module layout, key types, code skeleton
3. `tests.md` — unit + integration + demo regression coverage
4. `acceptance.md` — what makes this deliverable "shipped" — concrete gates
5. `review-standards.md` — slice-specific checklist for the R1-R5 reviewer

### §1.5 Slice doc per slice

Each slice produces a doc under `docs/slices/COV_<seq>_<deliverable>_<slice>.md` with:

- Scope (1-2 paragraphs)
- Files touched
- Test/verification plan
- Backlinks to spec set
- Anti-scope (explicit "not in this slice")

Slice naming: `COV_<seq>_<deliverable_short>_<slice_short>`, e.g. `COV_01_envoy_extproc_skeleton`.

## §2. Deliverable enumeration (30 items)

Excludes the 6 Python adapters already shipped (LangChain / LangGraph / Pydantic-AI / OpenAI Agents SDK / Microsoft AGT / LiteLLM proxy). Excludes the 5 "Don't do" items.

### §2.1 Tier 1 — Ship now (30 days)

| # | Deliverable | Category | Owner sub-agent (impl) |
|---|-------------|----------|------------------------|
| D01 | Envoy AI Gateway ExtProc sidecar | Gateway plugin | Backend Architect |
| D02 | Closed CLI Pattern 3 install script + CA bootstrap | CLI tooling | DevOps Automator |
| D03 | `OPENAI_BASE_URL` drop-in landing page (docs) | Documentation | Technical Writer |
| D04 | LangChain `BaseCallbackHandler` adapter — **TypeScript** (Python already shipped) | TS SDK | Frontend Developer |

### §2.2 Tier 2 — 90 days

| # | Deliverable | Category | Owner sub-agent (impl) |
|---|-------------|----------|------------------------|
| D05 | TS SDK substrate (`spendguard-sdk` npm package) — prerequisite for all TS adapters | TS SDK | Frontend Developer |
| D06 | Vercel AI SDK `wrapLanguageModel` middleware (covers Mastra) | TS adapter | Frontend Developer |
| D07 | Microsoft Agent Framework (MAF) middleware | .NET + Python adapter | Backend Architect |
| D08 | OpenAI Agents SDK — **TypeScript** Model wrap | TS adapter | Frontend Developer |
| D09 | Kong AI Gateway plugin (Lua / Go) | Gateway plugin | Backend Architect |
| D10 | Dify Model Provider Plugin (Python plugin daemon) | No-code platform | Backend Architect |
| D11 | LiteLLM `async_pre_call_hook` proxy guardrail plugin | Gateway plugin | Backend Architect |
| D12 | LiteLLM SDK monkey-patch shim (`spendguard-litellm-shim`) | Python SDK | Backend Architect |

### §2.3 Tier 3 — Backlog (still in scope per user directive)

| # | Deliverable | Category | Owner sub-agent (impl) |
|---|-------------|----------|------------------------|
| D13 | Subscription-tier meter mode (Claude Code Pro + Codex ChatGPT-OAuth) | Egress proxy | Backend Architect |
| D14 | Devin billing importer (`spendguard-importer-devin`) | Reconciliation collector | Backend Architect |
| D15 | Manus billing importer (`spendguard-importer-manus`) | Reconciliation collector | Backend Architect |
| D16 | Genspark billing importer (`spendguard-importer-genspark`) | Reconciliation collector | Backend Architect |
| D17 | Cursor MITM codec (SOW-only, marked `experimental`) | Proprietary protocol | Backend Architect |
| D18 | Windsurf MITM codec (SOW-only, marked `experimental`) | Proprietary protocol | Backend Architect |
| D19 | Google ADK `before_model_callback` adapter | Python adapter | AI Engineer |
| D20 | AWS Strands `HookProvider.before_invocation` adapter | Python adapter | AI Engineer |
| D21 | DSPy `BaseCallback` adapter | Python adapter | AI Engineer |
| D22 | Agno `pre_hooks` adapter | Python adapter | AI Engineer |
| D23 | BeeAI `Emitter` adapter | Python adapter | AI Engineer |
| D24 | AutoGen / AG2 `ChatCompletionClient` wrap | Python adapter | AI Engineer |
| D25 | SmolAgents `step_callbacks` + `Model.generate` wrap | Python adapter | AI Engineer |
| D26 | Letta `LLMClient` subclass | Python adapter | AI Engineer |
| D27 | LlamaIndex `CallbackManager` adapter | Python adapter | AI Engineer |
| D28 | Atomic Agents Instructor client wrap | Python adapter | AI Engineer |
| D29 | Inngest AgentKit `step.ai.wrap()` adapter | TS adapter | Frontend Developer |
| D30 | Anthropic claude-agent-sdk egress-proxy install recipe (D02 satisfies impl; this is the docs) | Documentation | Technical Writer |
| D31 | Coze Studio Model Provider plugin | No-code platform | Backend Architect |
| D32 | Botpress integration via Integration SDK | No-code platform | Backend Architect |
| D33 | AnythingLLM custom OpenAI-compatible base URL recipe (D03 covers; this is doc + smoke) | No-code platform | Technical Writer |
| D34 | LobeChat custom base URL recipe (D03 covers; doc + smoke) | No-code platform | Technical Writer |
| D35 | Flowise custom node | No-code platform | Frontend Developer |
| D36 | Langflow custom Python component | No-code platform | Backend Architect |
| D37 | n8n community node | No-code platform | Frontend Developer |

That's **37 deliverables**. Some are pure documentation (D03, D30, D33, D34) and slice into 1-2 work items; others (D01, D05, D07, D10) are multi-slice infrastructure.

### §2.4 Dependency order (must ship in this sequence where dependency edges exist)

```
D05 (TS SDK substrate) ──┬──→ D04 (LangChain TS)
                         ├──→ D06 (Vercel AI SDK)
                         ├──→ D08 (OpenAI Agents TS)
                         └──→ D29 (Inngest AgentKit)

D11 (LiteLLM proxy plugin) ───→ D12 (LiteLLM SDK shim)
                                  ↑
                                  └─ both unlock CrewAI / DSPy(D21) / Strands(D20)
                                     transitively (no per-framework slice needed
                                     for the LiteLLM-routed path)

D01 (Envoy ExtProc) ───→ no downstream blockers

D02 (CLI install script) ──→ D03 (base-URL landing page) ──→ D30, D33, D34 docs

D13 (subscription meter) requires D02 (CA install)
```

Execution order: **D05 → (D04, D06, D08, D29 parallel)**, **D01 / D02 / D03 / D07 / D09 / D10 / D11 / D12 parallel from start**, **Tier 3 follows in the listed order**.

## §3. Acceptance gates for "100% feasible"

Per user directive, every slice must be **100% feasible**. A slice is feasible iff:

1. Its `acceptance.md` lists concrete gates (file path tests, demo-mode runs, build commands).
2. Every gate is runnable in the current repo state at slice-spec time.
3. No gate depends on a third-party action SpendGuard cannot trigger (e.g. "Anthropic merges PR X" is not a gate; "we open PR X" is).
4. Reviewer (`superpowers:code-reviewer`) can re-run every gate without privileged access beyond what's in the repo.

For architecturally-unreachable deliverables (D14/D15/D16), feasibility = "billing-importer endpoint is testable against a vendor-staged fixture, even if the live admin API is gated." Acceptance includes "synthetic audit event emitted" as the primary gate, not "live import succeeded."

## §4. Slice-size guideline

| Slice size | Heuristic |
|------------|-----------|
| **S (small)** | 1 file, ≤ 200 LOC, ≤ 4 tests, single concept | 50% of slices |
| **M (medium)** | 2-4 files, ≤ 500 LOC, 5-15 tests | 40% of slices |
| **L (large)** | only when concept cannot be cleanly cut (e.g. proto changes, schema migrations) | 10% of slices |

Maximum slice size: 1000 LOC of impl + 500 LOC of test. Anything bigger gets re-sliced.

Per-deliverable slice estimate (varies by complexity):

| Tier | Slice count per deliverable |
|------|----------------------------|
| Pure docs (D03, D30, D33, D34) | 1-2 |
| Simple adapter (D19-D28) | 3-5 |
| TS adapter (D04, D06, D08, D29) | 4-6 |
| Gateway plugin (D01, D09, D11) | 5-8 |
| Multi-language (D07 MAF .NET+Python) | 6-10 |
| Infrastructure (D05 TS SDK substrate) | 6-10 |
| Reverse-engineered codec (D17, D18) | 8-12 (SOW only) |

Rough total: **~160 slices**. At R1-R5 review loops with average ~1.5 rounds per slice = ~240 review invocations.

## §5. Document layout convention

```
docs/specs/coverage/
├── D01_envoy_extproc/
│   ├── design.md
│   ├── implementation.md
│   ├── tests.md
│   ├── acceptance.md
│   └── review-standards.md
├── D02_closed_cli_install/
│   ├── design.md
│   ├── ...
├── ...
└── D37_n8n_community_node/

docs/slices/
├── COV_01_envoy_extproc_skeleton.md
├── COV_02_envoy_extproc_token_counter.md
├── ...
```

## §6. Anti-scope

Out of scope of this build plan:

- The 6 already-shipped Python adapters (LangChain / LangGraph / Pydantic-AI / OpenAI Agents Python / Microsoft AGT / LiteLLM proxy). They are baseline; do not re-touch.
- The 5 "Don't do" items: Make / Zapier Agents / Voiceflow non-Enterprise / Gemini OAuth proxy / Stack AI + Lyzr.
- Marketing / pricing / sales motion. This plan is engineering coverage.
- The `crosswalk/asp.yaml` PR to aeoess — separate workstream, tracked in [`project_asp_standards_push`](../../memory).

## §7. Definition of done (per deliverable)

A deliverable is done when:

- All slices in its slice plan are merged into main
- `acceptance.md` gates all run green
- A 1-paragraph entry has been added to `README.md` `## 🔌 Adapter integrations` table
- A `docs/site/docs/integrations/<deliverable>.md` page exists if the integration is user-facing
- A demo-mode entry exists in `Makefile` if the deliverable is demoable

## §8. Memory write-back convention

When a deliverable is fully done, write a memory entry under `~/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/` with name `project_coverage_<D##>_shipped.md` following the existing GA / HARDEN / POST_GA pattern (single paragraph: merge commit + round count + arbitration y/n + closed issues).

---

## §9. Execution notification cadence (per user directive)

Per `feedback_dont_stop_design`: design phase commits + pushes all spec sets without per-deliverable check-in.

Per `feedback_dont_stop_per_slice`: each slice impl → review → merge auto-chains; no per-slice check-in.

User is notified at two points only:
1. End of Phase A (all 37 spec sets committed)
2. End of Phase B (all slices merged, or a slice cannot pass R5 panel arbitration and explicitly blocks)
