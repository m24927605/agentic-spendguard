# D40a - Review Standards

Use with a Codex sub-agent for every D40a slice. P0 and P1 findings block.

2026-06-12 user-directed reviewer override: the formal R1-R5 adversarial
review gate for D40/D41 coverage work is an independent Codex sub-agent
reviewer. Do not route reviews through an external Codex service, local
Ollama model, or Claude Code `superpowers:code-reviewer`.

## 1. Precedence (P0)

`design.md` is LOCKED and trumps slice docs. If the slice doc differs from the design, flag the slice doc; do not accept implementation drift.

## 2. Claim discipline (P0)

| Check | Pass condition |
|---|---|
| 2.1 | D40a is described as "base-URL recipe" or "egress proxy" coverage, never plugin coverage. |
| 2.2 | Docs state enforcement happens in SpendGuard egress proxy/sidecar, not in OpenClaw. |
| 2.3 | Exact OpenClaw config keys are backed by `OA-V1`; no invented config names. |
| 2.4 | D40b is cross-linked as future/adjacent plugin work, not required for D40a. |

## 3. Demo correctness (P0)

| Check | Pass condition |
|---|---|
| 3.1 | `make demo-down` was run before demo reruns. |
| 3.2 | DENY step proves counting-stub call count unchanged. |
| 3.3 | `verify_step_openclaw_base_url.sql` uses hard SQL failures, not warnings. |
| 3.4 | Demo uses local counting stub for hard gate; no live provider key required. |
| 3.5 | `unitId`, `windowInstanceId`, and pricing tuple are present in runtime wiring. |
| 3.6 | If `SPENDGUARD_PROXY_OPENAI_BASE_URL` is used, unset/default behavior still resolves to the existing OpenAI routing table URLs and is unit-tested. |
| 3.7 | If `DEMO_HARD_CAP_CLAIM_AMOUNT_ATOMIC_GT` is used, its default remains `1000000000` and the lowered value is scoped to the D40a overlay only. |

## 4. Repository hygiene (P1)

| Check | Pass condition |
|---|---|
| 4.1 | No changes under `sdk/fixtures/cross-language/`. |
| 4.2 | No SDK or proto files changed. |
| 4.3 | No unrelated demo modes changed except shared Makefile routing. |
| 4.4 | MDX code blocks do not break docs-site build. |
| 4.5 | The demo runner is described as an OpenClaw config-fixture runner, not proof that the full OpenClaw gateway binary was embedded in the stack. |

## 5. Reviewer prompt template

```text
You are the adversarial code reviewer for slice <SLICE_ID> (round R<N>) of
D40a - OpenClaw base-URL recipe.

Reviewer tool: Codex sub-agent, per the 2026-06-12 user-directed override above.

Read in order:
1. docs/specs/coverage/D40a_openclaw_base_url_recipe/design.md
2. docs/specs/coverage/D40a_openclaw_base_url_recipe/review-standards.md
3. docs/slices/<SLICE_ID>.md
4. The diff under review: <DIFF_REF>

Apply review-standards §1-§4. Treat claim drift and fake demo evidence as
Blocker findings. Verify every applicable VERIFY-AT-IMPL marker is pinned in
the slice doc with version/source evidence. Re-run runnable gates where
feasible and demand exact command output for demo gates.

Output numbered findings with severity, file:line, evidence, and spec ref.
End with exactly one verdict line:
VERDICT: PASS
or
VERDICT: FAIL (<b> blockers, <m> majors)
```
