# D30 — Anthropic claude-agent-sdk Egress-Proxy Install Recipe — `design.md`

> Status: Doc + smoke-test spec; lands before any slice.
> Sibling docs: `implementation.md`, `tests.md`, `acceptance.md`, `review-standards.md`.
> Build plan reference: `docs/strategy/framework-coverage-build-plan-2026-06.md` §2.3 (Tier 3 #D30).
> Strategy reference: `docs/strategy/framework-coverage-2026-06.md` Pattern 3 / Closed CLI.
> Owner sub-agent: Technical Writer.
> Audience: Project owner (sign-off), Technical Writer implementer, R1-R5 reviewer.

---

## 1. What we are shipping and why

Anthropic's `claude-agent-sdk` (Python + TypeScript) is the only first-party Anthropic agent SDK. It is special: the SDK **subprocesses the `claude` CLI binary**. The LLM call happens *inside* the CLI process — the SDK does not call `api.anthropic.com` directly. Therefore SpendGuard **cannot** gate it through an SDK-level adapter:

- The SDK's `PreToolUse` hook fires only for tool calls (Bash, Edit, etc.); it is **tool-scope, not LLM-scope**. It does not see the underlying `messages → response` exchange.
- The CLI honours `HTTPS_PROXY` and `NODE_EXTRA_CA_CERTS` (Node-based). With D02's CA install, all `api.anthropic.com/v1/messages` traffic flows through the SpendGuard egress proxy — the same gate that already routes Anthropic per `services/egress_proxy/src/routing.rs:208-212`.

D30 is therefore **documentation + a smoke-test demo**, not new SpendGuard code. D02 ships the install; D30 proves the install gates `claude-agent-sdk` end-to-end and gives developers a copy-paste recipe.

### 1.1 In-scope deliverables

1. One `docs/site/docs/integrations/claude-agent-sdk.md` page (Python + TS, D02 install handoff, PreToolUse warning, copy-paste sample).
2. Smoke scripts under `examples/claude-agent-sdk-egress/{python,typescript}/` that run one SDK task against the D02 install and assert reserve + commit audit rows.
3. `DEMO_MODE=agent_real_claude_agent_sdk_egress` wired into `deploy/demo/Makefile` + `deploy/demo/demo/run_demo.py`, driving the Python smoke through the existing `egress-proxy` compose stack (in-container CA bundle; no D02-on-host requirement for the demo).

### 1.2 Anti-scope

- **No SDK adapter package.** No LLM-scope hook exists; building one would lie about the gate.
- **No CA install logic.** D02 owns it; D30 assumes `spendguard install` succeeded.
- **No subscription-meter mode.** Claude Code Pro / Max is D13; D30 is BYOK only.
- **No `claude` CLI binary recipe.** Lives in D02's per-tool override matrix (row "Claude Code"); D30 is the SDK layer above it.
- **No TS SpendGuard SDK dependency.** Smoke uses raw `@anthropic-ai/claude-agent-sdk` plus a DB assert via existing tooling.

## 2. Architecture

```
Developer code (Python or TS)
   uses @anthropic-ai/claude-agent-sdk / claude_agent_sdk
        ↓ subprocess
   `claude` CLI binary (Node)
        ↓ HTTPS_PROXY=http://localhost:9000 + NODE_EXTRA_CA_CERTS=<spendguard CA>
   SpendGuard egress-proxy (api.anthropic.com/v1/messages route)
        ↓ PRE: sidecar RequestDecision  → CONTINUE / STOP
        ↓ forward to api.anthropic.com
        ↓ POST: sidecar ConfirmPublishOutcome (commit_estimated)
   audit_outbox: reserve row + commit row
```

The smoke asserts both rows appear with matching `request_id` after one `claude-agent-sdk` task.

## 3. Key design decisions

- **D-1. Pattern 3 only.** Document above the fold: `PreToolUse` is tool-scope, the egress proxy is the only LLM-scope gate. Prevents customers from mis-deploying a tool-scope hook as a budget control.
- **D-2. Sample task.** Canonical prompt `"List two ways to reverse a string in Python in under 30 words."` — short, costs <1¢, exactly one `messages` call, no tool invocation. Tool-using samples deferred so they do not muddy the D-1 message.
- **D-3. Python + TS parity.** Ship both `examples/claude-agent-sdk-egress/{python,typescript}/`. Doc covers both; each smoke targets its own SDK; both share the verification SQL.
- **D-4. Demo mode covers Python only.** `DEMO_MODE=agent_real_claude_agent_sdk_egress` runs the Python flavour inside the existing demo container. TS coverage is via `npm install && node smoke.mjs` from the CI workflow, not via demo-up — adding the Node toolchain to the demo image roughly doubles its size for no signal Python does not already give us.
- **D-5. Audit-chain assertion.** Each smoke ends by running a new `deploy/demo/verify_step_claude_agent_sdk_egress.sql` that asserts (a) one `RESERVE_RESPONSE` row with `provider='anthropic'` and `model LIKE 'claude-%'`, (b) one matching `COMMIT_OUTCOME` row with the same `request_id`, (c) `committed_input_tokens > 0` and `committed_output_tokens > 0`. Reuses existing `verify_step_*.sql` convention.
- **D-6. Doc page surface.** Page lives at `docs/site/docs/integrations/claude-agent-sdk.md` (legacy mkdocs surface, per user request). A site-v2 mirror is deferred to a follow-up slice and is not a D30 gate.

## 4. Slicing

Two slices. Total ~500 LOC docs + ~400 LOC smoke; well under the 1000-LOC cap.

| Slice | Title | Size |
|-------|-------|------|
| `COV_<N>_d30_doc_and_python_smoke` | Doc page + Python smoke + demo mode wiring + verify SQL | M |
| `COV_<N+1>_d30_typescript_smoke` | TypeScript smoke + CI workflow + doc cross-link | S |

## 5. Locked decisions recap

1. Pattern 3 only; PreToolUse warning above the fold (D-1).
2. Sample task is the canonical short-prompt no-tool prompt (D-2).
3. Python + TS smoke parity, both shipped, both verified by the same SQL (D-3, D-5).
4. Demo mode covers Python only; TS via CI (D-4).
5. Doc page lives at `docs/site/docs/integrations/claude-agent-sdk.md` (D-6).
