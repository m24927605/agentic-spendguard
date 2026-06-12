# D41 voice adapters - Review Standards

Use with a Codex sub-agent for every D41 adapter slice.

2026-06-12 user-directed reviewer override: the formal R1-R5 adversarial
review gate for D40/D41 coverage work is an independent Codex sub-agent
reviewer. Do not route reviews through an external Codex service, local
Ollama model, or Claude Code `superpowers:code-reviewer`.

## 1. Precedence (P0)

`D41_session_reservation_substrate/design.md` owns lifecycle semantics. `D41_voice_livekit_pipecat/design.md` owns adapter surfaces. Slice docs cannot weaken either.

## 2. Substrate dependency (P0)

| Check | Pass condition |
|---|---|
| 2.1 | Adapter uses `reserve_session`, `commit_session_delta`, and `release_session`; no local per-request approximation. |
| 2.2 | Adapter fails closed if session reserve fails. |
| 2.3 | Paid provider connection does not start before reserve succeeds. |
| 2.4 | Commit deltas are positive and idempotent. |

## 3. Unit/pricing tuple (P0)

| Check | Pass condition |
|---|---|
| 3.1 | `tenant_id`, `budget_id`, `window_instance_id`, `unit_id`, and `pricing` are required. |
| 3.2 | Tuple threads unchanged through session reserve and commits. |
| 3.3 | No empty unit_id or pricing tuple in demo path. |

## 4. Framework API pins (P0)

| Check | Pass condition |
|---|---|
| 4.1 | `V41-V1`/`V41-V2` pins include exact package version and interface evidence. |
| 4.2 | `V41-V3`/`V41-V4` usage signal pins are tested. |
| 4.3 | If exact upstream surface differs, design.md has a dated amendment before code relies on it. |

## 5. Demo quality (P1)

| Check | Pass condition |
|---|---|
| 5.1 | Demo is deterministic and local; no microphone/browser/live provider required for hard gate. |
| 5.2 | DENY proves provider stub not called. |
| 5.3 | SQL gate proves session reserve, multiple commit deltas, release, and failure path. |

## 6. Reviewer prompt template

```text
You are the adversarial code reviewer for slice <SLICE_ID> (round R<N>) of
D41 - LiveKit Agents + Pipecat voice adapters.

Reviewer tool: Codex sub-agent, per the 2026-06-12 user-directed override above.

Read in order:
1. docs/specs/coverage/D41_session_reservation_substrate/design.md
2. docs/specs/coverage/D41_voice_livekit_pipecat/design.md
3. docs/specs/coverage/D41_voice_livekit_pipecat/review-standards.md
4. docs/slices/<SLICE_ID>.md
5. The diff under review: <DIFF_REF>

Block on per-request lifecycle workarounds, fail-open reserve behavior, missing
unit/window/pricing tuple, unpinned framework API assumptions, or fake demo
evidence. Verify the slice's V41 markers and applicable acceptance gates.

Output numbered findings with severity, file:line, evidence, and spec ref.
End with exactly one verdict line:
VERDICT: PASS
or
VERDICT: FAIL (<b> blockers, <m> majors)
```
